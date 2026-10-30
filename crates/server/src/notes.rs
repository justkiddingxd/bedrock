//! Server vaults over HTTP: the notes API, and the store the MCP tools use.
//! Writes go through the same path as sync pushes, so the apps pull them
//! (and merge them with local edits) like any other change.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bedrock_core::ops::text;
use bedrock_core::vault;
use bedrock_mcp::{tools, Backend, FileMeta, Store, ToolError, VaultInfo};
use bedrock_proto::{plain, AppendRequest, NoteContent, NoteMeta, PushResult, RemindersRequest, VaultMode, WriteNoteRequest, WriteResult};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::vaults::{self, Access, WriteItem};
use crate::{reminders, AppState};

fn tool_err(e: ApiError) -> ToolError {
    ToolError(e.message())
}

/// One server vault, as seen by one user.
#[derive(Clone)]
pub struct ServerStore {
    st: AppState,
    acc: Access,
    user: AuthUser,
    today: NaiveDate,
}

impl ServerStore {
    pub async fn open(st: &AppState, acc: Access, user: AuthUser) -> ApiResult<Self> {
        acc.require_server_mode()?;
        let offset = vaults::tz_offset(st, acc.owner_id).await? as i64;
        let today = (chrono::Utc::now() + chrono::Duration::minutes(offset)).date_naive();
        Ok(Self { st: st.clone(), acc, user, today })
    }

    async fn read_row(&self, path: &str) -> ApiResult<Option<(String, i64, i64)>> {
        let row = sqlx::query("SELECT blob, seq, updated_at FROM files WHERE vault_id = ? AND file_id = ? AND deleted = 0")
            .bind(&self.acc.id)
            .bind(plain::file_id(path))
            .fetch_optional(&self.st.db)
            .await?;
        Ok(row.and_then(|r| {
            let blob: Vec<u8> = r.get(0);
            let (_, content) = plain::decode(&blob)?;
            Some((String::from_utf8_lossy(&content).into_owned(), r.get(1), r.get(2)))
        }))
    }

    async fn write_item(&self, item: WriteItem) -> ApiResult<PushResult> {
        let (mut results, _) = vaults::apply(&self.st, &self.acc, &self.user, vec![item]).await?;
        Ok(results.remove(0))
    }
}

impl Store for ServerStore {
    async fn list(&self) -> bedrock_mcp::Result<Vec<FileMeta>> {
        let rows = sqlx::query("SELECT path, size, updated_at FROM files WHERE vault_id = ? AND deleted = 0 AND path IS NOT NULL")
            .bind(&self.acc.id)
            .fetch_all(&self.st.db)
            .await
            .map_err(|e| ToolError(e.to_string()))?;
        Ok(rows.iter().map(|r| FileMeta { path: r.get(0), size: r.get::<i64, _>(1).max(0) as u64, updated: r.get(2) }).collect())
    }

    async fn read(&self, path: &str) -> bedrock_mcp::Result<Option<String>> {
        Ok(self.read_row(path).await.map_err(tool_err)?.map(|r| r.0))
    }

    async fn write(&self, path: &str, content: &str) -> bedrock_mcp::Result<()> {
        self.write_item(WriteItem::plain(path, Some(content.as_bytes()), None)).await.map_err(tool_err)?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> bedrock_mcp::Result<()> {
        self.write_item(WriteItem::plain(path, None, None)).await.map_err(tool_err)?;
        Ok(())
    }

    fn can_write(&self) -> bool {
        self.acc.role.can_write()
    }

    fn today(&self) -> NaiveDate {
        self.today
    }
}

/// All server vaults of an account (remote MCP).
pub struct ServerBackend {
    pub st: AppState,
    pub user: AuthUser,
}

impl ServerBackend {
    async fn server_vaults(&self) -> ApiResult<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT v.id, v.name, CASE WHEN v.owner_id = ?1 THEN 'owner' ELSE m.role END
             FROM vaults v LEFT JOIN vault_members m ON m.vault_id = v.id AND m.user_id = ?1
             WHERE v.mode = 'server' AND (v.owner_id = ?1 OR m.user_id IS NOT NULL) ORDER BY v.name",
        )
        .bind(self.user.id)
        .fetch_all(&self.st.db)
        .await?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
    }
}

impl Backend for ServerBackend {
    type Store = ServerStore;

    fn multi_vault(&self) -> bool {
        true
    }

    async fn vaults(&self) -> bedrock_mcp::Result<Vec<VaultInfo>> {
        Ok(self
            .server_vaults()
            .await
            .map_err(tool_err)?
            .into_iter()
            .map(|(id, name, role)| VaultInfo { id, name, role })
            .collect())
    }

