//! Tauri commands for the vault: files, notes, links, search, tasks, tags.

use std::path::{Path, PathBuf};

use bedrock_core::index::{NoteInfo, NoteRef, NoteSummary, SearchHit, SwitchHit, TagCount, TaskItem};
use bedrock_core::merge::{self, MergeOutcome};
use bedrock_core::vault::{self, Entry, Vault};
use bedrock_core::{ops, render, Index};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::state::{AppState, Workspace};
use crate::{sync_task, watcher};

pub type CmdResult<T> = Result<T, String>;

pub trait StrErr<T> {
    fn str_err(self) -> CmdResult<T>;
}

impl<T, E: std::fmt::Display> StrErr<T> for Result<T, E> {
    fn str_err(self) -> CmdResult<T> {
        self.map_err(|e| e.to_string())
    }
}

pub fn with_ws<T>(state: &AppState, f: impl FnOnce(&Workspace) -> CmdResult<T>) -> CmdResult<T> {
    let guard = state.ws.read().map_err(|_| "внутренняя ошибка блокировки".to_string())?;
    f(guard.as_ref().ok_or_else(|| "хранилище не открыто".to_string())?)
}

fn with_ws_mut<T>(state: &AppState, f: impl FnOnce(&mut Workspace) -> CmdResult<T>) -> CmdResult<T> {
    let mut guard = state.ws.write().map_err(|_| "внутренняя ошибка блокировки".to_string())?;
    f(guard.as_mut().ok_or_else(|| "хранилище не открыто".to_string())?)
}

pub fn current_vault(state: &AppState) -> CmdResult<Vault> {
    with_ws(state, |ws| Ok(ws.vault.clone()))
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

// ---------------------------------------------------------------- vaults

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInfo {
    root: String,
    name: String,
    notes: usize,
}

fn recent_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("recent.json"))
}

#[tauri::command]
pub fn recent_vaults(app: AppHandle) -> Vec<String> {
    recent_file(&app)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| Path::new(p).is_dir())
        .collect()
}

fn remember_recent(app: &AppHandle, root: &str) {
    let mut list = recent_vaults(app.clone());
    list.retain(|p| p != root);
    list.insert(0, root.to_string());
    list.truncate(10);
    if let Some(path) = recent_file(app) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, serde_json::to_vec(&list).unwrap_or_default());
    }
}

