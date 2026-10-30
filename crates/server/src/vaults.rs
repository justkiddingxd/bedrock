//! Who may do what in a vault, and the one write path every change goes
//! through (sync pushes, the notes API and MCP alike), so seq numbers, live
//! events, reminders and notifications stay consistent.

use bedrock_proto::{plain, Change, PushResult, Role, VaultMode};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::{now, reminders, telegram, AppState, VaultEvent};

/// The caller's view of a vault.
#[derive(Debug, Clone)]
pub struct Access {
    pub id: String,
    pub name: String,
    pub head: i64,
    pub mode: VaultMode,
    pub role: Role,
    pub owner_id: i64,
}

impl Access {
    pub fn require_write(&self) -> ApiResult<()> {
        if self.role.can_write() { Ok(()) } else { Err(ApiError::Forbidden("в этом хранилище у вас доступ только для чтения".into())) }
    }

    pub fn require_owner(&self) -> ApiResult<()> {
        if self.role == Role::Owner { Ok(()) } else { Err(ApiError::Forbidden("это может только владелец хранилища".into())) }
    }

    pub fn require_server_mode(&self) -> ApiResult<()> {
        match self.mode {
            VaultMode::Server => Ok(()),
            VaultMode::E2e => Err(ApiError::BadRequest(
                "хранилище зашифровано end-to-end: сервер не может читать его заметки".into(),
            )),
        }
    }
}

pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => "owner",
        Role::Editor => "editor",
        Role::Viewer => "viewer",
    }
}

pub fn parse_role(s: &str) -> Role {
    match s {
        "editor" => Role::Editor,
        "owner" => Role::Owner,
        _ => Role::Viewer,
    }
}

pub fn mode_name(mode: VaultMode) -> &'static str {
    match mode {
        VaultMode::E2e => "e2e",
        VaultMode::Server => "server",
    }
}

pub fn parse_mode(s: &str) -> VaultMode {
    if s == "server" { VaultMode::Server } else { VaultMode::E2e }
}

