//! Upcoming reminders for the Telegram bot and webhooks.
//!
//! Server vaults: found here, in every note written (`⏰ 2026-10-30 15:00`,
//! in the owner's local time). A repeating one (`🔁 каждый день`) has a row for
//! its last time and one for its next, and each time that comes makes the next
//! row, so it goes on with no app open and its note unchanged. End-to-end
//! encrypted vaults: the apps send the reminders they found, since only they
//! can read the notes.

use std::collections::HashSet;

use bedrock_core::{parse, vault};
use bedrock_proto::{plain, ReminderItem};
use chrono::{DateTime, Duration, NaiveDateTime};
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::vaults::{self, Access};
use crate::{now, AppState};

/// A reminder this far in the past when first seen is not sent any more.
const STALE_SECS: i64 = 5 * 60;
/// Reminders missed by more than this (server was down) are dropped.
const MISSED_SECS: i64 = 6 * 60 * 60;

fn short_hash(text: &str) -> String {
    hex::encode(&Sha256::digest(text.as_bytes())[..6])
}

/// The stored key of one time of a reminder found in a note of a server vault.
pub fn key(file_id: &str, at: &str, text: &str) -> String {
    format!("{file_id}:{at}:{}", short_hash(text))
}

/// The local time in a key made by [`key`].
pub fn key_time(key: &str) -> Option<&str> {
    Some(key.split_once(':')?.1.rsplit_once(':')?.0)
}

/// The times of a reminder to keep rows for: a repeating one has its last time
/// (unless long missed), for sending it late, and its next.
fn times(r: &parse::Reminder, now: NaiveDateTime) -> Vec<NaiveDateTime> {
    if r.rule().is_none() {
        return NaiveDateTime::parse_from_str(&r.at, "%Y-%m-%d %H:%M").ok().into_iter().collect();
    }
    let missed = now - Duration::seconds(MISSED_SECS);
    r.last_due(now).filter(|t| *t >= missed).into_iter().chain(r.next_due(now)).collect()
}

/// Replaces the reminders of one note of a server vault (`content` is `None` when it was deleted).
pub async fn sync_file(st: &AppState, acc: &Access, file_id: &str, path: &str, content: Option<&[u8]>) -> ApiResult<()> {
    sync(st, &acc.id, acc.owner_id, file_id, path, content).await
}

async fn sync(st: &AppState, vault_id: &str, owner_id: i64, file_id: &str, path: &str, content: Option<&[u8]>) -> ApiResult<()> {
    let prefix = format!("{file_id}:");
    let mut wanted = Vec::new();
    if let Some(content) = content.filter(|_| vault::is_note(path)) {
        let text = String::from_utf8_lossy(content);
        let parsed = parse::parse(&text);
        let note = parsed.title.clone().unwrap_or_else(|| vault::stem(path).to_string());
        let offset = vaults::tz_offset(st, owner_id).await? as i64 * 60;
        let local_now = DateTime::from_timestamp(now() + offset, 0).unwrap_or_default().naive_utc();
        for r in parsed.reminders.iter().filter(|r| !r.done) {
            for local in times(r, local_now) {
                let at = local.and_utc().timestamp() - offset;
                wanted.push((key(file_id, &local.format("%Y-%m-%d %H:%M").to_string(), &r.text), at, r.text.clone(), note.clone()));
            }
        }
    }
    replace(st, vault_id, "server", Some(&prefix), wanted).await
}

/// Reads the rows of the note a key belongs to from the note again: after one
/// time of a repeating reminder, this makes the row of the next.
pub async fn refresh(st: &AppState, vault_id: &str, key: &str) -> ApiResult<()> {
    let Some((file_id, _)) = key.split_once(':') else { return Ok(()) };
    let row = sqlx::query("SELECT f.blob, v.owner_id FROM files f JOIN vaults v ON v.id = f.vault_id WHERE f.vault_id = ? AND f.file_id = ? AND f.deleted = 0")
        .bind(vault_id)
        .bind(file_id)
        .fetch_optional(&st.db)
        .await?;
    let Some(row) = row else { return Ok(()) };
    let Some((path, content)) = row.get::<Option<Vec<u8>>, _>(0).and_then(|b| plain::decode(&b)) else { return Ok(()) };
    sync(st, vault_id, row.get(1), file_id, &path, Some(&content)).await
}

