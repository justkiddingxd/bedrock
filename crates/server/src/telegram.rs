//! Telegram: linking accounts, sending reminders, and reporting edits other
//! people make in shared vaults.
//!
//! Every user can bring their own bot (made with @BotFather); the server runs
//! it for them. A server-wide bot (`--telegram-token`) is optional and serves
//! everyone who has none. Linking: the app asks for a one-time code, the user
//! opens `t.me/<bot>?start=<code>` and presses Start; the bot sees
//! `/start <code>` and remembers the chat, and which bot it came through.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::Json;
use bedrock_proto::{Role, SetBotRequest, TelegramLink, TelegramSettings, TelegramStatus, VaultMode};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::task::JoinHandle;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::vaults::{self, Access};
use crate::{now, reminders, webhook, AppState, Config};

const CODE_TTL_SECS: i64 = 30 * 60;

/// One bot: the server's (`owner` is `None`) or a user's.
pub struct Bot {
    token: String,
    owner: Option<i64>,
    /// Known once getMe succeeded.
    username: RwLock<Option<String>>,
}

impl Bot {
    pub fn username(&self) -> Option<String> {
        self.username.read().unwrap().clone()
    }
}

/// Edits in one vault waiting to be reported to one person.
struct Digest {
    vault: String,
    authors: BTreeSet<String>,
    notes: BTreeSet<String>,
    files: usize,
    last: Instant,
}

/// All bots the server runs, and what they still have to send.
pub struct Telegram {
    api: String,
    http: reqwest::Client,
    server: Option<Arc<Bot>>,
    users: RwLock<HashMap<i64, (Arc<Bot>, JoinHandle<()>)>>,
    /// Keyed by (vault id, recipient user id).
    digests: Mutex<HashMap<(String, i64), Digest>>,
    tick: Duration,
    change_delay: Duration,
}

impl Telegram {
    pub fn new(cfg: &Config) -> Arc<Self> {
        let server = cfg
            .telegram_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| Arc::new(Bot { token: t.to_string(), owner: None, username: RwLock::new(None) }));
        Arc::new(Self {
            api: cfg.telegram_api.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder().timeout(Duration::from_secs(40)).build().expect("HTTP client"),
            server,
            users: RwLock::new(HashMap::new()),
            digests: Mutex::new(HashMap::new()),
            tick: Duration::from_secs(cfg.reminder_tick_secs.max(1)),
            change_delay: Duration::from_secs(cfg.change_delay_secs),
        })
    }

    pub fn server_bot(&self) -> Option<String> {
        self.server.as_ref().and_then(|b| b.username())
    }

    fn user_bot(&self, user_id: i64) -> Option<Arc<Bot>> {
        self.users.read().unwrap().get(&user_id).map(|(b, _)| b.clone())
    }

    /// The bot behind a chat link: its owner's, or the server's.
    fn bot_for(&self, owner: Option<i64>) -> Option<Arc<Bot>> {
        match owner {
            Some(id) => self.user_bot(id),
            None => self.server.clone(),
        }
    }

    async fn call(&self, token: &str, method: &str, body: Value) -> anyhow::Result<Value> {
        let resp: Value = self.http.post(format!("{}/bot{token}/{method}", self.api)).json(&body).send().await?.json().await?;
        if resp["ok"].as_bool() != Some(true) {
            anyhow::bail!("telegram {method}: {}", resp["description"].as_str().unwrap_or("error"));
        }
        Ok(resp["result"].clone())
    }

    async fn send(&self, bot: &Bot, chat_id: i64, text: &str) {
        let body = json!({ "chat_id": chat_id, "text": text, "disable_web_page_preview": true });
        if let Err(e) = self.call(&bot.token, "sendMessage", body).await {
            tracing::warn!("telegram send to {chat_id}: {e:#}");
        }
    }
}

