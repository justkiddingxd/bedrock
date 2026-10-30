//! Background sync for the open vault: a loop that syncs on start, shortly
//! after local edits, when the server announces new changes, and once a minute.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bedrock_core::Vault;
use bedrock_sync::proto::{MeSettings, ServerInfo, VaultMode, VaultSummary};
use bedrock_sync::{ApiClient, KdfParams, SyncConfig, SyncEngine, SyncError, SyncReport};
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{Mutex, Notify};

use crate::commands::{current_vault, CmdResult, StrErr};
use crate::state::AppState;

const DEBOUNCE: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub configured: bool,
    pub server: Option<String>,
    pub username: Option<String>,
    pub vault_id: Option<String>,
    pub vault_name: Option<String>,
    /// "e2e" or "server".
    pub mode: Option<String>,
    pub read_only: bool,
    pub share_reminders: bool,
    /// "off" | "idle" | "syncing" | "error"
    pub state: String,
    pub last_sync: Option<i64>,
    pub last_error: Option<String>,
    pub needs_login: bool,
    pub last_report: Option<SyncReport>,
}

fn off_status() -> SyncStatus {
    SyncStatus { state: "off".into(), ..Default::default() }
}

struct PendingLogin {
    server: String,
    username: String,
    token: String,
}

#[derive(Default)]
pub struct SyncHandle {
    engine: Mutex<Option<SyncEngine>>,
    kick: Notify,
    status: StdMutex<SyncStatus>,
    tasks: StdMutex<Vec<JoinHandle<()>>>,
    pending: StdMutex<Option<PendingLogin>>,
    /// Client and settings of the connected vault, for account calls that
    /// should not wait for a running sync.
    account: StdMutex<Option<(ApiClient, SyncConfig)>>,
}

impl SyncHandle {
    /// Asks the sync loop to run soon.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    fn status(&self) -> SyncStatus {
        let s = self.status.lock().unwrap().clone();
        if s.state.is_empty() { off_status() } else { s }
    }

    fn set_status(&self, app: &AppHandle, f: impl FnOnce(&mut SyncStatus)) {
        let snapshot = {
            let mut guard = self.status.lock().unwrap();
            f(&mut guard);
            guard.clone()
        };
        let _ = app.emit("sync:status", &snapshot);
    }

    fn stop_tasks(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    pub fn account(&self) -> Option<(ApiClient, SyncConfig)> {
        self.account.lock().unwrap().clone()
    }

    /// Changes the saved connection settings of the open vault.
    pub async fn update_config(&self, app: &AppHandle, f: impl FnOnce(&mut SyncConfig)) -> CmdResult<()> {
        let mut guard = self.engine.lock().await;
        let engine = guard.as_mut().ok_or_else(|| "синхронизация не настроена".to_string())?;
        engine.update_config(f).str_err()?;
        let cfg = engine.config().clone();
        drop(guard);
        if let Some(acc) = self.account.lock().unwrap().as_mut() {
            acc.1 = cfg.clone();
        }
        self.set_status(app, |s| s.share_reminders = cfg.share_reminders);
        Ok(())
    }
}

fn mode_name(mode: VaultMode) -> String {
    match mode {
        VaultMode::E2e => "e2e".into(),
        VaultMode::Server => "server".into(),
    }
}

/// Stops syncing the previous vault and starts syncing `vault` if it is set up.
pub fn on_vault_opened(app: AppHandle, handle: Arc<SyncHandle>, vault: Vault) {
    handle.stop_tasks();
    tauri::async_runtime::spawn(async move {
        match SyncEngine::load(vault) {
            Ok(Some(engine)) => activate(app, handle, engine).await,
            Ok(None) => {
                *handle.engine.lock().await = None;
                *handle.account.lock().unwrap() = None;
                handle.set_status(&app, |s| *s = off_status());
            }
            Err(e) => {
                *handle.engine.lock().await = None;
                *handle.account.lock().unwrap() = None;
                handle.set_status(&app, |s| {
                    *s = SyncStatus { state: "error".into(), last_error: Some(e.to_string()), ..Default::default() }
                });
            }
        }
    });
}

