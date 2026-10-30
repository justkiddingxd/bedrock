//! Reminders of the open vault: a system notification when a `⏰` time
//! comes (also for ones missed while the app was closed, up to 12 hours),
//! and, for end-to-end encrypted vaults whose owner allowed it, handing the
//! upcoming ones to the server so its Telegram bot can send them too.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bedrock_core::index::ReminderItem;
use bedrock_sync::proto::{self, VaultMode};
use chrono::{Local, NaiveDateTime, TimeZone};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;

const TICK: Duration = Duration::from_secs(15);
const LATE_HOURS: i64 = 12;
const REUPLOAD: Duration = Duration::from_secs(10 * 60);
const FIRED_FILE: &str = "reminders-fired.json";

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut fired: Option<Fired> = None;
        let mut uploaded: Option<(String, u64, Instant)> = None;
        loop {
            if let Some((root, meta, items)) = snapshot(&app) {
                if fired.as_ref().is_none_or(|f| f.root != root) {
                    fired = Some(Fired::load(root, meta));
                }
                if let Some(f) = fired.as_mut() {
                    notify_due(&app, f, &items);
                }
                upload(&app, &items, &mut uploaded).await;
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn snapshot(app: &AppHandle) -> Option<(PathBuf, PathBuf, Vec<ReminderItem>)> {
    let state = app.state::<AppState>();
    let ws = state.ws.read().ok()?;
    let ws = ws.as_ref()?;
    Some((ws.vault.root().to_path_buf(), ws.vault.meta_dir(), ws.index.reminders()))
}

fn local_time(r: &ReminderItem) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&r.reminder.at, "%Y-%m-%d %H:%M").ok()
}

/// Reminders already announced, kept in `.bedrock/` so a restart does not repeat them.
struct Fired {
    root: PathBuf,
    file: PathBuf,
    keys: HashSet<String>,
}

impl Fired {
    fn load(root: PathBuf, meta: PathBuf) -> Self {
        let file = meta.join(FIRED_FILE);
        let keys = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { root, file, keys }
    }

    fn save(&mut self) {
        // Forget entries older than a few days; they can never fire again.
        let horizon = (Local::now() - chrono::Duration::days(3)).format("%Y-%m-%d").to_string();
        self.keys.retain(|k| k.split('|').nth(1).is_some_and(|at| at >= horizon.as_str()));
        if let Ok(data) = serde_json::to_vec(&self.keys) {
            let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(&self.root));
            let _ = std::fs::write(&self.file, data);
        }
    }
}

fn key_of(r: &ReminderItem) -> String {
    format!("{}|{}|{}", r.path, r.reminder.at, r.reminder.text)
}

fn notify_due(app: &AppHandle, fired: &mut Fired, items: &[ReminderItem]) {
    let now = Local::now().naive_local();
    let mut changed = false;
    for r in items {
        let Some(at) = local_time(r) else { continue };
        if at > now || now - at > chrono::Duration::hours(LATE_HOURS) || !fired.keys.insert(key_of(r)) {
            continue;
        }
        changed = true;
        let text = if r.reminder.text.is_empty() { r.note_title.clone() } else { r.reminder.text.clone() };
        let late = now - at > chrono::Duration::minutes(5);
        let body = if late { format!("{} · пропущено, было в {}", r.note_title, at.format("%H:%M")) } else { r.note_title.clone() };
        if let Err(e) = app.notification().builder().title(format!("⏰ {text}")).body(body).show() {
            tracing::warn!("notification: {e}");
        }
        let _ = app.emit("reminder", json!({ "path": r.path, "line": r.reminder.line, "text": text, "note": r.note_title }));
    }
    if changed {
        fired.save();
    }
}

/// FNV-1a: a stable id that every device computes the same way.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Keeps the server's copy of an e2e vault's upcoming reminders current.
async fn upload(app: &AppHandle, items: &[ReminderItem], uploaded: &mut Option<(String, u64, Instant)>) {
    let Some((client, cfg)) = app.state::<AppState>().sync.account() else { return };
    if cfg.mode != VaultMode::E2e {
        return;
    }
    let mut list = Vec::new();
    if cfg.share_reminders {
        let soon = Local::now().naive_local() - chrono::Duration::minutes(5);
        for r in items {
            let Some(at) = local_time(r).filter(|at| *at >= soon) else { continue };
            let Some(at) = Local.from_local_datetime(&at).earliest() else { continue };
            list.push(proto::ReminderItem {
                key: format!("{:016x}", fnv(&key_of(r))),
                at: at.timestamp(),
                text: if r.reminder.text.is_empty() { r.note_title.clone() } else { r.reminder.text.clone() },
                note: r.note_title.clone(),
            });
        }
    } else if uploaded.as_ref().is_none_or(|u| u.0 != cfg.vault_id) {
        // Never shared from here, nothing to withdraw.
        return;
    }
    let hash = fnv(&list.iter().map(|i| i.key.as_str()).collect::<Vec<_>>().join(","));
    let fresh = uploaded.as_ref().is_some_and(|u| u.0 == cfg.vault_id && u.1 == hash && u.2.elapsed() < REUPLOAD);
    if fresh {
        return;
    }
    let empty = list.is_empty();
    match client.put_reminders(&cfg.vault_id, list).await {
        // Turning sharing off withdraws everything once, then stops.
        Ok(()) if empty && !cfg.share_reminders => *uploaded = None,
        Ok(()) => *uploaded = Some((cfg.vault_id.clone(), hash, Instant::now())),
        Err(e) => tracing::warn!("reminders upload: {e}"),
    }
}
