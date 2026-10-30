//! Sharing and integrations of the connected server account: vault members,
//! the Telegram bot, the reminder webhook, API tokens, and how to plug Bedrock
//! into Claude (MCP).

use bedrock_core::index::ReminderItem;
use bedrock_sync::proto::{
    ApiToken, CreatedToken, Member, ReminderWebhook, Role, TelegramLink, TelegramSettings, TelegramStatus, WebhookTestResult,
};
use bedrock_sync::{ApiClient, SyncConfig};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::{with_ws, CmdResult, StrErr};
use crate::state::AppState;

fn connected(state: &AppState) -> CmdResult<(ApiClient, SyncConfig)> {
    state.sync.account().ok_or_else(|| "сначала подключите синхронизацию".to_string())
}

#[tauri::command]
pub async fn vault_members(state: State<'_, AppState>) -> CmdResult<Vec<Member>> {
    let (client, cfg) = connected(&state)?;
    client.members(&cfg.vault_id).await.str_err()
}

#[tauri::command]
pub async fn add_vault_member(state: State<'_, AppState>, username: String, role: String) -> CmdResult<Vec<Member>> {
    let (client, cfg) = connected(&state)?;
    let role = if role == "viewer" { Role::Viewer } else { Role::Editor };
    client.add_member(&cfg.vault_id, username.trim(), role).await.str_err()
}

#[tauri::command]
pub async fn remove_vault_member(state: State<'_, AppState>, username: String) -> CmdResult<Vec<Member>> {
    let (client, cfg) = connected(&state)?;
    client.remove_member(&cfg.vault_id, &username).await.str_err()
}

#[tauri::command]
pub async fn telegram_status(state: State<'_, AppState>) -> CmdResult<TelegramStatus> {
    connected(&state)?.0.telegram().await.str_err()
}

#[tauri::command]
pub async fn telegram_link(state: State<'_, AppState>) -> CmdResult<TelegramLink> {
    connected(&state)?.0.telegram_link().await.str_err()
}

#[tauri::command]
pub async fn telegram_set_changes(state: State<'_, AppState>, enabled: bool) -> CmdResult<TelegramStatus> {
    connected(&state)?.0.telegram_settings(&TelegramSettings { notify_changes: enabled }).await.str_err()
}

#[tauri::command]
pub async fn telegram_unlink(state: State<'_, AppState>) -> CmdResult<TelegramStatus> {
    connected(&state)?.0.telegram_unlink().await.str_err()
}

/// Adds the user's own Telegram bot (token from @BotFather).
#[tauri::command]
pub async fn telegram_set_bot(state: State<'_, AppState>, token: String) -> CmdResult<TelegramStatus> {
    connected(&state)?.0.telegram_set_bot(token.trim()).await.str_err()
}

#[tauri::command]
pub async fn telegram_remove_bot(state: State<'_, AppState>) -> CmdResult<TelegramStatus> {
    connected(&state)?.0.telegram_remove_bot().await.str_err()
}

#[tauri::command]
pub async fn reminder_webhook(state: State<'_, AppState>) -> CmdResult<ReminderWebhook> {
    connected(&state)?.0.reminder_webhook().await.str_err()
}

/// An empty secret keeps the saved one.
#[tauri::command]
pub async fn set_reminder_webhook(state: State<'_, AppState>, url: String, secret: String) -> CmdResult<ReminderWebhook> {
    let secret = Some(secret.trim()).filter(|s| !s.is_empty());
    connected(&state)?.0.set_reminder_webhook(url.trim(), secret).await.str_err()
}

#[tauri::command]
pub async fn delete_reminder_webhook(state: State<'_, AppState>) -> CmdResult<ReminderWebhook> {
    connected(&state)?.0.delete_reminder_webhook().await.str_err()
}

#[tauri::command]
pub async fn test_reminder_webhook(state: State<'_, AppState>) -> CmdResult<WebhookTestResult> {
    connected(&state)?.0.test_reminder_webhook().await.str_err()
}

#[tauri::command]
pub async fn api_tokens(state: State<'_, AppState>) -> CmdResult<Vec<ApiToken>> {
    connected(&state)?.0.tokens().await.str_err()
}

#[tauri::command]
pub async fn create_api_token(state: State<'_, AppState>, name: String) -> CmdResult<CreatedToken> {
    connected(&state)?.0.create_token(name.trim()).await.str_err()
}

#[tauri::command]
pub async fn delete_api_token(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    connected(&state)?.0.delete_token(id).await.str_err()
}

/// E2e vaults: whether reminder texts may go to the server for the Telegram bot.
#[tauri::command]
pub async fn set_share_reminders(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    state.sync.update_config(&app, |c| c.share_reminders = enabled).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    /// The `bedrock-mcp` program next to the app, if it is there.
    binary: Option<String>,
    vault: Option<String>,
    server: Option<String>,
    /// The connected vault can be used by remote MCP (server mode).
    server_vault: bool,
}

#[tauri::command]
pub fn mcp_info(state: State<'_, AppState>) -> McpInfo {
    let exe = if cfg!(windows) { "bedrock-mcp.exe" } else { "bedrock-mcp" };
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(exe)))
        .filter(|p| p.exists())
        .map(|p| dunce_path(&p));
    let vault = with_ws(&state, |ws| Ok(dunce_path(ws.vault.root()))).ok();
    let account = state.sync.account();
    McpInfo {
        binary,
        vault,
        server: account.as_ref().map(|a| a.1.server.clone()),
        server_vault: account.is_some_and(|a| a.1.mode == bedrock_sync::proto::VaultMode::Server),
    }
}

/// A path as people write it (no `\\?\` prefix on Windows).
fn dunce_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

#[tauri::command]
pub fn list_reminders(state: State<'_, AppState>) -> CmdResult<Vec<ReminderItem>> {
    with_ws(&state, |ws| Ok(ws.index.reminders()))
}