    async fn open(&self, vault: Option<&str>) -> bedrock_mcp::Result<ServerStore> {
        let all = self.server_vaults().await.map_err(tool_err)?;
        let id = match vault {
            Some(v) => {
                let lower = v.to_lowercase();
                all.iter().find(|(id, name, _)| id == v || name.to_lowercase() == lower).map(|x| x.0.clone())
            }
            None if all.len() == 1 => Some(all[0].0.clone()),
            None if all.is_empty() => {
                return Err("нет серверных хранилищ: MCP работает только с хранилищами в режиме «Серверное»".into());
            }
            None => {
                let names: Vec<&str> = all.iter().map(|x| x.1.as_str()).collect();
                return Err(ToolError(format!("укажите vault: {}", names.join(", "))));
            }
        };
        let id = id.ok_or_else(|| ToolError(format!("хранилище «{}» не найдено среди серверных", vault.unwrap_or_default())))?;
        let acc = vaults::access(&self.st, &id, self.user.id).await.map_err(tool_err)?;
        ServerStore::open(&self.st, acc, self.user.clone()).await.map_err(tool_err)
    }
}

/// A single vault, to reuse the tools behind the REST endpoints.
struct OneVault(ServerStore);

impl Backend for OneVault {
    type Store = ServerStore;

    fn multi_vault(&self) -> bool {
        false
    }

    async fn vaults(&self) -> bedrock_mcp::Result<Vec<VaultInfo>> {
        Ok(Vec::new())
    }

    async fn open(&self, _vault: Option<&str>) -> bedrock_mcp::Result<ServerStore> {
        Ok(self.0.clone())
    }
}

// ---------------------------------------------------------------- REST

async fn store_for(st: &AppState, id: &str, user: AuthUser) -> ApiResult<ServerStore> {
    let acc = vaults::access(st, id, user.id).await?;
    ServerStore::open(st, acc, user).await
}

fn clean_path(raw: &str) -> ApiResult<String> {
    let path = vault::normalize(raw).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if vault::is_hidden(&path) {
        return Err(ApiError::BadRequest(format!("недопустимый путь {path}")));
    }
    Ok(path)
}

async fn run_tool(store: ServerStore, name: &str, args: Value) -> ApiResult<Json<Value>> {
    let out = tools::call(&OneVault(store), name, &args).await.map_err(|e| ApiError::BadRequest(e.0))?;
    Ok(Json(serde_json::from_str(&out).unwrap_or(Value::String(out))))
}

pub async fn list(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Vec<NoteMeta>>> {
    let store = store_for(&st, &id, user).await?;
    let rows = sqlx::query("SELECT path, size, seq, updated_at FROM files WHERE vault_id = ? AND deleted = 0 AND path IS NOT NULL ORDER BY path")
        .bind(&store.acc.id)
        .fetch_all(&st.db)
        .await?;
    Ok(Json(rows.iter().map(|r| NoteMeta { path: r.get(0), size: r.get(1), seq: r.get(2), updated_at: r.get(3) }).collect()))
}

pub async fn get(State(st): State<AppState>, user: AuthUser, Path((id, path)): Path<(String, String)>) -> ApiResult<Json<NoteContent>> {
    let store = store_for(&st, &id, user).await?;
    let path = clean_path(&path)?;
    let (content, seq, updated_at) = store.read_row(&path).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(NoteContent { path, content, seq, updated_at }))
}

fn write_result(path: String, result: PushResult) -> ApiResult<Json<WriteResult>> {
    match result {
        PushResult::Ok { seq, .. } => Ok(Json(WriteResult { path, seq })),
        PushResult::Conflict { current, .. } => {
            Err(ApiError::Conflict(format!("заметка изменилась (seq {}), прочитайте её заново", current.seq)))
        }
    }
}

pub async fn put(
    State(st): State<AppState>,
    user: AuthUser,
    Path((id, path)): Path<(String, String)>,
    Json(req): Json<WriteNoteRequest>,
) -> ApiResult<Json<WriteResult>> {
    let store = store_for(&st, &id, user).await?;
    store.acc.require_write()?;
    let path = clean_path(&path)?;
    let result = store.write_item(WriteItem::plain(&path, Some(req.content.as_bytes()), req.base_seq)).await?;
    write_result(path, result)
}

