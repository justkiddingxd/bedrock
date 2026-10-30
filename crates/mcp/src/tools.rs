//! The tools: notes, tasks and reminders of a vault.

use bedrock_core::ops::{self, text};
use bedrock_core::parse;
use bedrock_core::vault;
use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{json, Value};

use crate::{Backend, FileMeta, Result, Store, ToolError};

const VAULT_CONVENTIONS: &str = "Notes are Markdown files (Obsidian-compatible). \
Tasks are lines like `- [ ] text 📅 2026-10-30 ⏫` (📅 due date, ⏫ high priority, ✅ date when done). \
Reminders are `⏰ YYYY-MM-DD HH:MM` (local time) on a line. Daily notes live in `Дневник/YYYY-MM-DD.md`.";

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": props, "required": required },
    })
}

/// Tool definitions for `tools/list`. With several vaults every tool but
/// `list_vaults` takes a `vault` argument.
pub fn definitions(multi_vault: bool) -> Vec<Value> {
    let path = json!({ "type": "string", "description": "Note path relative to the vault, e.g. \"Работа/План.md\" (\".md\" may be omitted; a bare title is matched too)." });
    let mut tools = vec![
        tool(
            "list_notes",
            &format!("List notes of the vault, most recently changed first. {VAULT_CONVENTIONS}"),
            json!({
                "folder": { "type": "string", "description": "Only notes inside this folder." },
                "limit": { "type": "integer", "description": "At most this many (default 100)." },
            }),
            &[],
        ),
        tool("read_note", "Read the full Markdown text of a note.", json!({ "path": path }), &["path"]),
        tool(
            "search_notes",
            "Find notes whose name or text contains the query (case-insensitive). Returns matching lines.",
            json!({
                "query": { "type": "string" },
                "limit": { "type": "integer", "description": "At most this many matches (default 30)." },
            }),
            &["query"],
        ),
        tool(
            "create_note",
            "Create a new note. A free name is picked if the title is taken. Returns its path.",
            json!({
                "title": { "type": "string", "description": "Note title, becomes the file name." },
                "content": { "type": "string", "description": "Markdown text." },
                "folder": { "type": "string", "description": "Folder to put it in (created if missing)." },
            }),
            &["title"],
        ),
        tool(
            "write_note",
            "Replace the whole text of a note, creating it if it does not exist.",
            json!({ "path": path, "content": { "type": "string" } }),
            &["path", "content"],
        ),
        tool(
            "edit_note",
            "Replace one exact piece of a note's text with another. `old_text` must occur exactly once.",
            json!({ "path": path, "old_text": { "type": "string" }, "new_text": { "type": "string" } }),
            &["path", "old_text", "new_text"],
        ),
        tool(
            "append_to_note",
            "Add text on a new line at the end of a note (created if missing).",
            json!({ "path": path, "text": { "type": "string" } }),
            &["path", "text"],
        ),
        tool("delete_note", "Delete a note.", json!({ "path": path }), &["path"]),
        tool(
            "list_tasks",
            "List tasks across the vault with their note path and line number (needed by complete_task).",
            json!({
                "status": { "type": "string", "enum": ["open", "done", "all"], "description": "Default: open." },
                "path": { "type": "string", "description": "Only tasks of this note." },
                "due_before": { "type": "string", "description": "Only tasks due on or before this date (YYYY-MM-DD), overdue included." },
            }),
            &[],
        ),
        tool(
            "add_task",
            "Add a task (`- [ ] text`). Goes to today's daily note unless `path` is given; inside a \"Задачи\" section if the note has one.",
            json!({
                "text": { "type": "string" },
                "due": { "type": "string", "description": "Due date YYYY-MM-DD." },
                "priority": { "type": "string", "enum": ["high", "medium", "normal", "low"] },
                "path": { "type": "string", "description": "Note to add it to." },
            }),
            &["text"],
        ),
        tool(
            "complete_task",
            "Mark a task done (adds ✅ with today's date), or not done with done=false. Find it by `line` from list_tasks, or by part of its `text`.",
            json!({
                "path": path,
                "line": { "type": "integer", "description": "0-based line number from list_tasks." },
                "text": { "type": "string", "description": "Part of the task text, if the line is unknown." },
                "done": { "type": "boolean", "description": "Default: true." },
            }),
            &["path"],
        ),
        tool(
            "add_reminder",
            "Add a reminder: a task with ⏰ that the Bedrock app (and the Telegram bot, if linked) announces at that time.",
            json!({
                "text": { "type": "string" },
                "at": { "type": "string", "description": "Local date and time, \"YYYY-MM-DD HH:MM\"." },
                "path": { "type": "string", "description": "Note to add it to (default: today's daily note)." },
            }),
            &["text", "at"],
        ),
        tool("list_reminders", "List reminders that are not done yet, soonest first.", json!({}), &[]),
        tool(
            "daily_note",
            "Get the daily note of a date (default today): its path and text.",
            json!({ "date": { "type": "string", "description": "YYYY-MM-DD." } }),
            &[],
        ),
    ];
    if multi_vault {
        let vault = json!({ "type": "string", "description": "Vault name or id (see list_vaults). May be omitted if the account has one vault." });
        for t in &mut tools {
            t["inputSchema"]["properties"]["vault"] = vault.clone();
        }
        tools.insert(0, tool("list_vaults", "List the vaults this account can use, with its role in each.", json!({}), &[]));
    }
    tools
}

