//! Reminders sent to a URL of the user's choice (their own agent, a bot, an
//! automation) as signed JSON, alongside Telegram and independent of it.
//!
//! Deliveries go through a queue in the database: a restart loses nothing, and
//! a slow receiver holds up neither the reminder clock nor Telegram. Every
//! attempt is signed anew: `X-Webhook-Timestamp` is the current unix time and
//! `X-Webhook-Signature-V2` is hex HMAC-SHA256 of `"<timestamp>.<body>"` keyed
//! with the user's secret. `X-Request-ID`, the event id, stays the same across
//! attempts, so the receiver can drop repeats.
//!
//! Anyone may be able to register, so the server does not call local and
//! private addresses unless the admin allowed the host (`--webhook-allow-hosts`).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use bedrock_core::parse;
use bedrock_proto::{plain, ReminderWebhook, SetWebhookRequest, WebhookTestResult};
use chrono::DateTime;
use hmac::{Hmac, Mac};
use reqwest::header::CONTENT_TYPE;
use reqwest::Url;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::sync::Notify;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::reminders::{self, Due};
use crate::{now, vaults, AppState, Config};

/// Finished deliveries are kept this long, so the same event is never queued twice.
const KEEP_SECS: i64 = 7 * 24 * 60 * 60;
/// How much of the receiver's answer the test result shows.
const BODY_LIMIT: usize = 2000;
const PRIVATE: &str = "адрес ведёт в локальную или частную сеть, сервер такие не вызывает";

#[derive(Serialize)]
struct Event<'a> {
    event_type: &'static str,
    id: &'a str,
    test: bool,
    at: &'a str,
    at_utc: String,
    vault: EventVault<'a>,
    task: EventTask<'a>,
}

#[derive(Serialize)]
struct EventVault<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct EventTask<'a> {
    text: &'a str,
    path: &'a str,
    line: usize,
    due: Option<&'a str>,
    priority: Option<parse::Priority>,
}

/// The signature of one attempt: hex HMAC-SHA256 of `"<timestamp>.<body>"`.
pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes keys of any length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// The same reminder always gets the same id.
fn event_id(vault_id: &str, path: &str, text: &str, at: &str) -> String {
    hex::encode(Sha256::digest(format!("{vault_id}|{path}|{text}|{at}").as_bytes()))
}

