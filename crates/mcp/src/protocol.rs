//! JSON-RPC 2.0 messages of the Model Context Protocol (tools only).

use serde_json::{json, Value};

use crate::{tools, Backend};

const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Sent to the model on connect; the full guide is docs/bedrock-for-ai.md.
const INSTRUCTIONS: &str = "Bedrock is an Obsidian-compatible notes and tasks app: a vault is a folder of Markdown files, \
and every write here lands in the user's notes (shared vaults: other members see it too).\n\
Format: tasks are `- [ ] text 📅 YYYY-MM-DD ⏫` (📅 due, ⏫/🔼/🔽 priority, `- [x] … ✅ date` when done); \
reminders are `⏰ YYYY-MM-DD HH:MM` in the user's local time; daily notes are `Дневник/YYYY-MM-DD.md`.\n\
How to work: find paths with list_notes/search_notes instead of guessing. To mark a task done, get its path and \
0-based line from list_tasks, then call complete_task with path, line and part of its text (lines shift after edits). \
Prefer edit_note or append_to_note for changes; write_note replaces the whole note, so read it first, and use it or \
delete_note only when the user asks. Errors come back as readable text: adjust the call instead of repeating it. \
Answer the user in plain words, not JSON.";

/// Handles one message (or a batch). Returns the response, or `None` for notifications.
pub async fn handle<B: Backend>(backend: &B, msg: Value) -> Option<Value> {
    if let Value::Array(batch) = msg {
        let mut out = Vec::new();
        for m in batch {
            if let Some(r) = Box::pin(handle(backend, m)).await {
                out.push(r);
            }
        }
        return (!out.is_empty()).then_some(Value::Array(out));
    }
    let id = msg.get("id").cloned().filter(|v| !v.is_null());
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    // Notifications ("notifications/initialized", cancellations) get no answer.
    let id = id?;

    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "bedrock", "title": "Bedrock", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions(backend.multi_vault()) })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // Tool failures are results the model can read and react to.
            Ok(match tools::call(backend, name, &args).await {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e.0 }], "isError": true }),
            })
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

/// The error answer to a line that is not JSON.
pub fn parse_error(detail: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {detail}") } })
}