/// Runs a tool and returns its text result.
pub async fn call<B: Backend>(backend: &B, name: &str, args: &Value) -> Result<String> {
    if name == "list_vaults" {
        return to_json(&backend.vaults().await?);
    }
    let store = backend.open(str_arg(args, "vault")).await?;
    let writes = matches!(
        name,
        "create_note" | "write_note" | "edit_note" | "append_to_note" | "delete_note" | "add_task" | "complete_task" | "add_reminder"
    );
    if writes && !store.can_write() {
        return Err("в этом хранилище у вас доступ только для чтения".into());
    }
    match name {
        "list_notes" => list_notes(&store, args).await,
        "read_note" => read_note(&store, args).await,
        "search_notes" => search_notes(&store, args).await,
        "create_note" => create_note(&store, args).await,
        "write_note" => write_note(&store, args).await,
        "edit_note" => edit_note(&store, args).await,
        "append_to_note" => append_to_note(&store, args).await,
        "delete_note" => delete_note(&store, args).await,
        "list_tasks" => list_tasks(&store, args).await,
        "add_task" => add_task(&store, args).await,
        "complete_task" => complete_task(&store, args).await,
        "add_reminder" => add_reminder(&store, args).await,
        "list_reminders" => list_reminders(&store).await,
        "daily_note" => daily_note(&store, args).await,
        _ => Err(ToolError(format!("неизвестный инструмент {name}"))),
    }
}

// ---------------------------------------------------------------- arguments

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    str_arg(args, key).ok_or_else(|| ToolError(format!("не задан параметр {key}")))
}

fn date_arg(args: &Value, key: &str) -> Result<Option<NaiveDate>> {
    str_arg(args, key)
        .map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| ToolError(format!("{key}: нужна дата YYYY-MM-DD"))))
        .transpose()
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| ToolError(e.to_string()))
}

/// A vault path for a new or existing note: ".md" added, `..` and hidden folders refused.
fn note_path(raw: &str) -> Result<String> {
    let raw = raw.trim().trim_start_matches('/');
    let with_ext = if vault::is_note(raw) { raw.to_string() } else { format!("{raw}.md") };
    let path = vault::normalize(&with_ext)?;
    if vault::is_hidden(&path) {
        return Err(ToolError(format!("недопустимый путь {path}")));
    }
    Ok(path)
}

/// File names cannot contain \ / : * ? " < > |.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c }).collect();
    cleaned.trim().trim_end_matches('.').trim().to_string()
}