/// On start: makes the rows of every note with a repeating reminder current,
/// as those written before this version have none for later times.
pub async fn sweep(st: &AppState) -> ApiResult<()> {
    let rows = sqlx::query(
        "SELECT f.vault_id, f.file_id, f.blob, v.owner_id FROM files f JOIN vaults v ON v.id = f.vault_id
         WHERE v.mode = 'server' AND f.deleted = 0 AND instr(f.blob, ?) > 0",
    )
    .bind("🔁".as_bytes())
    .fetch_all(&st.db)
    .await?;
    for row in rows {
        let (vault_id, file_id): (String, String) = (row.get(0), row.get(1));
        if let Some((path, content)) = row.get::<Option<Vec<u8>>, _>(2).and_then(|b| plain::decode(&b)) {
            sync(st, &vault_id, row.get(3), &file_id, &path, Some(&content)).await?;
        }
    }
    Ok(())
}

/// Replaces the reminders an app sent for an end-to-end encrypted vault.
pub async fn put_client(st: &AppState, acc: &Access, items: Vec<ReminderItem>) -> ApiResult<()> {
    if items.len() > 2000 {
        return Err(ApiError::BadRequest("слишком много напоминаний".into()));
    }
    let mut wanted = Vec::with_capacity(items.len());
    for it in items {
        if it.key.is_empty() || it.key.len() > 128 || it.text.chars().count() > 1000 || it.note.chars().count() > 300 {
            return Err(ApiError::BadRequest("некорректное напоминание".into()));
        }
        wanted.push((format!("c:{}", it.key), it.at, it.text, it.note));
    }
    replace(st, &acc.id, "client", None, wanted).await
}

/// Makes the stored set for (`source`, key `prefix`) equal to `wanted`,
/// keeping the sent flag of reminders that did not change.
async fn replace(
    st: &AppState,
    vault_id: &str,
    source: &str,
    prefix: Option<&str>,
    wanted: Vec<(String, i64, String, String)>,
) -> ApiResult<()> {
    let _guard = st.write_lock.lock().await;
    let mut tx = st.db.begin().await?;
    let rows = sqlx::query("SELECT key FROM reminders WHERE vault_id = ? AND source = ? AND substr(key, 1, ?) = ?")
        .bind(vault_id)
        .bind(source)
        .bind(prefix.map_or(0, |p| p.chars().count() as i64))
        .bind(prefix.unwrap_or(""))
        .fetch_all(&mut *tx)
        .await?;
    let keep: HashSet<&str> = wanted.iter().map(|w| w.0.as_str()).collect();
    for r in &rows {
        let key: String = r.get(0);
        if !keep.contains(key.as_str()) {
            sqlx::query("DELETE FROM reminders WHERE vault_id = ? AND key = ?").bind(vault_id).bind(&key).execute(&mut *tx).await?;
        }
    }
    let now = now();
    for (key, at, text, note) in &wanted {
        sqlx::query("INSERT OR IGNORE INTO reminders (vault_id, key, at, text, note, source, sent) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(vault_id)
            .bind(key)
            .bind(at)
            .bind(text)
            .bind(note)
            .bind(source)
            .bind(*at < now - STALE_SECS)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub struct Due {
    pub vault_id: String,
    pub vault_name: String,
    pub key: String,
    /// Unix time, seconds.
    pub at: i64,
    pub text: String,
    pub note: String,
    /// "server" (found in a note of a server vault) or "client".
    pub source: String,
}

/// Reminders whose time has come. Long-missed ones are marked sent and skipped.
pub async fn due(st: &AppState) -> ApiResult<Vec<Due>> {
    let now = now();
    let missed = sqlx::query("UPDATE reminders SET sent = 1 WHERE sent = 0 AND at < ? AND source = 'client'").bind(now - MISSED_SECS);
    missed.execute(&st.db).await?;
    let missed = sqlx::query("UPDATE reminders SET sent = 1 WHERE sent = 0 AND at < ? AND source = 'server' RETURNING vault_id, key")
        .bind(now - MISSED_SECS)
        .fetch_all(&st.db)
        .await?;
    for r in &missed {
        refresh(st, r.get(0), r.get(1)).await?;
    }
    let rows = sqlx::query(
        "SELECT r.vault_id, v.name, r.key, r.at, r.text, r.note, r.source FROM reminders r JOIN vaults v ON v.id = r.vault_id
         WHERE r.sent = 0 AND r.at <= ? ORDER BY r.at LIMIT 200",
    )
    .bind(now)
    .fetch_all(&st.db)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Due {
            vault_id: r.get(0),
            vault_name: r.get(1),
            key: r.get(2),
            at: r.get(3),
            text: r.get(4),
            note: r.get(5),
            source: r.get(6),
        })
        .collect())
}

pub async fn mark_sent(st: &AppState, vault_id: &str, key: &str) -> ApiResult<()> {
    sqlx::query("UPDATE reminders SET sent = 1 WHERE vault_id = ? AND key = ?").bind(vault_id).bind(key).execute(&st.db).await?;
    Ok(())
}
