use std::convert::Infallible;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use bedrock_core::vault;
use bedrock_proto::*;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::Deserialize;
use sqlx::Row;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::trace::TraceLayer;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::vaults::{self, mode_name, parse_mode, parse_role, role_name, WriteItem};
use crate::{notes, now, telegram, webhook, AppState, Registration};

const MAX_ITEMS_PER_PUSH: usize = 1000;

pub fn router(state: AppState) -> Router {
    // Base64 inflates blobs by 4/3; leave room for a batch of small files too.
    let body_limit = (state.cfg.max_file_mb as usize * 1024 * 1024) * 3 / 2 + 1024 * 1024;
    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/info", get(info))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", put(auth::set_me))
        .route("/api/tokens", get(auth::list_tokens).post(auth::create_token))
        .route("/api/tokens/{id}", axum::routing::delete(auth::delete_token))
        .route("/api/telegram", get(telegram::status).put(telegram::settings).delete(telegram::unlink))
        .route("/api/telegram/link", post(telegram::link))
        .route("/api/telegram/bot", put(telegram::set_bot).delete(telegram::remove_bot))
        .route("/api/reminder-webhook", get(webhook::status).put(webhook::set).delete(webhook::remove))
        .route("/api/reminder-webhook/test", post(webhook::test))
        .route("/api/vaults", get(list_vaults).post(create_vault))
        .route("/api/vaults/{id}", get(get_vault).delete(delete_vault))
        .route("/api/vaults/{id}/members", get(members).post(add_member))
        .route("/api/vaults/{id}/members/{username}", axum::routing::delete(remove_member))
        .route("/api/vaults/{id}/changes", get(changes))
        .route("/api/vaults/{id}/push", post(push))
        .route("/api/vaults/{id}/events", get(events))
        .route("/api/vaults/{id}/reminders", get(notes::get_reminders).put(notes::put_reminders))
        .route("/api/vaults/{id}/notes", get(notes::list))
        .route("/api/vaults/{id}/notes/{*path}", get(notes::get).put(notes::put).delete(notes::delete))
        .route("/api/vaults/{id}/append/{*path}", post(notes::append))
        .route("/api/vaults/{id}/tasks", get(notes::tasks).post(notes::add_task))
        .route("/api/vaults/{id}/tasks/complete", post(notes::complete_task))
        .route("/mcp", post(notes::mcp).get(notes::mcp_get))
        .route("/mcp/{token}", post(notes::mcp_token).get(notes::mcp_get))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn info(State(st): State<AppState>) -> ApiResult<Json<ServerInfo>> {
    let registration_open = match st.cfg.registration {
        Registration::Open => true,
        Registration::FirstOnly => auth::user_count(&st).await? == 0,
        Registration::Closed => false,
    };
    Ok(Json(ServerInfo {
        name: st.cfg.name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol: PROTOCOL_VERSION,
        registration_open,
        max_file_mb: st.cfg.max_file_mb,
        telegram_bot: st.tg.server_bot(),
    }))
}

async fn list_vaults(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<VaultSummary>>> {
    let rows = sqlx::query(
        "SELECT v.id, v.name, v.created_at, v.head_seq, v.mode, u.username, CASE WHEN v.owner_id = ?1 THEN 'owner' ELSE m.role END
         FROM vaults v JOIN users u ON u.id = v.owner_id
         LEFT JOIN vault_members m ON m.vault_id = v.id AND m.user_id = ?1
         WHERE v.owner_id = ?1 OR m.user_id IS NOT NULL
         ORDER BY v.name",
    )
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| VaultSummary {
                id: r.get(0),
                name: r.get(1),
                created_at: r.get(2),
                head_seq: r.get(3),
                mode: parse_mode(&r.get::<String, _>(4)),
                owner: r.get(5),
                role: parse_role(&r.get::<String, _>(6)),
            })
            .collect(),
    ))
}