fn notes_of(files: &[FileMeta]) -> impl Iterator<Item = &FileMeta> {
    files.iter().filter(|f| vault::is_note(&f.path))
}

/// Finds an existing note by path, path without ".md", or unique title.
async fn resolve<S: Store>(store: &S, raw: &str) -> Result<String> {
    let files = store.list().await?;
    let want = note_path(raw)?;
    let notes: Vec<&str> = notes_of(&files).map(|f| f.path.as_str()).collect();
    if let Some(p) = notes.iter().find(|p| **p == want) {
        return Ok(p.to_string());
    }
    let lower = want.to_lowercase();
    if let Some(p) = notes.iter().find(|p| p.to_lowercase() == lower) {
        return Ok(p.to_string());
    }
    let title = vault::stem(&want).to_lowercase();
    let by_title: Vec<&&str> = notes.iter().filter(|p| vault::stem(p).to_lowercase() == title).collect();
    match by_title.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(ToolError(format!("заметка «{raw}» не найдена"))),
        many => Err(ToolError(format!(
            "несколько заметок с названием «{raw}»: {}",
            many.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// An existing note, or a new path if there is none.
async fn resolve_or_new<S: Store>(store: &S, raw: &str) -> Result<String> {
    match resolve(store, raw).await {
        Ok(p) => Ok(p),
        Err(_) => note_path(raw),
    }
}

fn time_of(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------- notes

async fn list_notes<S: Store>(store: &S, args: &Value) -> Result<String> {
    let files = store.list().await?;
    let folder = str_arg(args, "folder").map(|f| format!("{}/", f.trim_matches('/')));
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let mut notes: Vec<&FileMeta> =
        notes_of(&files).filter(|f| folder.as_ref().is_none_or(|d| f.path.starts_with(d.as_str()))).collect();
    notes.sort_by(|a, b| b.updated.cmp(&a.updated));
    let total = notes.len();
    let list: Vec<Value> =
        notes.iter().take(limit).map(|f| json!({ "path": f.path, "updated": time_of(f.updated) })).collect();
    to_json(&json!({ "total": total, "notes": list }))
}

async fn read_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve(store, req_str(args, "path")?).await?;
    let content = store.read(&path).await?.unwrap_or_default();
    Ok(format!("Path: {path}\n\n{content}"))
}

async fn search_notes<S: Store>(store: &S, args: &Value) -> Result<String> {
    let query = req_str(args, "query")?.to_lowercase();
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(30) as usize;
    let files = store.list().await?;
    let mut hits = Vec::new();
    for f in notes_of(&files) {
        if hits.len() >= limit {
            break;
        }
        let name_hit = f.path.to_lowercase().contains(&query);
        let content = store.read(&f.path).await?.unwrap_or_default();
        let mut lines = 0;
        for (i, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&query) {
                hits.push(json!({ "path": f.path, "line": i, "text": line.trim() }));
                lines += 1;
                if lines == 3 || hits.len() >= limit {
                    break;
                }
            }
        }
        if name_hit && lines == 0 {
            hits.push(json!({ "path": f.path, "line": null, "text": "(совпадение в названии)" }));
        }
    }
    to_json(&hits)
}

async fn create_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let title = safe_name(req_str(args, "title")?);
    if title.is_empty() {
        return Err("пустое название".into());
    }
    let folder = str_arg(args, "folder").map(|f| f.trim_matches('/').to_string()).unwrap_or_default();
    let files = store.list().await?;
    let taken = |p: &str| files.iter().any(|f| f.path.eq_ignore_ascii_case(p));
    let base = note_path(&vault::join(&folder, &title))?;
    let mut path = base.clone();
    let mut n = 1;
    while taken(&path) {
        path = note_path(&vault::join(&folder, &format!("{title} {n}")))?;
        n += 1;
    }
    store.write(&path, str_arg(args, "content").unwrap_or_default()).await?;
    to_json(&json!({ "path": path }))
}

async fn write_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve_or_new(store, req_str(args, "path")?).await?;
    let content = args.get("content").and_then(Value::as_str).ok_or("не задан параметр content")?;
    let existed = store.read(&path).await?.is_some();
    store.write(&path, content).await?;
    to_json(&json!({ "path": path, "created": !existed }))
}

async fn edit_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve(store, req_str(args, "path")?).await?;
    let old = args.get("old_text").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("не задан параметр old_text")?;
    let new = args.get("new_text").and_then(Value::as_str).ok_or("не задан параметр new_text")?;
    let content = store.read(&path).await?.unwrap_or_default();
    match content.matches(old).count() {
        0 => return Err(ToolError(format!("в «{path}» нет такого текста"))),
        1 => {}
        n => return Err(ToolError(format!("текст встречается {n} раз, уточните его"))),
    }
    store.write(&path, &content.replacen(old, new, 1)).await?;
    to_json(&json!({ "path": path }))
}