fn utc(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn human(secs: u64) -> String {
    if secs >= 60 && secs % 60 == 0 {
        format!("{} мин", secs / 60)
    } else {
        format!("{secs} с")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Delivered,
    /// Repeating will not help: 400, 401, 403, 404, 413 and other client errors.
    Rejected,
    /// Worth another try: 429, 5xx, network errors and timeouts.
    Retry,
}

fn classify(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Delivered,
        429 | 500..=599 => Outcome::Retry,
        _ => Outcome::Rejected,
    }
}

struct Reply {
    outcome: Outcome,
    status: Option<u16>,
    body: String,
    error: Option<String>,
}

impl Reply {
    fn failed(outcome: Outcome, error: impl Into<String>) -> Self {
        Self { outcome, status: None, body: String::new(), error: Some(error.into()) }
    }

    fn summary(&self) -> String {
        match (self.status, &self.error) {
            (Some(status), _) => format!("ответ {status}"),
            (None, Some(error)) => error.clone(),
            (None, None) => String::new(),
        }
    }
}

/// The delivery machinery shared by the worker and the HTTP handlers.
pub struct Hooks {
    http: reqwest::Client,
    allow: Arc<Vec<String>>,
    retries: Vec<u64>,
    timeout: u64,
    wake: Notify,
}

impl Hooks {
    pub fn new(cfg: &Config) -> Arc<Self> {
        let allow: Arc<Vec<String>> = Arc::new(
            cfg.webhook_allow_hosts
                .iter()
                .map(|h| h.trim().trim_matches(['[', ']']).to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
        );
        let timeout = cfg.webhook_timeout_secs.max(1);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicOnly(allow.clone())))
            .user_agent(concat!("Bedrock/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Arc::new(Self { http, allow, retries: cfg.webhook_retry_secs.clone(), timeout, wake: Notify::new() })
    }

    fn allowed(&self, host: &str) -> bool {
        self.allow.iter().any(|h| h == host)
    }

    /// Checks a URL before it is saved.
    async fn check(&self, raw: &str) -> Result<Url, &'static str> {
        let url = Url::parse(raw).map_err(|_| "адрес вебхука: нужен полный адрес вида https://example.com/hook")?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("адрес вебхука должен начинаться с http:// или https://");
        }
        let host = host_of(&url).ok_or("в адресе вебхука нет сервера")?;
        if self.allowed(&host) {
            return Ok(url);
        }
        let ips: Vec<IpAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|_| "не нахожу такой сервер: проверьте адрес вебхука")?
                .map(|a| a.ip())
                .collect(),
        };
        if !ips.into_iter().any(is_public) {
            return Err(PRIVATE);
        }
        Ok(url)
    }

    /// One signed attempt.
    async fn post(&self, url: &str, secret: &str, event_id: &str, body: &[u8]) -> Reply {
        let Ok(url) = Url::parse(url) else { return Reply::failed(Outcome::Rejected, "неверный адрес вебхука") };
        // Names are checked by the resolver when connecting, literal addresses here.
        let literal = host_of(&url).filter(|h| !self.allowed(h)).and_then(|h| h.parse::<IpAddr>().ok());
        if literal.is_some_and(|ip| !is_public(ip)) {
            return Reply::failed(Outcome::Rejected, PRIVATE);
        }
        let timestamp = now();
        let sent = self
            .http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header("X-Request-ID", event_id)
            .header("X-Webhook-Timestamp", timestamp.to_string())
            .header("X-Webhook-Signature-V2", sign(secret, timestamp, body))
            .body(body.to_vec())
            .send()
            .await;
        match sent {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let mut buf = Vec::new();
                while buf.len() < BODY_LIMIT {
                    match resp.chunk().await {
                        Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
                        _ => break,
                    }
                }
                buf.truncate(BODY_LIMIT);
                Reply { outcome: classify(status), status: Some(status), body: String::from_utf8_lossy(&buf).into_owned(), error: None }
            }
            Err(e) if refused(&e) => Reply::failed(Outcome::Rejected, PRIVATE),
            Err(e) if e.is_timeout() => Reply::failed(Outcome::Retry, format!("получатель не ответил за {} с", self.timeout)),
            Err(e) if e.is_connect() => Reply::failed(Outcome::Retry, "не удалось соединиться с получателем"),
            Err(_) => Reply::failed(Outcome::Retry, "ошибка сети"),
        }
    }
}

fn host_of(url: &Url) -> Option<String> {
    url.host_str().map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase()).filter(|h| !h.is_empty())
}

fn refused(e: &reqwest::Error) -> bool {
    let mut source: Option<&dyn std::error::Error> = Some(e);
    while let Some(err) = source {
        if err.to_string() == PRIVATE {
            return true;
        }
        source = err.source();
    }
    false
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80)
            }
        },
    }
}