#[tauri::command]
pub async fn open_vault(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<VaultInfo> {
    let (vault, index) = tauri::async_runtime::spawn_blocking(move || {
        let vault = Vault::open(&path)?;
        let index = Index::build(&vault);
        Ok::<_, bedrock_core::Error>((vault, index))
    })
    .await
    .str_err()?
    .str_err()?;

    let info = VaultInfo { root: vault.root().display().to_string(), name: vault.name(), notes: index.len() };
    app.asset_protocol_scope().allow_directory(vault.root(), true).str_err()?;
    *state.ws.write().map_err(|_| "внутренняя ошибка блокировки".to_string())? =
        Some(Workspace { vault: vault.clone(), index });

    let w = watcher::start(app.clone(), state.ws.clone(), state.sync.clone(), vault.clone()).str_err()?;
    *state.watcher.lock().map_err(|_| "внутренняя ошибка блокировки".to_string())? = Some(w);

    remember_recent(&app, &info.root);
    sync_task::on_vault_opened(app, state.sync.clone(), vault);
    Ok(info)
}

// ---------------------------------------------------------------- files

#[tauri::command]
pub fn list_files(state: State<'_, AppState>) -> CmdResult<Vec<Entry>> {
    with_ws(&state, |ws| Ok(ws.vault.walk()))
}

#[tauri::command]
pub fn read_note(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    with_ws(&state, |ws| ws.vault.read_string(&path).str_err())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    /// What is on disk now; differs from what the editor sent after a merge.
    content: String,
    merged: bool,
    conflict: Option<String>,
}

/// Saves editor content. `base` is the content the editor last loaded or
/// saved: if the file changed on disk since (sync, another editor), both
/// edits are merged instead of overwriting the other one.
#[tauri::command]
pub fn save_note(state: State<'_, AppState>, path: String, content: String, base: String) -> CmdResult<SaveResult> {
    with_ws_mut(&state, |ws| {
        let disk = ws.vault.read_string_opt(&path).str_err()?;
        let mut result = SaveResult { content: content.clone(), merged: false, conflict: None };
        if let Some(disk) = disk.filter(|d| *d != base && *d != content) {
            match merge::merge3(&base, &content, &disk) {
                MergeOutcome::Clean(m) => {
                    result.content = m;
                    result.merged = true;
                }
                MergeOutcome::Conflict => {
                    let copy = merge::conflict_copy_path(&ws.vault, &path, "disk").str_err()?;
                    ws.vault.write(&copy, disk.as_bytes()).str_err()?;
                    ws.index.update_path(&ws.vault, &copy);
                    result.conflict = Some(copy);
                }
            }
        }
        ws.vault.write(&path, result.content.as_bytes()).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(result)
    })
}

#[tauri::command]
pub fn create_note(state: State<'_, AppState>, folder: Option<String>, name: Option<String>) -> CmdResult<String> {
    with_ws_mut(&state, |ws| {
        let path = ops::create_note(&ws.vault, folder.as_deref(), name.as_deref()).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(path)
    })
}

#[tauri::command]
pub fn create_folder(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    with_ws(&state, |ws| {
        let path = ws.vault.unique_path(&path).str_err()?;
        ws.vault.create_dir(&path).str_err()?;
        Ok(path)
    })
}

#[derive(Serialize)]
pub struct RenameResult {
    path: String,
    updated: Vec<String>,
}

#[tauri::command]
pub fn rename_path(state: State<'_, AppState>, from: String, to: String) -> CmdResult<RenameResult> {
    with_ws_mut(&state, |ws| {
        let path = vault::normalize(&to).str_err()?;
        let updated = ops::rename_with_links(&ws.vault, &mut ws.index, &from, &path).str_err()?;
        Ok(RenameResult { path, updated })
    })
}

/// Moves to the vault's `.trash/`; returns the trashed name for `restore_path`.
#[tauri::command]
pub fn delete_path(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    with_ws_mut(&state, |ws| {
        let trashed = ws.vault.trash(&path).str_err()?;
        ws.index.remove_path(&path);
        Ok(trashed)
    })
}

/// Drops an untitled note that was left without any text. Does nothing (and
/// returns false) once the note has content.
#[tauri::command]
pub fn discard_blank_note(state: State<'_, AppState>, path: String) -> CmdResult<bool> {
    with_ws_mut(&state, |ws| {
        let removed = ws.vault.remove_if_blank(&path).str_err()?;
        if removed {
            ws.index.remove_path(&path);
        }
        Ok(removed)
    })
}

/// Undo for `delete_path`.
#[tauri::command]
pub fn restore_path(state: State<'_, AppState>, trashed: String, path: String) -> CmdResult<String> {
    with_ws_mut(&state, |ws| {
        let restored = ws.vault.restore(&trashed, &path).str_err()?;
        ws.index.update_path(&ws.vault, &restored);
        Ok(restored)
    })
}

// ---------------------------------------------------------------- links

#[tauri::command]
pub fn note_info(state: State<'_, AppState>, path: String) -> CmdResult<Option<NoteInfo>> {
    with_ws(&state, |ws| Ok(ws.index.note_info(&path)))
}

#[tauri::command]
pub fn render_note(state: State<'_, AppState>, path: String, content: String) -> CmdResult<String> {
    with_ws(&state, |ws| Ok(render::render(&content, &ws.index, &path)))
}

#[tauri::command]
pub fn all_notes(state: State<'_, AppState>) -> CmdResult<Vec<NoteRef>> {
    with_ws(&state, |ws| Ok(ws.index.all_notes()))
}

/// Resolves a link; with `create`, a missing note is created (like Obsidian).
#[tauri::command]
pub fn follow_link(state: State<'_, AppState>, target: String, from: String, create: bool) -> CmdResult<Option<String>> {
    with_ws_mut(&state, |ws| {
        if let Some(path) = ws.index.resolve(&target, &from) {
            return Ok(Some(path));
        }
        if !create || target.trim().is_empty() {
            return Ok(None);
        }
        let mut path = vault::normalize(&target).str_err()?;
        if vault::extension(&path).is_empty() {
            path.push_str(".md");
        }
        if !ws.vault.exists(&path) {
            ws.vault.write(&path, b"").str_err()?;
        }
        ws.index.update_path(&ws.vault, &path);
        Ok(Some(path))
    })
}

// ---------------------------------------------------------------- search

#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String) -> CmdResult<Vec<SearchHit>> {
    with_ws(&state, |ws| Ok(ws.index.search(&query, 100)))
}

#[tauri::command]
pub fn quick_switch(state: State<'_, AppState>, query: String) -> CmdResult<Vec<SwitchHit>> {
    with_ws(&state, |ws| Ok(ws.index.quick_switch(&query, 50)))
}

// ---------------------------------------------------------------- tasks & tags

#[tauri::command]
pub fn list_tasks(state: State<'_, AppState>, include_done: bool) -> CmdResult<Vec<TaskItem>> {
    with_ws(&state, |ws| Ok(ws.index.tasks(include_done)))
}