async fn create_vault(
    State(st): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateVaultRequest>,
) -> ApiResult<Json<VaultSummary>> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest("название хранилища: от 1 до 100 символов".into()));
    }
    let envelope = match (req.mode, &req.key_envelope) {
        (VaultMode::E2e, Some(env)) => serde_json::to_string(env)?,
        (VaultMode::E2e, None) => return Err(ApiError::BadRequest("зашифрованному хранилищу нужен ключ".into())),
        (VaultMode::Server, None) => String::new(),
        (VaultMode::Server, Some(_)) => return Err(ApiError::BadRequest("серверному хранилищу ключ не нужен".into())),
    };
    let mut raw = [0u8; 16];
    OsRng.fill_bytes(&mut raw);
    let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let created_at = now();
    sqlx::query("INSERT INTO vaults (id, owner_id, name, key_envelope, head_seq, created_at, mode) VALUES (?, ?, ?, ?, 0, ?, ?)")
        .bind(&id)
        .bind(user.id)
        .bind(name)
        .bind(envelope)
        .bind(created_at)
        .bind(mode_name(req.mode))
        .execute(&st.db)
        .await?;
    Ok(Json(VaultSummary {
        id,
        name: name.to_string(),
        created_at,
        head_seq: 0,
        mode: req.mode,
        role: Role::Owner,
        owner: user.username,
    }))
}

