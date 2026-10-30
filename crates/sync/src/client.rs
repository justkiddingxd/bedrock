use std::time::Duration;

use futures_util::StreamExt;
use bedrock_proto::*;
use reqwest::header::ACCEPT;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::{Result, SyncError};

/// Typed client for the sync server API.
#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base: &str) -> Result<Self> {
        let base = base.trim().trim_end_matches('/').to_string();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err(SyncError::Other("адрес сервера должен начинаться с http:// или https://".into()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("bedrock/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { http, base, token: None })
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let mut rb = self.http.request(method, format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            rb = rb.bearer_auth(t);
        }
        rb
    }

    async fn send<T: DeserializeOwned>(rb: RequestBuilder) -> Result<T> {
        let resp = check(rb.timeout(Duration::from_secs(120)).send().await?).await?;
        Ok(resp.json().await?)
    }

    pub async fn info(&self) -> Result<ServerInfo> {
        Self::send(self.request(Method::GET, "/api/info")).await
    }

    pub async fn register(&self, username: &str, password: &str) -> Result<AuthResponse> {
        let body = AuthRequest { username: username.into(), password: password.into() };
        Self::send(self.request(Method::POST, "/api/auth/register").json(&body)).await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthResponse> {
        let body = AuthRequest { username: username.into(), password: password.into() };
        Self::send(self.request(Method::POST, "/api/auth/login").json(&body)).await
    }

    pub async fn logout(&self) -> Result<()> {
        check(self.request(Method::POST, "/api/auth/logout").send().await?).await?;
        Ok(())
    }

    pub async fn list_vaults(&self) -> Result<Vec<VaultSummary>> {
        Self::send(self.request(Method::GET, "/api/vaults")).await
    }

    pub async fn create_vault(&self, name: &str, mode: VaultMode, key_envelope: Option<KeyEnvelope>) -> Result<VaultSummary> {
        let body = CreateVaultRequest { name: name.into(), mode, key_envelope };
        Self::send(self.request(Method::POST, "/api/vaults").json(&body)).await
    }

    pub async fn get_vault(&self, id: &str) -> Result<VaultDetails> {
        Self::send(self.request(Method::GET, &format!("/api/vaults/{id}"))).await
    }

    pub async fn members(&self, id: &str) -> Result<Vec<Member>> {
        Self::send(self.request(Method::GET, &format!("/api/vaults/{id}/members"))).await
    }

    pub async fn add_member(&self, id: &str, username: &str, role: Role) -> Result<Vec<Member>> {
        let body = AddMemberRequest { username: username.into(), role };
        Self::send(self.request(Method::POST, &format!("/api/vaults/{id}/members")).json(&body)).await
    }

    /// Removes someone from a vault; a member can remove themselves (leave).
    pub async fn remove_member(&self, id: &str, username: &str) -> Result<Vec<Member>> {
        Self::send(self.request(Method::DELETE, &format!("/api/vaults/{id}/members/{username}"))).await
    }

    pub async fn set_me(&self, settings: &MeSettings) -> Result<()> {
        check(self.request(Method::PUT, "/api/me").json(settings).send().await?).await?;
        Ok(())
    }

    pub async fn telegram(&self) -> Result<TelegramStatus> {
        Self::send(self.request(Method::GET, "/api/telegram")).await
    }

    pub async fn telegram_link(&self) -> Result<TelegramLink> {
        Self::send(self.request(Method::POST, "/api/telegram/link")).await
    }

    pub async fn telegram_settings(&self, settings: &TelegramSettings) -> Result<TelegramStatus> {
        Self::send(self.request(Method::PUT, "/api/telegram").json(settings)).await
    }

    pub async fn telegram_unlink(&self) -> Result<TelegramStatus> {
        Self::send(self.request(Method::DELETE, "/api/telegram")).await
    }

    /// Adds the user's own bot (token from @BotFather).
    pub async fn telegram_set_bot(&self, token: &str) -> Result<TelegramStatus> {
        Self::send(self.request(Method::PUT, "/api/telegram/bot").json(&SetBotRequest { token: token.into() })).await
    }

    pub async fn telegram_remove_bot(&self) -> Result<TelegramStatus> {
        Self::send(self.request(Method::DELETE, "/api/telegram/bot")).await
    }

    pub async fn reminder_webhook(&self) -> Result<ReminderWebhook> {
        Self::send(self.request(Method::GET, "/api/reminder-webhook")).await
    }

    /// `secret: None` keeps the saved secret.
    pub async fn set_reminder_webhook(&self, url: &str, secret: Option<&str>) -> Result<ReminderWebhook> {
        let body = SetWebhookRequest { url: url.into(), secret: secret.map(Into::into) };
        Self::send(self.request(Method::PUT, "/api/reminder-webhook").json(&body)).await
    }

    pub async fn delete_reminder_webhook(&self) -> Result<ReminderWebhook> {
        Self::send(self.request(Method::DELETE, "/api/reminder-webhook")).await
    }

    pub async fn test_reminder_webhook(&self) -> Result<WebhookTestResult> {
        Self::send(self.request(Method::POST, "/api/reminder-webhook/test")).await
    }

    pub async fn tokens(&self) -> Result<Vec<ApiToken>> {
        Self::send(self.request(Method::GET, "/api/tokens")).await
    }

    pub async fn create_token(&self, name: &str) -> Result<CreatedToken> {
        Self::send(self.request(Method::POST, "/api/tokens").json(&CreateTokenRequest { name: name.into() })).await
    }

    pub async fn delete_token(&self, id: i64) -> Result<()> {
        check(self.request(Method::DELETE, &format!("/api/tokens/{id}")).send().await?).await?;
        Ok(())
    }

    /// Replaces the upcoming reminders of an end-to-end encrypted vault.
    pub async fn put_reminders(&self, id: &str, items: Vec<ReminderItem>) -> Result<()> {
        let rb = self.request(Method::PUT, &format!("/api/vaults/{id}/reminders")).json(&RemindersRequest { items });
        check(rb.send().await?).await?;
        Ok(())
    }

    pub async fn changes(&self, id: &str, since: i64, limit: i64) -> Result<ChangesResponse> {
        let rb = self.request(Method::GET, &format!("/api/vaults/{id}/changes")).query(&[("since", since), ("limit", limit)]);
        Self::send(rb).await
    }

    pub async fn push(&self, id: &str, items: Vec<PushItem>) -> Result<PushResponse> {
        let rb = self.request(Method::POST, &format!("/api/vaults/{id}/push")).json(&PushRequest { items });
        Self::send(rb).await
    }

    /// Listens to the vault's event stream and calls `on_head` with every new
    /// head seq. Returns when the connection closes.
    pub async fn watch(&self, id: &str, mut on_head: impl FnMut(i64)) -> Result<()> {
        let resp = self
            .request(Method::GET, &format!("/api/vaults/{id}/events"))
            .header(ACCEPT, "text/event-stream")
            .send()
            .await?;
        let mut stream = check(resp).await?.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                if let Some(data) = line.trim_end().strip_prefix("data:") {
                    if let Ok(seq) = data.trim().parse() {
                        on_head(seq);
                    }
                }
            }
        }
        Ok(())
    }
}

async fn check(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<ErrorBody>(&text).map(|b| b.error).unwrap_or(text);
    if status == StatusCode::UNAUTHORIZED {
        return Err(SyncError::Unauthorized(message));
    }
    Err(SyncError::Server { status: status.as_u16(), message })
}