/// Starts the server bot, the bots users added, and the reminder clock.
pub fn spawn(st: AppState) {
    let tg = st.tg.clone();
    if let Some(bot) = tg.server.clone() {
        let s = st.clone();
        tokio::spawn(async move { poll(s, bot).await });
    }
    let s = st.clone();
    tokio::spawn(async move {
        match sqlx::query("SELECT user_id, token, username FROM user_bots").fetch_all(&s.db).await {
            Ok(rows) => {
                for r in rows {
                    start_user_bot(&s, r.get(0), r.get(1), r.get(2));
                }
            }
            Err(e) => tracing::warn!("loading user bots: {e}"),
        }
    });
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(tg.tick);
        loop {
            timer.tick().await;
            if let Err(e) = tick(&st).await {
                tracing::warn!("reminders: {e:?}");
            }
        }
    });
}

fn start_user_bot(st: &AppState, user_id: i64, token: String, username: String) {
    let bot = Arc::new(Bot { token, owner: Some(user_id), username: RwLock::new(Some(username)) });
    let (s, b) = (st.clone(), bot.clone());
    let task = tokio::spawn(async move { poll(s, b).await });
    if let Some((_, old)) = st.tg.users.write().unwrap().insert(user_id, (bot, task)) {
        old.abort();
    }
}

fn stop_user_bot(st: &AppState, user_id: i64) {
    if let Some((_, task)) = st.tg.users.write().unwrap().remove(&user_id) {
        task.abort();
    }
}