async fn activate(app: AppHandle, handle: Arc<SyncHandle>, engine: SyncEngine) {
    handle.stop_tasks();
    let cfg = engine.config().clone();
    let client = engine.client().clone();
    let last_sync = engine.last_sync();
    *handle.engine.lock().await = Some(engine);
    *handle.account.lock().unwrap() = Some((client.clone(), cfg.clone()));
    handle.set_status(&app, |s| {
        *s = SyncStatus {
            configured: true,
            server: Some(cfg.server.clone()),
            username: Some(cfg.username.clone()),
            vault_id: Some(cfg.vault_id.clone()),
            vault_name: Some(cfg.vault_name.clone()),
            mode: Some(mode_name(cfg.mode)),
            read_only: cfg.read_only,
            share_reminders: cfg.share_reminders,
            state: "idle".into(),
            last_sync,
            ..Default::default()
        }
    });

    // Reminders in server vaults are in the owner's local time.
    let tz = client.clone();
    tauri::async_runtime::spawn(async move {
        let tz_offset_min = chrono::Local::now().offset().local_minus_utc() / 60;
        let _ = tz.set_me(&MeSettings { tz_offset_min }).await;
    });

    let (h, a) = (handle.clone(), app.clone());
    let sync_loop = tauri::async_runtime::spawn(async move {
        loop {
            let _ = run_once(&a, &h).await;
            tokio::select! {
                _ = h.kick.notified() => tokio::time::sleep(DEBOUNCE).await,
                _ = tokio::time::sleep(POLL) => {}
            }
        }
    });

    let h = handle.clone();
    let listener = tauri::async_runtime::spawn(async move {
        let mut backoff = 2;
        loop {
            if client.watch(&cfg.vault_id, |_| h.kick()).await.is_ok() {
                backoff = 2;
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(60);
        }
    });

    handle.tasks.lock().unwrap().extend([sync_loop, listener]);
}

async fn run_once(app: &AppHandle, handle: &SyncHandle) -> CmdResult<SyncReport> {
    let mut guard = handle.engine.lock().await;
    let Some(engine) = guard.as_mut() else {
        return Err("синхронизация не настроена".into());
    };
    handle.set_status(app, |s| s.state = "syncing".into());
    let result = engine.sync().await;
    let last_sync = engine.last_sync();
    let cfg = engine.config().clone();
    drop(guard);

    match &result {
        Ok(report) => handle.set_status(app, |s| {
            // The owner may have changed our role or renamed the vault.
            s.read_only = cfg.read_only;
            s.vault_name = Some(cfg.vault_name.clone());
            s.state = "idle".into();
            s.last_sync = last_sync;
            s.last_error = None;
            s.needs_login = false;
            s.last_report = Some(report.clone());
        }),
        Err(e) => handle.set_status(app, |s| {
            s.state = "error".into();
            s.last_error = Some(e.to_string());
            s.needs_login = matches!(e, SyncError::Unauthorized(_));
        }),
    }
    result.str_err()
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.sync.status()
}

#[derive(Serialize)]
pub struct LoginResult {
    username: String,
    server: ServerInfo,
    vaults: Vec<VaultSummary>,
}

/// Logs in (or registers) and lists the account's remote vaults. The token
/// is kept until `sync_connect` picks a vault.
#[tauri::command]
pub async fn sync_login(
    state: State<'_, AppState>,
    server: String,
    username: String,
    password: String,
    register: bool,
) -> CmdResult<LoginResult> {
    let client = ApiClient::new(&server).str_err()?;
    let info = client.info().await.str_err()?;
    let auth = if register {
        client.register(&username, &password).await
    } else {
        client.login(&username, &password).await
    }
    .str_err()?;
    let vaults = client.clone().with_token(&auth.token).list_vaults().await.str_err()?;
    *state.sync.pending.lock().unwrap() =
        Some(PendingLogin { server: client.base().to_string(), username: auth.username.clone(), token: auth.token });
    Ok(LoginResult { username: auth.username, server: info, vaults })
}

/// Links the open vault to a remote vault (an existing one by id, or a new
/// one named `new_name` in `mode`), unlocking e2e vaults with the vault password.
#[tauri::command]
pub async fn sync_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    vault_id: Option<String>,
    new_name: Option<String>,
    mode: Option<String>,
    password: Option<String>,
) -> CmdResult<SyncStatus> {
    let pending = state.sync.pending.lock().unwrap().take().ok_or_else(|| "сначала войдите на сервер".to_string())?;
    let vault = current_vault(&state)?;

    let result = async {
        let client = ApiClient::new(&pending.server)?.with_token(&pending.token);
        let vault_id = match vault_id {
            Some(id) => id,
            None => {
                let name = new_name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| vault.name());
                let mode = if mode.as_deref() == Some("server") { VaultMode::Server } else { VaultMode::E2e };
                SyncEngine::create_remote_vault(&client, name.trim(), mode, password.as_deref(), KdfParams::DEFAULT).await?.id
            }
        };
        SyncEngine::connect(vault, &pending.server, &pending.username, &pending.token, &vault_id, password.as_deref()).await
    }
    .await;

    match result {
        Ok(engine) => {
            activate(app, state.sync.clone(), engine).await;
            Ok(state.sync.status())
        }
        Err(e) => {
            // Keep the login so the user can retry, e.g. with another password.
            *state.sync.pending.lock().unwrap() = Some(pending);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn sync_now(app: AppHandle, state: State<'_, AppState>) -> CmdResult<SyncReport> {
    run_once(&app, &state.sync).await
}

#[tauri::command]
pub async fn sync_disconnect(app: AppHandle, state: State<'_, AppState>) -> CmdResult<SyncStatus> {
    state.sync.stop_tasks();
    let engine = state.sync.engine.lock().await.take();
    *state.sync.account.lock().unwrap() = None;
    if let Some(engine) = engine {
        engine.disconnect().str_err()?;
    }
    state.sync.set_status(&app, |s| *s = off_status());
    Ok(state.sync.status())
}