async fn get_vault(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<VaultDetails>> {
    let acc = vaults::access(&st, &id, user.id).await?;
    let row = sqlx::query("SELECT v.created_at, v.key_envelope, u.username FROM vaults v JOIN users u ON u.id = v.owner_id WHERE v.id = ?")
        .bind(&id)
        .fetch_one(&st.db)
        .await?;
    let envelope: String = row.get(1);
    Ok(Json(VaultDetails {
        id: acc.id,
        name: acc.name,
        created_at: row.get(0),
        head_seq: acc.head,
        mode: acc.mode,
        role: acc.role,
        owner: row.get(2),
        key_envelope: if envelope.is_empty() { None } else { Some(serde_json::from_str(&envelope)?) },
    }))
}

async fn delete_vault(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<()> {
    vaults::access(&st, &id, user.id).await?.require_owner()?;
    let _guard = st.write_lock.lock().await;
    sqlx::query("DELETE FROM vaults WHERE id = ?").bind(&id).execute(&st.db).await?;
    Ok(())
}

// ---------------------------------------------------------------- members

async fn member_list(st: &AppState, id: &str) -> ApiResult<Json<Vec<Member>>> {
    Ok(Json(
        vaults::participants(st, id).await?.into_iter().map(|(_, username, role)| Member { username, role }).collect(),
    ))
}

async fn members(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Vec<Member>>> {
    vaults::access(&st, &id, user.id).await?;
    member_list(&st, &id).await
}

async fn add_member(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> ApiResult<Json<Vec<Member>>> {
    let acc = vaults::access(&st, &id, user.id).await?;
    acc.require_owner()?;
    if req.role == Role::Owner {
        return Err(ApiError::BadRequest("роль участника: editor или viewer".into()));
    }
    let row = sqlx::query("SELECT id FROM users WHERE username = ?")
        .bind(req.username.trim())
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| ApiError::BadRequest(format!("пользователь «{}» не найден на этом сервере", req.username.trim())))?;
    let member_id: i64 = row.get(0);
    if member_id == acc.owner_id {
        return Err(ApiError::BadRequest("владелец уже имеет доступ".into()));
    }
    let existed = sqlx::query("SELECT 1 FROM vault_members WHERE vault_id = ? AND user_id = ?")
        .bind(&id)
        .bind(member_id)
        .fetch_optional(&st.db)
        .await?
        .is_some();
    sqlx::query(
        "INSERT INTO vault_members (vault_id, user_id, role, added_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(vault_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(&id)
    .bind(member_id)
    .bind(role_name(req.role))
    .bind(now())
    .execute(&st.db)
    .await?;
    if !existed {
        let mut text = format!("👋 {} открыл(а) вам доступ к хранилищу «{}» ({}).", user.username, acc.name, telegram::role_label(req.role));
        if acc.mode == VaultMode::E2e {
            text.push_str(" Оно зашифровано: пароль хранилища спросите у владельца.");
        }
        telegram::notify(&st, member_id, &text).await;
    }
    member_list(&st, &id).await
}

async fn remove_member(
    State(st): State<AppState>,
    user: AuthUser,
    Path((id, username)): Path<(String, String)>,
) -> ApiResult<Json<Vec<Member>>> {
    let acc = vaults::access(&st, &id, user.id).await?;
    let leaving = username.eq_ignore_ascii_case(&user.username);
    if !leaving {
        acc.require_owner()?;
    } else if acc.role == Role::Owner {
        return Err(ApiError::BadRequest("владелец не может выйти из своего хранилища".into()));
    }
    let res = sqlx::query("DELETE FROM vault_members WHERE vault_id = ? AND user_id = (SELECT id FROM users WHERE username = ?)")
        .bind(&id)
        .bind(&username)
        .execute(&st.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    member_list(&st, &id).await
}

// ---------------------------------------------------------------- sync

#[derive(Deserialize)]
struct ChangesQuery {
    since: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    500
}

async fn changes(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<ChangesQuery>,
) -> ApiResult<Json<ChangesResponse>> {
    // Read the head first: rows read afterwards are at least as new, so a
    // client that jumps to `head_seq` never skips a change.
    let head_seq = vaults::access(&st, &id, user.id).await?.head;
    let limit = q.limit.clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT file_id, seq, deleted, blob, updated_at FROM files WHERE vault_id = ? AND seq > ? ORDER BY seq LIMIT ?",
    )
    .bind(&id)
    .bind(q.since)
    .bind(limit)
    .fetch_all(&st.db)
    .await?;
    let more = rows.len() as i64 == limit;
    Ok(Json(ChangesResponse { changes: rows.iter().map(vaults::change_from_row).collect(), head_seq, more }))
}

async fn push(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<PushRequest>,
) -> ApiResult<Json<PushResponse>> {
    let acc = vaults::access(&st, &id, user.id).await?;
    acc.require_write()?;
    if req.items.len() > MAX_ITEMS_PER_PUSH {
        return Err(ApiError::BadRequest(format!("не больше {MAX_ITEMS_PER_PUSH} файлов за раз")));
    }
    let max_size = st.cfg.max_file_mb as usize * 1024 * 1024;
    let mut items = Vec::with_capacity(req.items.len());
    for item in req.items {
        if item.file_id.len() != 64 || !item.file_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::BadRequest("некорректный file_id".into()));
        }
        let blob = match &item.blob {
            Some(b) => Some(B64.decode(b).map_err(|_| ApiError::BadRequest("blob не в base64".into()))?),
            None => None,
        };
        if blob.as_ref().is_some_and(|b| b.len() > max_size) {
            return Err(ApiError::BadRequest(format!("файл больше {} МБ", st.cfg.max_file_mb)));
        }
        // Server vaults: the server reads the blob, so check it is what it claims to be.
        let (path, content) = match (&acc.mode, &blob) {
            (VaultMode::Server, Some(b)) => {
                let (path, content) = plain::decode(b).ok_or_else(|| ApiError::BadRequest("неверный формат блоба".into()))?;
                let valid = vault::normalize(&path).is_ok_and(|p| p == path && !vault::is_hidden(&p));
                if !valid || plain::file_id(&path) != item.file_id {
                    return Err(ApiError::BadRequest(format!("недопустимый путь {path}")));
                }
                (Some(path), Some(content))
            }
            _ => (None, None),
        };
        items.push(WriteItem { file_id: item.file_id, base_seq: Some(item.base_seq), blob, path, content });
    }
    let (results, head_seq) = vaults::apply(&st, &acc, &user, items).await?;
    Ok(Json(PushResponse { results, head_seq }))
}

/// Server-sent events: `head` with the vault's newest seq, sent on connect and
/// after every push, so clients can pull right away instead of polling.
async fn events(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let head = vaults::access(&st, &id, user.id).await?.head;
    let initial = tokio_stream::once(Ok(Event::default().event("head").data(head.to_string())));
    let updates = BroadcastStream::new(st.events.subscribe()).filter_map(move |msg| match msg {
        Ok(ev) if ev.vault_id == id => Some(Ok(Event::default().event("head").data(ev.head.to_string()))),
        Ok(_) => None,
        // Missed some events: tell the client to check anyway.
        Err(_) => Some(Ok(Event::default().event("head").data("0"))),
    });
    Ok(Sse::new(initial.chain(updates)).keep_alive(KeepAlive::default()))
}
