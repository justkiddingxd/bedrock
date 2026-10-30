//! Bedrock sync server: accounts, shared vaults and their change feed
//! (end-to-end encrypted or readable by the server), plus the notes API,
//! remote MCP, and reminders in Telegram and webhooks. The same binary serves a
//! shared multi-user instance (`--registration open`) or a private
//! self-hosted one (the default lets only the first account register).

mod auth;
mod config;
mod db;
mod error;
mod notes;
mod reminders;
mod routes;
mod telegram;
mod vaults;
mod webhook;

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::{broadcast, Mutex};

pub use config::{Config, Registration};

#[derive(Clone, Debug)]
pub(crate) struct VaultEvent {
    pub vault_id: String,
    pub head: i64,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
    pub events: broadcast::Sender<VaultEvent>,
    /// SQLite allows one writer; serializing pushes here avoids busy errors.
    pub write_lock: Arc<Mutex<()>>,
    pub tg: Arc<telegram::Telegram>,
    pub hooks: Arc<webhook::Hooks>,
}

/// Opens the database and returns the HTTP router.
pub async fn build_app(cfg: Config) -> anyhow::Result<axum::Router> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    let db = db::connect(&cfg.data_dir.join("bedrock.db")).await?;
    db::migrate(&db).await?;
    let (events, _) = broadcast::channel(1024);
    let tg = telegram::Telegram::new(&cfg);
    let hooks = webhook::Hooks::new(&cfg);
    let state = AppState { db, cfg: Arc::new(cfg), events, write_lock: Arc::new(Mutex::new(())), tg, hooks };
    telegram::spawn(state.clone());
    webhook::spawn(state.clone());
    Ok(routes::router(state))
}

pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let bind = cfg.bind;
    tracing::info!(
        "bedrock-server {} on http://{bind}, data in {}, registration: {:?}, server bot: {}",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display(),
        cfg.registration,
        if cfg.telegram_token.is_some() { "on" } else { "off" }
    );
    let app = build_app(cfg).await?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