async fn append_to_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve_or_new(store, req_str(args, "path")?).await?;
    let text = args.get("text").and_then(Value::as_str).ok_or("не задан параметр text")?;
    let content = store.read(&path).await?.unwrap_or_default();
    store.write(&path, &text::append(&content, text)).await?;
    to_json(&json!({ "path": path }))
}

async fn delete_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve(store, req_str(args, "path")?).await?;
    store.delete(&path).await?;
    to_json(&json!({ "deleted": path }))
}

// ---------------------------------------------------------------- tasks

fn priority_mark(p: &str) -> Result<&'static str> {
    Ok(match p {
        "high" => " ⏫",
        "medium" => " 🔼",
        "low" => " 🔽",
        "normal" => "",
        _ => return Err("priority: high, medium, normal или low".into()),
    })
}

async fn list_tasks<S: Store>(store: &S, args: &Value) -> Result<String> {
    let status = str_arg(args, "status").unwrap_or("open");
    let due_before = date_arg(args, "due_before")?.map(|d| d.format("%Y-%m-%d").to_string());
    let only = match str_arg(args, "path") {
        Some(p) => Some(resolve(store, p).await?),
        None => None,
    };
    let files = store.list().await?;
    let mut tasks = Vec::new();
    for f in notes_of(&files) {
        if only.as_ref().is_some_and(|p| *p != f.path) {
            continue;
        }
        let content = store.read(&f.path).await?.unwrap_or_default();
        for t in parse::parse(&content).tasks {
            let wanted = match status {
                "done" => t.done,
                "all" => true,
                _ => !t.done,
            };
            let in_range = due_before.as_ref().is_none_or(|d| t.due.as_ref().is_some_and(|due| due <= d));
            if wanted && in_range && !t.text.is_empty() {
                tasks.push((f.path.clone(), t));
            }
        }
    }
    tasks.sort_by(|(pa, a), (pb, b)| {
        let key = |t: &parse::Task| (t.done, t.due.is_none(), t.due.clone(), t.priority);
        key(a).cmp(&key(b)).then_with(|| pa.cmp(pb)).then(a.line.cmp(&b.line))
    });
    let list: Vec<Value> = tasks
        .iter()
        .take(300)
        .map(|(path, t)| {
            json!({
                "path": path, "line": t.line, "text": t.text, "done": t.done,
                "due": t.due, "priority": t.priority, "done_date": t.done_date,
            })
        })
        .collect();
    to_json(&json!({ "total": tasks.len(), "tasks": list }))
}

/// Inserts `line` as a task and returns where it went.
async fn insert_task<S: Store>(store: &S, path: &str, line: &str) -> Result<usize> {
    let content = store.read(path).await?.unwrap_or_default();
    let updated = text::insert_task(&content, line);
    store.write(path, &updated).await?;
    let wanted = format!("- [ ] {line}");
    Ok(updated.lines().position(|l| l == wanted).unwrap_or(0))
}

