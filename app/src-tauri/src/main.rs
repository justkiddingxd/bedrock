#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod commands;
mod reminders;
mod state;
mod sync_task;
mod watcher;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(state::AppState::default())
        .setup(|app| {
            reminders::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::recent_vaults,
            commands::open_vault,
            commands::list_files,
            commands::read_note,
            commands::save_note,
            commands::create_note,
            commands::create_folder,
            commands::rename_path,
            commands::delete_path,
            commands::note_info,
            commands::render_note,
            commands::all_notes,
            commands::follow_link,
            commands::search,
            commands::quick_switch,
            commands::list_tasks,
            commands::toggle_task,
            commands::list_tags,
            commands::daily_note,
            commands::set_task_due,
            commands::restore_path,
            commands::discard_blank_note,
            commands::list_notes,
            sync_task::sync_status,
            sync_task::sync_login,
            sync_task::sync_connect,
            sync_task::sync_now,
            sync_task::sync_disconnect,
            account::vault_members,
            account::add_vault_member,
            account::remove_vault_member,
            account::telegram_status,
            account::telegram_link,
            account::telegram_set_changes,
            account::telegram_unlink,
            account::telegram_set_bot,
            account::telegram_remove_bot,
            account::api_tokens,
            account::create_api_token,
            account::delete_api_token,
            account::reminder_webhook,
            account::set_reminder_webhook,
            account::delete_reminder_webhook,
            account::test_reminder_webhook,
            account::set_share_reminders,
            account::mcp_info,
            account::list_reminders,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bedrock");
}