/// Resolves names to public addresses only (allowed hosts aside), so a saved
/// name cannot later be pointed at the server's own network.
struct PublicOnly(Arc<Vec<String>>);

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_ascii_lowercase();
        let allowed = self.0.iter().any(|h| *h == host);
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| allowed || is_public(a.ip())).collect();
            if addrs.is_empty() {
                return Err(PRIVATE.into());
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// ---------------------------------------------------------------- queue

/// Queues a due reminder of a server vault for everyone in the vault who set
/// up a webhook. The reminder clock calls it before sending anything to Telegram.
pub async fn queue_reminder(st: &AppState, r: &Due) -> ApiResult<()> {
    if r.source != "server" {
        return Ok(());
    }
    let users: Vec<i64> = sqlx::query(
        "SELECT user_id FROM reminder_webhooks WHERE user_id IN
         (SELECT owner_id FROM vaults WHERE id = ?1 UNION SELECT user_id FROM vault_members WHERE vault_id = ?1)",
    )
    .bind(&r.vault_id)
    .fetch_all(&st.db)
    .await?
    .iter()
    .map(|row| row.get(0))
    .collect();
    if users.is_empty() {
        return Ok(());
    }
    let Some((id, body)) = reminder_event(st, r).await? else {
        tracing::warn!("webhook: reminder {} is not in its note any more", r.key);
        return Ok(());
    };
    let now = now();
    for user_id in users {
        sqlx::query("INSERT OR IGNORE INTO webhook_queue (user_id, event_id, body, next_at, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(user_id)
            .bind(&id)
            .bind(&body)
            .bind(now)
            .bind(now)
            .execute(&st.db)
            .await?;
    }
    st.hooks.wake.notify_one();
    Ok(())
}

/// The event of a due reminder, from its note as it is now (for the line,
/// due date and priority). Serialized once: every attempt sends these bytes.
async fn reminder_event(st: &AppState, r: &Due) -> ApiResult<Option<(String, String)>> {
    let Some((file_id, _)) = r.key.split_once(':') else { return Ok(None) };
    let row = sqlx::query("SELECT blob FROM files WHERE vault_id = ? AND file_id = ? AND deleted = 0")
        .bind(&r.vault_id)
        .bind(file_id)
        .fetch_optional(&st.db)
        .await?;
    let Some((path, content)) = row.and_then(|row| row.get::<Option<Vec<u8>>, _>(0)).and_then(|b| plain::decode(&b)) else {
        return Ok(None);
    };
    let parsed = parse::parse(&String::from_utf8_lossy(&content));
    let Some(rem) = parsed.reminders.iter().find(|x| !x.done && reminders::key(file_id, x) == r.key) else { return Ok(None) };
    let task = parsed.tasks.iter().find(|t| t.line == rem.line);
    let id = event_id(&r.vault_id, &path, &rem.text, &rem.at);
    let event = Event {
        event_type: "reminder",
        id: &id,
        test: false,
        at: &rem.at,
        at_utc: utc(r.at),
        vault: EventVault { id: &r.vault_id, name: &r.vault_name },
        task: EventTask {
            text: &rem.text,
            path: &path,
            line: rem.line,
            due: task.and_then(|t| t.due.as_deref()),
            priority: task.map(|t| t.priority),
        },
    };
    let body = serde_json::to_string(&event)?;
    Ok(Some((id, body)))
}

/// Starts the delivery worker.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut purged = 0;
        loop {
            let wait = match start_due(&st).await {
                Ok(wait) => wait,
                Err(e) => {
                    tracing::warn!("webhooks: {e:?}");
                    Duration::from_secs(30)
                }
            };
            if now() - purged > 3600 {
                purged = now();
                let old = sqlx::query("DELETE FROM webhook_queue WHERE state != 'pending' AND created_at < ?")
                    .bind(purged - KEEP_SECS)
                    .execute(&st.db)
                    .await;
                if let Err(e) = old {
                    tracing::warn!("webhooks: {e}");
                }
            }
            tokio::select! {
                _ = st.hooks.wake.notified() => {}
                _ = tokio::time::sleep(wait) => {}
            }
        }
    });
}

/// Starts the deliveries whose time has come, each in its own task, and
/// returns how long to wait for the next one.
async fn start_due(st: &AppState) -> ApiResult<Duration> {
    let now = now();
    let rows = sqlx::query("SELECT id FROM webhook_queue WHERE state = 'pending' AND next_at <= ? ORDER BY next_at LIMIT 50")
        .bind(now)
        .fetch_all(&st.db)
        .await?;
    // Taken for the length of one attempt; one cut short by a restart is repeated after that.
    let lease = st.hooks.timeout as i64 + 50;
    for row in rows {
        let id: i64 = row.get(0);
        let taken = sqlx::query("UPDATE webhook_queue SET next_at = ? WHERE id = ? AND state = 'pending' AND next_at <= ?")
            .bind(now + lease)
            .bind(id)
            .bind(now)
            .execute(&st.db)
            .await?
            .rows_affected()
            == 1;
        if taken {
            let st = st.clone();
            tokio::spawn(async move {
                if let Err(e) = attempt(&st, id).await {
                    tracing::warn!("webhook delivery {id}: {e:?}");
                }
                st.hooks.wake.notify_one();
            });
        }
    }
    let next: Option<i64> = sqlx::query("SELECT MIN(next_at) FROM webhook_queue WHERE state = 'pending'").fetch_one(&st.db).await?.get(0);
    Ok(Duration::from_secs(next.map_or(600, |at| (at - crate::now()).clamp(1, 600)) as u64))
}