async fn add_task<S: Store>(store: &S, args: &Value) -> Result<String> {
    let text = req_str(args, "text")?.replace('\n', " ");
    let due = date_arg(args, "due")?;
    let priority = priority_mark(str_arg(args, "priority").unwrap_or("normal"))?;
    let path = match str_arg(args, "path") {
        Some(p) => resolve_or_new(store, p).await?,
        None => ops::daily_path(store.today()),
    };
    let due = due.map(|d| format!(" 📅 {}", d.format("%Y-%m-%d"))).unwrap_or_default();
    let task = format!("{text}{priority}{due}");
    let line = insert_task(store, &path, &task).await?;
    to_json(&json!({ "path": path, "line": line, "task": format!("- [ ] {task}") }))
}

async fn complete_task<S: Store>(store: &S, args: &Value) -> Result<String> {
    let path = resolve(store, req_str(args, "path")?).await?;
    let done = args.get("done").and_then(Value::as_bool).unwrap_or(true);
    let content = store.read(&path).await?.unwrap_or_default();
    let tasks = parse::parse(&content).tasks;
    let by_line = args.get("line").and_then(Value::as_u64).and_then(|l| tasks.iter().find(|t| t.line as u64 == l));
    let query = str_arg(args, "text").map(str::to_lowercase);
    let task = match (by_line, &query) {
        // The line moved if its text no longer matches.
        (Some(t), Some(q)) if !t.text.to_lowercase().contains(q.as_str()) => None,
        (Some(t), _) => Some(t),
        (None, _) => None,
    };
    let task = match (task, &query) {
        (Some(t), _) => t,
        (None, Some(q)) => {
            let matches: Vec<&parse::Task> = tasks.iter().filter(|t| t.text.to_lowercase().contains(q.as_str())).collect();
            // Prefer a task that is not yet in the requested state.
            *matches.iter().find(|t| t.done != done).or(matches.first()).ok_or_else(|| ToolError(format!("в «{path}» нет задачи «{q}»")))?
        }
        (None, None) => return Err("укажите line (из list_tasks) или text".into()),
    };
    let updated = text::set_task_done(&content, task.line, done, store.today())?;
    store.write(&path, &updated).await?;
    to_json(&json!({ "path": path, "line": task.line, "text": task.text, "done": done }))
}

// ---------------------------------------------------------------- reminders and daily notes

async fn add_reminder<S: Store>(store: &S, args: &Value) -> Result<String> {
    let text = req_str(args, "text")?.replace('\n', " ");
    let at = NaiveDateTime::parse_from_str(req_str(args, "at")?, "%Y-%m-%d %H:%M")
        .map_err(|_| ToolError("at: нужно время \"YYYY-MM-DD HH:MM\"".into()))?;
    let path = match str_arg(args, "path") {
        Some(p) => resolve_or_new(store, p).await?,
        None => ops::daily_path(store.today()),
    };
    // The due date puts the task on that day's list too.
    let task = format!("{text} 📅 {} ⏰ {}", at.format("%Y-%m-%d"), at.format("%Y-%m-%d %H:%M"));
    let line = insert_task(store, &path, &task).await?;
    to_json(&json!({ "path": path, "line": line, "at": at.format("%Y-%m-%d %H:%M").to_string() }))
}

async fn list_reminders<S: Store>(store: &S) -> Result<String> {
    let files = store.list().await?;
    let mut out = Vec::new();
    for f in notes_of(&files) {
        let content = store.read(&f.path).await?.unwrap_or_default();
        for r in parse::parse(&content).reminders.into_iter().filter(|r| !r.done) {
            out.push((r.at.clone(), json!({ "path": f.path, "line": r.line, "at": r.at, "text": r.text })));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    to_json(&out.into_iter().map(|(_, v)| v).collect::<Vec<_>>())
}

async fn daily_note<S: Store>(store: &S, args: &Value) -> Result<String> {
    let date = date_arg(args, "date")?.unwrap_or_else(|| store.today());
    let path = ops::daily_path(date);
    let content = store.read(&path).await?;
    to_json(&json!({ "path": path, "exists": content.is_some(), "content": content.unwrap_or_default() }))
}