pub async fn delete(State(st): State<AppState>, user: AuthUser, Path((id, path)): Path<(String, String)>) -> ApiResult<()> {
    let store = store_for(&st, &id, user).await?;
    store.acc.require_write()?;
    let path = clean_path(&path)?;
    store.read_row(&path).await?.ok_or(ApiError::NotFound)?;
    store.write_item(WriteItem::plain(&path, None, None)).await?;
    Ok(())
}

pub async fn append(
    State(st): State<AppState>,
    user: AuthUser,
    Path((id, path)): Path<(String, String)>,
    Json(req): Json<AppendRequest>,
) -> ApiResult<Json<WriteResult>> {
    let store = store_for(&st, &id, user).await?;
    store.acc.require_write()?;
    let path = clean_path(&path)?;
    let (content, seq) = match store.read_row(&path).await? {
        Some((c, seq, _)) => (c, seq),
        None => (String::new(), 0),
    };
    let updated = text::append(&content, &req.text);
    let result = store.write_item(WriteItem::plain(&path, Some(updated.as_bytes()), Some(seq))).await?;
    write_result(path, result)
}

#[derive(Deserialize)]
pub struct TasksQuery {
    status: Option<String>,
    path: Option<String>,
    due_before: Option<String>,
}

pub async fn tasks(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>, Query(q): Query<TasksQuery>) -> ApiResult<Json<Value>> {
    let store = store_for(&st, &id, user).await?;
    run_tool(store, "list_tasks", json!({ "status": q.status, "path": q.path, "due_before": q.due_before })).await
}

pub async fn add_task(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>, Json(args): Json<Value>) -> ApiResult<Json<Value>> {
    run_tool(store_for(&st, &id, user).await?, "add_task", args).await
}

pub async fn complete_task(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>, Json(args): Json<Value>) -> ApiResult<Json<Value>> {
    run_tool(store_for(&st, &id, user).await?, "complete_task", args).await
}

pub async fn get_reminders(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    run_tool(store_for(&st, &id, user).await?, "list_reminders", json!({})).await
}

/// Apps hand over the reminders of end-to-end encrypted vaults.
pub async fn put_reminders(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<RemindersRequest>,
) -> ApiResult<()> {
    let acc = vaults::access(&st, &id, user.id).await?;
    if acc.mode == VaultMode::Server {
        return Err(ApiError::BadRequest("в серверных хранилищах сервер находит напоминания сам".into()));
    }
    reminders::put_client(&st, &acc, req.items).await
}

// ---------------------------------------------------------------- MCP over HTTP

/// `POST /mcp` with `Authorization: Bearer <token>`.
pub async fn mcp(State(st): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let token = auth::bearer(&headers).map(str::to_string);
    mcp_with(st, token, body).await
}

/// `POST /mcp/<token>`, for clients that can only be given a URL.
pub async fn mcp_token(State(st): State<AppState>, Path(token): Path<String>, body: axum::body::Bytes) -> Response {
    mcp_with(st, Some(token), body).await
}

const NO_TOKEN: &str = "нужен API-токен: создайте его в Bedrock → Telegram и Claude и передайте в заголовке \
Authorization: Bearer <токен> или в адресе /mcp/<токен>";
const BAD_TOKEN: &str = "API-токен не подошёл к этому серверу: скопируйте его заново из Bedrock → Telegram и Claude \
(токен показывается один раз; если он потерян, создайте новый)";

/// The token itself, also when someone pasted the whole connector URL (…/mcp/<token>).
fn clean_token(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches('"');
    let t = t.rsplit_once("/mcp/").map_or(t, |(_, token)| token).trim_end_matches('/').trim();
    (!t.is_empty()).then(|| t.to_string())
}

async fn mcp_with(st: AppState, token: Option<String>, body: axum::body::Bytes) -> Response {
    let Some(token) = token.as_deref().and_then(clean_token) else {
        return ApiError::Unauthorized(NO_TOKEN).into_response();
    };
    let user = match auth::user_for_token(&st, &token).await {
        Ok(u) => u,
        Err(ApiError::Unauthorized(_)) => return ApiError::Unauthorized(BAD_TOKEN).into_response(),
        Err(e) => return e.into_response(),
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(m) => m,
        Err(e) => return Json(bedrock_mcp::protocol::parse_error(&e.to_string())).into_response(),
    };
    match bedrock_mcp::protocol::handle(&ServerBackend { st, user }, msg).await {
        Some(reply) => Json(reply).into_response(),
        None => axum::http::StatusCode::ACCEPTED.into_response(),
    }
}

/// This server does not push messages over SSE.
pub async fn mcp_get() -> Response {
    axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response()
}