#[tauri::command]
pub fn toggle_task(state: State<'_, AppState>, path: String, line: usize) -> CmdResult<String> {
    with_ws_mut(&state, |ws| {
        let content = ops::toggle_task(&ws.vault, &path, line, today()).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(content)
    })
}

/// Sets a task's status: "x" done, " " to do, "/" in progress (the board's columns).
#[tauri::command]
pub fn set_task_status(state: State<'_, AppState>, path: String, line: usize, status: String) -> CmdResult<String> {
    let status = status.chars().next().ok_or("пустой статус")?;
    with_ws_mut(&state, |ws| {
        let content = ops::set_task_status(&ws.vault, &path, line, status, today()).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(content)
    })
}

#[derive(serde::Serialize)]
pub struct LineDone {
    line: String,
    next: Option<String>,
}

/// A task line from the editor marked done or not: the new line and, for a
/// repeating task, its next occurrence to put above it.
#[tauri::command]
pub fn task_line_done(line: String, done: bool) -> CmdResult<LineDone> {
    let (line, next) = ops::text::line_done(&line, done, today()).str_err()?;
    Ok(LineDone { line, next })
}

/// Where pasted, dropped and picked files go (Obsidian's «attachments folder»).
const ATTACH_DIR: &str = "Вложения";
const ATTACH_MAX: usize = 64 * 1024 * 1024;

#[derive(serde::Serialize)]
pub struct Attached {
    path: String,
    /// Shortest link text that finds the file: `![[link]]` or `[[link]]`.
    link: String,
    image: bool,
}

fn store_attachment(ws: &mut Workspace, name: &str, bytes: &[u8]) -> CmdResult<Attached> {
    if bytes.len() > ATTACH_MAX {
        return Err(format!("файл больше {} МБ", ATTACH_MAX / 1024 / 1024));
    }
    let clean: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c }).collect();
    let clean = clean.trim().trim_start_matches('.');
    let file = if clean.is_empty() { "файл" } else { clean };
    let rel = ws.vault.unique_path(&vault::join(ATTACH_DIR, file)).str_err()?;
    ws.vault.write(&rel, bytes).str_err()?;
    ws.index.update_path(&ws.vault, &rel);
    Ok(Attached { link: ws.index.link_text_for(&rel), image: vault::is_image(&rel), path: rel })
}

/// Saves a pasted file (base64) into the attachments folder.
#[tauri::command]
pub fn save_attachment(state: State<'_, AppState>, name: String, data: String) -> CmdResult<Attached> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).str_err()?;
    with_ws_mut(&state, |ws| store_attachment(ws, &name, &bytes))
}

/// Copies a file from the computer (dropped on the editor or picked) into the attachments folder.
#[tauri::command]
pub fn import_attachment(state: State<'_, AppState>, source: String) -> CmdResult<Attached> {
    let src = std::path::PathBuf::from(&source);
    let name = src.file_name().and_then(|n| n.to_str()).ok_or("у файла нет имени")?.to_string();
    if std::fs::metadata(&src).str_err()?.len() as usize > ATTACH_MAX {
        return Err(format!("файл больше {} МБ", ATTACH_MAX / 1024 / 1024));
    }
    let bytes = std::fs::read(&src).str_err()?;
    with_ws_mut(&state, |ws| store_attachment(ws, &name, &bytes))
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>) -> CmdResult<Vec<TagCount>> {
    with_ws(&state, |ws| Ok(ws.index.tags()))
}

fn parse_date(date: Option<&str>) -> CmdResult<chrono::NaiveDate> {
    match date {
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").str_err(),
        None => Ok(today()),
    }
}

/// Opens (creating if needed) the daily note for `date` (YYYY-MM-DD, default today).
#[tauri::command]
pub fn daily_note(state: State<'_, AppState>, date: Option<String>) -> CmdResult<String> {
    let date = parse_date(date.as_deref())?;
    with_ws_mut(&state, |ws| {
        let path = ops::daily_note(&ws.vault, date).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(path)
    })
}

/// Sets (or with `due: None` clears) a task's due date.
#[tauri::command]
pub fn set_task_due(state: State<'_, AppState>, path: String, line: usize, due: Option<String>) -> CmdResult<String> {
    let due = due.as_deref().map(|d| parse_date(Some(d))).transpose()?;
    with_ws_mut(&state, |ws| {
        let content = ops::set_task_due(&ws.vault, &path, line, due).str_err()?;
        ws.index.update_path(&ws.vault, &path);
        Ok(content)
    })
}

#[tauri::command]
pub fn list_notes(state: State<'_, AppState>) -> CmdResult<Vec<NoteSummary>> {
    with_ws(&state, |ws| Ok(ws.index.summaries()))
}