/// Fails with 404 unless the user owns the vault or is a member of it.
pub async fn access(st: &AppState, vault_id: &str, user_id: i64) -> ApiResult<Access> {
    let row = sqlx::query(
        "SELECT v.id, v.name, v.head_seq, v.mode, v.owner_id, m.role
         FROM vaults v LEFT JOIN vault_members m ON m.vault_id = v.id AND m.user_id = ?
         WHERE v.id = ?",
    )
    .bind(user_id)
    .bind(vault_id)
    .fetch_optional(&st.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let owner_id: i64 = row.get(4);
    let role = if owner_id == user_id {
        Role::Owner
    } else {
        match row.get::<Option<String>, _>(5) {
            Some(r) => parse_role(&r),
            None => return Err(ApiError::NotFound),
        }
    };
    Ok(Access { id: row.get(0), name: row.get(1), head: row.get(2), mode: parse_mode(&row.get::<String, _>(3)), role, owner_id })
}

/// Everyone with access to a vault: (user id, username, role), owner first.
pub async fn participants(st: &AppState, vault_id: &str) -> ApiResult<Vec<(i64, String, Role)>> {
    let rows = sqlx::query(
        "SELECT u.id, u.username, 'owner' FROM vaults v JOIN users u ON u.id = v.owner_id WHERE v.id = ?1
         UNION ALL
         SELECT u.id, u.username, m.role FROM vault_members m JOIN users u ON u.id = m.user_id WHERE m.vault_id = ?1",
    )
    .bind(vault_id)
    .fetch_all(&st.db)
    .await?;
    let mut out: Vec<(i64, String, Role)> =
        rows.iter().map(|r| (r.get(0), r.get(1), parse_role(&r.get::<String, _>(2)))).collect();
    if out.len() > 2 {
        out[1..].sort_by_key(|p| p.1.to_lowercase());
    }
    Ok(out)
}

/// Local time minus UTC of a user, in minutes.
pub async fn tz_offset(st: &AppState, user_id: i64) -> ApiResult<i32> {
    Ok(sqlx::query("SELECT tz_offset FROM users WHERE id = ?").bind(user_id).fetch_one(&st.db).await?.get(0))
}

pub fn change_from_row(r: &SqliteRow) -> Change {
    use base64::Engine;
    let blob: Option<Vec<u8>> = r.get(3);
    Change {
        file_id: r.get(0),
        seq: r.get(1),
        deleted: r.get(2),
        blob: blob.map(|b| base64::engine::general_purpose::STANDARD.encode(b)),
        updated_at: r.get(4),
    }
}

/// One file to write.
pub struct WriteItem {
    pub file_id: String,
    /// The version the change is based on; `None` overwrites whatever is there.
    pub base_seq: Option<i64>,
    /// New blob, or `None` to delete.
    pub blob: Option<Vec<u8>>,
    /// Path and content, known for server vaults.
    pub path: Option<String>,
    pub content: Option<Vec<u8>>,
}

impl WriteItem {
    /// A whole-file write to a server vault.
    pub fn plain(path: &str, content: Option<&[u8]>, base_seq: Option<i64>) -> Self {
        Self {
            file_id: plain::file_id(path),
            base_seq,
            blob: content.map(|c| plain::encode(path, c)),
            path: Some(path.to_string()),
            content: content.map(<[u8]>::to_vec),
        }
    }
}

/// Applies writes with optimistic concurrency and announces them.
pub async fn apply(st: &AppState, acc: &Access, author: &AuthUser, items: Vec<WriteItem>) -> ApiResult<(Vec<PushResult>, i64)> {
    let guard = st.write_lock.lock().await;
    let mut tx = st.db.begin().await?;
    let start_head: i64 = sqlx::query("SELECT head_seq FROM vaults WHERE id = ?").bind(&acc.id).fetch_one(&mut *tx).await?.get(0);
    let mut head = start_head;
    let now = now();
    let mut results = Vec::with_capacity(items.len());
    let mut written = Vec::new();

    for item in items {
        let current = sqlx::query("SELECT file_id, seq, deleted, blob, updated_at, path FROM files WHERE vault_id = ? AND file_id = ?")
            .bind(&acc.id)
            .bind(&item.file_id)
            .fetch_optional(&mut *tx)
            .await?;
        let (cur_seq, cur_deleted) = match &current {
            Some(r) => (r.get::<i64, _>(1), r.get::<bool, _>(2)),
            None => (0, true),
        };
        // Deleting something that is already gone always succeeds.
        if item.blob.is_none() && cur_deleted {
            results.push(PushResult::Ok { file_id: item.file_id, seq: cur_seq });
            continue;
        }
        // Someone else wrote this file after the client's base version.
        if let Some(row) = current.as_ref().filter(|_| item.base_seq.is_some_and(|b| b != cur_seq)) {
            results.push(PushResult::Conflict { file_id: item.file_id, current: change_from_row(row) });
            continue;
        }
        let path = item.path.clone().or_else(|| current.as_ref().and_then(|r| r.get::<Option<String>, _>(5)));
        head += 1;
        sqlx::query(
            "INSERT INTO files (vault_id, file_id, seq, deleted, blob, size, updated_at, path) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(vault_id, file_id) DO UPDATE SET
               seq = excluded.seq, deleted = excluded.deleted, blob = excluded.blob,
               size = excluded.size, updated_at = excluded.updated_at, path = excluded.path",
        )
        .bind(&acc.id)
        .bind(&item.file_id)
        .bind(head)
        .bind(item.blob.is_none())
        .bind(item.blob.as_deref())
        .bind(item.blob.as_deref().map(|b| b.len() as i64).unwrap_or(0))
        .bind(now)
        .bind(&path)
        .execute(&mut *tx)
        .await?;
        results.push(PushResult::Ok { file_id: item.file_id.clone(), seq: head });
        written.push((item, path));
    }

    if head != start_head {
        sqlx::query("UPDATE vaults SET head_seq = ? WHERE id = ?").bind(head).bind(&acc.id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    drop(guard);

    if head != start_head {
        let _ = st.events.send(VaultEvent { vault_id: acc.id.clone(), head });
        if acc.mode == VaultMode::Server {
            for (item, path) in &written {
                if let Some(path) = path {
                    reminders::sync_file(st, acc, &item.file_id, path, item.content.as_deref()).await?;
                }
            }
        }
        let paths: Vec<Option<String>> = written.into_iter().map(|(_, p)| p).collect();
        telegram::queue_changes(st, acc, author, &paths).await;
    }
    Ok((results, head))
}