async fn poll(st: AppState, bot: Arc<Bot>) {
    let tg = st.tg.clone();
    let mut wait = 2;
    while bot.username().is_none() {
        match tg.call(&bot.token, "getMe", json!({})).await {
            Ok(me) => {
                let name = me["username"].as_str().unwrap_or_default().to_string();
                tracing::info!("telegram bot @{name}");
                *bot.username.write().unwrap() = Some(name);
            }
            Err(e) => {
                tracing::warn!("telegram getMe: {e:#}");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                wait = (wait * 2).min(300);
            }
        }
    }
    let mut offset = 0i64;
    loop {
        let body = json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message"] });
        match tg.call(&bot.token, "getUpdates", body).await {
            Ok(Value::Array(updates)) => {
                for u in updates {
                    offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                    if let Err(e) = on_message(&st, &bot, &u["message"]).await {
                        tracing::warn!("telegram update: {e:?}");
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("telegram getUpdates @{}: {e:#}", bot.username().unwrap_or_default());
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

const HELP: &str = "Я присылаю напоминания из Bedrock (строки с ⏰) и сообщаю, когда другие меняют общие хранилища.\n\n\
/changes — включить или выключить сообщения об изменениях\n/stop — отключить этот чат";

async fn on_message(st: &AppState, bot: &Bot, msg: &Value) -> ApiResult<()> {
    let (Some(chat_id), Some(text)) = (msg["chat"]["id"].as_i64(), msg["text"].as_str()) else { return Ok(()) };
    let mut words = text.split_whitespace();
    // In groups commands come as "/start@BotName".
    let command = words.next().unwrap_or_default().split('@').next().unwrap_or_default();
    let reply = match command {
        "/start" => match words.next() {
            Some(code) => link_chat(st, bot, code, chat_id).await?,
            None => "Чтобы получать напоминания, откройте Bedrock → Telegram и Claude и нажмите «Подключить Telegram».".into(),
        },
        "/stop" => {
            sqlx::query("DELETE FROM telegram_links WHERE chat_id = ? AND bot_owner IS ?").bind(chat_id).bind(bot.owner).execute(&st.db).await?;
            "Чат отключён: напоминания сюда больше не придут.".into()
        }
        "/changes" => {
            sqlx::query("UPDATE telegram_links SET notify_changes = 1 - notify_changes WHERE chat_id = ? AND bot_owner IS ?")
                .bind(chat_id)
                .bind(bot.owner)
                .execute(&st.db)
                .await?;
            let row = sqlx::query("SELECT notify_changes FROM telegram_links WHERE chat_id = ? AND bot_owner IS ?")
                .bind(chat_id)
                .bind(bot.owner)
                .fetch_optional(&st.db)
                .await?;
            match row {
                Some(r) if r.get::<bool, _>(0) => "Сообщения об изменениях в общих хранилищах включены.".into(),
                Some(_) => "Сообщения об изменениях выключены. Напоминания по-прежнему приходят.".into(),
                None => "Этот чат не подключён к Bedrock.".into(),
            }
        }
        _ => HELP.to_string(),
    };
    st.tg.send(bot, chat_id, &reply).await;
    Ok(())
}

async fn link_chat(st: &AppState, bot: &Bot, code: &str, chat_id: i64) -> ApiResult<String> {
    let now = now();
    let row = sqlx::query(
        "SELECT c.user_id, u.username, c.bot_owner FROM telegram_codes c JOIN users u ON u.id = c.user_id
         WHERE c.code = ? AND c.created_at > ?",
    )
    .bind(code)
    .bind(now - CODE_TTL_SECS)
    .fetch_optional(&st.db)
    .await?;
    let Some(row) = row else {
        return Ok("Ссылка устарела. Получите новую в Bedrock → Telegram и Claude.".into());
    };
    let (user_id, username, owner): (i64, String, Option<i64>) = (row.get(0), row.get(1), row.get(2));
    if owner != bot.owner {
        return Ok("Эта ссылка для другого бота. Откройте ссылку из Bedrock ещё раз.".into());
    }
    sqlx::query(
        "INSERT INTO telegram_links (user_id, chat_id, notify_changes, linked_at, bot_owner) VALUES (?, ?, 1, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET chat_id = excluded.chat_id, linked_at = excluded.linked_at, bot_owner = excluded.bot_owner",
    )
    .bind(user_id)
    .bind(chat_id)
    .bind(now)
    .bind(owner)
    .execute(&st.db)
    .await?;
    sqlx::query("DELETE FROM telegram_codes WHERE code = ? OR created_at <= ?").bind(code).bind(now - CODE_TTL_SECS).execute(&st.db).await?;
    Ok(format!("Готово: аккаунт «{username}» подключён. Сюда будут приходить напоминания и изменения в общих хранилищах.\n\n{HELP}"))
}

/// Where a user's messages go: their bot, chat, and whether they want edit reports.
async fn route(st: &AppState, user_id: i64) -> ApiResult<Option<(Arc<Bot>, i64, bool)>> {
    let row = sqlx::query("SELECT chat_id, notify_changes, bot_owner FROM telegram_links WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(&st.db)
        .await?;
    Ok(row.and_then(|r| st.tg.bot_for(r.get(2)).map(|bot| (bot, r.get(0), r.get(1)))))
}

/// Sends a message to a user if they linked Telegram.
pub async fn notify(st: &AppState, user_id: i64, text: &str) {
    if let Ok(Some((bot, chat, _))) = route(st, user_id).await {
        st.tg.send(&bot, chat, text).await;
    }
}

async fn tick(st: &AppState) -> ApiResult<()> {
    let due = reminders::due(st).await?;
    // Webhooks are only queued here, before any Telegram message: neither waits for the other.
    for r in &due {
        webhook::queue_reminder(st, r).await?;
    }
    for r in due {
        let body = if r.text.is_empty() { r.note.clone() } else { r.text.clone() };
        let text = format!("⏰ {body}\n{} · {}", r.note, r.vault_name);
        for (user_id, _, _) in vaults::participants(st, &r.vault_id).await? {
            notify(st, user_id, &text).await;
        }
        reminders::mark_sent(st, &r.vault_id, &r.key).await?;
    }

    let ready: Vec<(i64, Digest)> = {
        let mut digests = st.tg.digests.lock().unwrap();
        let keys: Vec<_> = digests.iter().filter(|(_, d)| d.last.elapsed() >= st.tg.change_delay).map(|(k, _)| k.clone()).collect();
        keys.into_iter().filter_map(|k| digests.remove(&k).map(|d| (k.1, d))).collect()
    };
    for (user_id, d) in ready {
        notify(st, user_id, &digest_text(&d)).await;
    }
    Ok(())
}

fn digest_text(d: &Digest) -> String {
    let who = d.authors.iter().cloned().collect::<Vec<_>>().join(", ");
    let what = if d.notes.is_empty() {
        let n = d.files;
        let word = match (n % 10, n % 100) {
            (1, r) if r != 11 => "файл",
            (2..=4, r) if !(12..=14).contains(&r) => "файла",
            _ => "файлов",
        };
        format!("{n} {word}")
    } else {
        let mut names: Vec<String> = d.notes.iter().take(5).cloned().collect();
        if d.notes.len() > 5 {
            names.push(format!("и ещё {}", d.notes.len() - 5));
        }
        names.join(", ")
    };
    format!("✏️ {who} — изменения в «{}»: {what}", d.vault)
}

/// Remembers edits for the other people in a shared vault; they hear about
/// them once the vault has been quiet for a while.
pub async fn queue_changes(st: &AppState, acc: &Access, author: &AuthUser, paths: &[Option<String>]) {
    let Ok(people) = vaults::participants(st, &acc.id).await else { return };
    if people.len() < 2 {
        return;
    }
    let notes: Vec<String> = if acc.mode == VaultMode::Server {
        paths
            .iter()
            .flatten()
            .filter(|p| bedrock_core::vault::is_note(p))
            .map(|p| bedrock_core::vault::stem(p).to_string())
            .collect()
    } else {
        Vec::new()
    };
    for (user_id, _, _) in people.into_iter().filter(|p| p.0 != author.id) {
        let Ok(Some((_, _, true))) = route(st, user_id).await else { continue };
        let mut digests = st.tg.digests.lock().unwrap();
        let d = digests.entry((acc.id.clone(), user_id)).or_insert_with(|| Digest {
            vault: acc.name.clone(),
            authors: BTreeSet::new(),
            notes: BTreeSet::new(),
            files: 0,
            last: Instant::now(),
        });
        d.authors.insert(author.username.clone());
        d.notes.extend(notes.iter().cloned());
        d.files += paths.len();
        d.last = Instant::now();
    }
}

pub fn role_label(role: Role) -> &'static str {
    match role {
        Role::Owner => "владелец",
        Role::Editor => "может редактировать",
        Role::Viewer => "только чтение",
    }
}

// ---------------------------------------------------------------- HTTP

async fn status_of(st: &AppState, user_id: i64) -> ApiResult<TelegramStatus> {
    let own_bot = st.tg.user_bot(user_id).and_then(|b| b.username());
    let server_bot = st.tg.server_bot();
    let link = route(st, user_id).await?;
    Ok(TelegramStatus {
        available: own_bot.is_some() || server_bot.is_some(),
        bot: own_bot.clone().or(server_bot.clone()),
        linked: link.is_some(),
        notify_changes: link.as_ref().is_none_or(|l| l.2),
        linked_via: link.and_then(|l| l.0.username()),
        own_bot,
        server_bot,
    })
}

pub async fn status(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<TelegramStatus>> {
    Ok(Json(status_of(&st, user.id).await?))
}

/// A one-time link to the user's own bot, or else the server's.
pub async fn link(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<TelegramLink>> {
    let (owner, bot) = match st.tg.user_bot(user.id).and_then(|b| b.username()) {
        Some(name) => (Some(user.id), name),
        None => (
            None,
            st.tg.server_bot().ok_or_else(|| ApiError::BadRequest("добавьте своего бота: на этом сервере общего бота нет".into()))?,
        ),
    };
    let code: String = auth::random_token().chars().take(24).collect();
    sqlx::query("INSERT INTO telegram_codes (code, user_id, created_at, bot_owner) VALUES (?, ?, ?, ?)")
        .bind(&code)
        .bind(user.id)
        .bind(now())
        .bind(owner)
        .execute(&st.db)
        .await?;
    Ok(Json(TelegramLink { url: format!("https://t.me/{bot}?start={code}"), code }))
}

pub async fn settings(State(st): State<AppState>, user: AuthUser, Json(req): Json<TelegramSettings>) -> ApiResult<Json<TelegramStatus>> {
    sqlx::query("UPDATE telegram_links SET notify_changes = ? WHERE user_id = ?").bind(req.notify_changes).bind(user.id).execute(&st.db).await?;
    Ok(Json(status_of(&st, user.id).await?))
}

pub async fn unlink(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<TelegramStatus>> {
    notify(&st, user.id, "Bedrock отключён от этого чата.").await;
    sqlx::query("DELETE FROM telegram_links WHERE user_id = ?").bind(user.id).execute(&st.db).await?;
    Ok(Json(status_of(&st, user.id).await?))
}

/// Adds (or replaces) the user's own bot after checking its token with Telegram.
pub async fn set_bot(State(st): State<AppState>, user: AuthUser, Json(req): Json<SetBotRequest>) -> ApiResult<Json<TelegramStatus>> {
    let token = req.token.trim();
    let well_formed = token.split_once(':').is_some_and(|(id, secret)| {
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && secret.len() >= 20
            && secret.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    if !well_formed {
        return Err(ApiError::BadRequest("это не похоже на токен бота: он выглядит как 123456789:AAE…".into()));
    }
    if st.tg.server.as_ref().is_some_and(|b| b.token == token) {
        return Err(ApiError::BadRequest("это бот сервера, он уже работает для всех".into()));
    }
    let taken = sqlx::query("SELECT 1 FROM user_bots WHERE token = ? AND user_id != ?").bind(token).bind(user.id).fetch_optional(&st.db).await?;
    if taken.is_some() {
        return Err(ApiError::Conflict("этот бот уже подключён к другому аккаунту".into()));
    }
    let me = st
        .tg
        .call(token, "getMe", json!({}))
        .await
        .map_err(|_| ApiError::BadRequest("Telegram не принял токен: проверьте, что скопировали его целиком".into()))?;
    let username = me["username"].as_str().unwrap_or_default().to_string();
    sqlx::query(
        "INSERT INTO user_bots (user_id, token, username, added_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET token = excluded.token, username = excluded.username, added_at = excluded.added_at",
    )
    .bind(user.id)
    .bind(token)
    .bind(&username)
    .bind(now())
    .execute(&st.db)
    .await?;
    // A chat linked through the previous bot of this user cannot be reached any more.
    sqlx::query("DELETE FROM telegram_links WHERE user_id = ? AND bot_owner = ?").bind(user.id).bind(user.id).execute(&st.db).await?;
    start_user_bot(&st, user.id, token.to_string(), username);
    Ok(Json(status_of(&st, user.id).await?))
}

pub async fn remove_bot(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<TelegramStatus>> {
    notify(&st, user.id, "Bedrock больше не использует этого бота.").await;
    stop_user_bot(&st, user.id);
    sqlx::query("DELETE FROM user_bots WHERE user_id = ?").bind(user.id).execute(&st.db).await?;
    sqlx::query("DELETE FROM telegram_links WHERE user_id = ? AND bot_owner = ?").bind(user.id).bind(user.id).execute(&st.db).await?;
    sqlx::query("DELETE FROM telegram_codes WHERE user_id = ? AND bot_owner = ?").bind(user.id).bind(user.id).execute(&st.db).await?;
    Ok(Json(status_of(&st, user.id).await?))
}