async fn attempt(st: &AppState, id: i64) -> ApiResult<()> {
    let row = sqlx::query(
        "SELECT q.user_id, q.event_id, q.body, q.attempts, w.url, w.secret
         FROM webhook_queue q LEFT JOIN reminder_webhooks w ON w.user_id = q.user_id WHERE q.id = ?",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    let (user_id, event_id, body, attempts): (i64, String, String, i64) = (row.get(0), row.get(1), row.get(2), row.get(3));
    let (Some(url), Some(secret)) = (row.get::<Option<String>, _>(4), row.get::<Option<String>, _>(5)) else {
        // The webhook was turned off meanwhile.
        sqlx::query("UPDATE webhook_queue SET state = 'failed' WHERE id = ?").bind(id).execute(&st.db).await?;
        return Ok(());
    };
    let n = attempts + 1;
    let reply = st.hooks.post(&url, &secret, &event_id, body.as_bytes()).await;
    tracing::info!("webhook {event_id}: attempt {n}, {}", reply.status.map_or_else(|| reply.summary(), |s| s.to_string()));
    let (state, next_at, status) = match (reply.outcome, st.hooks.retries.get(attempts as usize)) {
        (Outcome::Delivered, _) => ("delivered", 0, format!("доставлено ({})", reply.summary())),
        (Outcome::Retry, Some(&wait)) => ("pending", now() + wait as i64, format!("{}, повтор через {}", reply.summary(), human(wait))),
        (Outcome::Retry, None) => ("failed", 0, format!("не доставлено, попыток: {n} (последняя: {})", reply.summary())),
        (Outcome::Rejected, _) => ("failed", 0, format!("отклонено ({}), повторов не будет", reply.summary())),
    };
    sqlx::query("UPDATE webhook_queue SET attempts = ?, state = ?, next_at = ? WHERE id = ?")
        .bind(n)
        .bind(state)
        .bind(next_at)
        .bind(id)
        .execute(&st.db)
        .await?;
    record(st, user_id, &status).await
}

async fn record(st: &AppState, user_id: i64, status: &str) -> ApiResult<()> {
    sqlx::query("UPDATE reminder_webhooks SET last_status = ?, last_delivery_at = ? WHERE user_id = ?")
        .bind(status)
        .bind(now())
        .bind(user_id)
        .execute(&st.db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- HTTP

async fn status_of(st: &AppState, user_id: i64) -> ApiResult<ReminderWebhook> {
    let row = sqlx::query("SELECT url, last_status, last_delivery_at FROM reminder_webhooks WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(&st.db)
        .await?;
    Ok(ReminderWebhook {
        configured: row.is_some(),
        url: row.as_ref().map(|r| r.get(0)),
        last_status: row.as_ref().and_then(|r| r.get(1)),
        last_delivery_at: row.as_ref().and_then(|r| r.get(2)),
    })
}

pub async fn status(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<ReminderWebhook>> {
    Ok(Json(status_of(&st, user.id).await?))
}

pub async fn set(State(st): State<AppState>, user: AuthUser, Json(req): Json<SetWebhookRequest>) -> ApiResult<Json<ReminderWebhook>> {
    let raw = req.url.trim();
    if raw.len() > 2000 {
        return Err(ApiError::BadRequest("адрес вебхука слишком длинный".into()));
    }
    let url = st.hooks.check(raw).await.map_err(|e| ApiError::BadRequest(e.into()))?;
    let secret = req.secret.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if secret.is_some_and(|s| s.chars().count() < 32 || s.len() > 1024) {
        return Err(ApiError::BadRequest("секрет вебхука: от 32 символов".into()));
    }
    // A new address or secret starts with a clean delivery status.
    let res = match secret {
        Some(secret) => {
            sqlx::query(
                "INSERT INTO reminder_webhooks (user_id, url, secret, updated_at) VALUES (?, ?, ?, ?)
                 ON CONFLICT(user_id) DO UPDATE SET url = excluded.url, secret = excluded.secret,
                 updated_at = excluded.updated_at, last_status = NULL, last_delivery_at = NULL",
            )
            .bind(user.id)
            .bind(url.as_str())
            .bind(secret)
            .bind(now())
            .execute(&st.db)
            .await?
        }
        None => {
            sqlx::query("UPDATE reminder_webhooks SET url = ?, updated_at = ?, last_status = NULL, last_delivery_at = NULL WHERE user_id = ?")
                .bind(url.as_str())
                .bind(now())
                .bind(user.id)
                .execute(&st.db)
                .await?
        }
    };
    if res.rows_affected() == 0 {
        return Err(ApiError::BadRequest("нужен секрет вебхука: от 32 символов".into()));
    }
    Ok(Json(status_of(&st, user.id).await?))
}

pub async fn remove(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<ReminderWebhook>> {
    sqlx::query("DELETE FROM reminder_webhooks WHERE user_id = ?").bind(user.id).execute(&st.db).await?;
    sqlx::query("UPDATE webhook_queue SET state = 'failed' WHERE user_id = ? AND state = 'pending'").bind(user.id).execute(&st.db).await?;
    Ok(Json(status_of(&st, user.id).await?))
}

/// Sends a test event right away, without retries, and returns the answer.
pub async fn test(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<WebhookTestResult>> {
    let row = sqlx::query("SELECT url, secret FROM reminder_webhooks WHERE user_id = ?")
        .bind(user.id)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| ApiError::BadRequest("сначала сохраните адрес и секрет вебхука".into()))?;
    let (url, secret): (String, String) = (row.get(0), row.get(1));
    let minute = now() / 60 * 60;
    let offset = vaults::tz_offset(&st, user.id).await? as i64 * 60;
    let at = DateTime::from_timestamp(minute + offset, 0).unwrap_or_default().format("%Y-%m-%d %H:%M").to_string();
    let id = hex::encode(Sha256::digest(auth::random_token().as_bytes()));
    let event = Event {
        event_type: "reminder",
        id: &id,
        test: true,
        at: &at,
        at_utc: utc(minute),
        vault: EventVault { id: "test", name: "Тест" },
        task: EventTask { text: "Тестовое напоминание из Bedrock", path: "Тест.md", line: 0, due: None, priority: None },
    };
    let body = serde_json::to_vec(&event)?;
    let reply = st.hooks.post(&url, &secret, &id, &body).await;
    tracing::info!("webhook test {id}: {}", reply.status.map_or_else(|| reply.summary(), |s| s.to_string()));
    let delivered = reply.outcome == Outcome::Delivered;
    let status = if delivered { format!("доставлено ({})", reply.summary()) } else { reply.summary() };
    record(&st, user.id, &format!("проверка: {status}")).await?;
    Ok(Json(WebhookTestResult { delivered, status: reply.status, body: reply.body, error: reply.error }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_known_vector() {
        // Same as Python: hmac.new(secret, b"1761890400." + body, sha256).hexdigest()
        let body = br#"{"event_type":"reminder","id":"abc","test":true}"#;
        assert_eq!(sign("0123456789abcdef0123456789abcdef", 1761890400, body), "62ac0c9f82540a5464c4e4c2db6f9aad41b37969931c3a1cae6886e6de216bb3");
    }

    #[test]
    fn answers_are_classified() {
        for s in [200, 201, 202, 204] {
            assert_eq!(classify(s), Outcome::Delivered, "{s}");
        }
        for s in [400, 401, 403, 404, 413, 301, 422] {
            assert_eq!(classify(s), Outcome::Rejected, "{s}");
        }
        for s in [429, 500, 502, 503, 504] {
            assert_eq!(classify(s), Outcome::Retry, "{s}");
        }
    }

    #[test]
    fn only_public_addresses() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.0.10", "172.16.5.4", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["185.225.191.222", "8.8.8.8", "2a00:1450:4001::200e"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn retry_waits_in_words() {
        assert_eq!([30, 120, 600].map(human), ["30 с", "2 мин", "10 мин"]);
    }
}
