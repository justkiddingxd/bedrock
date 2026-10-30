//! The tools end to end over JSON-RPC, on a vault folder.

use bedrock_core::Vault;
use bedrock_mcp::local::LocalBackend;
use bedrock_mcp::protocol::handle;
use serde_json::{json, Value};

async fn call(b: &LocalBackend, name: &str, args: Value) -> (bool, String) {
    let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let r = handle(b, msg).await.unwrap();
    let res = &r["result"];
    (res["isError"].as_bool().unwrap(), res["content"][0]["text"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn tools_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    vault.write("Работа/План.md", "# План\n\n## Задачи\n- [ ] Созвон с Пашей\n\n## Заметки\nтекст\n".as_bytes()).unwrap();
    let b = LocalBackend::new(vault.clone());

    let init = handle(&b, json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } })).await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert!(handle(&b, json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await.is_none());
    let list = handle(&b, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await.unwrap();
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"complete_task") && !names.contains(&"list_vaults"));

    // A task in a note found by its bare title, then marked done by text.
    let (err, out) = call(&b, "add_task", json!({ "text": "Отправить отчёт", "due": "2026-11-02", "priority": "high", "path": "План" })).await;
    assert!(!err, "{out}");
    let plan = vault.read_string("Работа/План.md").unwrap();
    assert!(plan.contains("- [ ] Созвон с Пашей\n- [ ] Отправить отчёт ⏫ 📅 2026-11-02\n"), "{plan}");
    let (_, tasks) = call(&b, "list_tasks", json!({})).await;
    let tasks: Value = serde_json::from_str(&tasks).unwrap();
    assert_eq!(tasks["tasks"][0]["text"], "Отправить отчёт", "due first");
    let (err, out) = call(&b, "complete_task", json!({ "path": "Работа/План.md", "text": "созвон" })).await;
    assert!(!err, "{out}");
    assert!(vault.read_string("Работа/План.md").unwrap().contains("- [x] Созвон с Пашей ✅ "));

    // Reminders land in today's daily note with a due date.
    let (err, out) = call(&b, "add_reminder", json!({ "text": "Позвонить маме", "at": "2026-10-30 09:00" })).await;
    assert!(!err, "{out}");
    let (_, rem) = call(&b, "list_reminders", json!({})).await;
    assert!(rem.contains("\"at\": \"2026-10-30 09:00\"") && rem.contains("Позвонить маме"), "{rem}");

    // Free names, exact edits, search, and errors the model can read.
    let (_, a) = call(&b, "create_note", json!({ "title": "Идеи: список", "content": "раз" })).await;
    let (_, b2) = call(&b, "create_note", json!({ "title": "Идеи: список" })).await;
    assert!(a.contains("Идеи- список.md") && b2.contains("Идеи- список 1.md"), "{a} {b2}");
    let (err, _) = call(&b, "edit_note", json!({ "path": "Идеи- список", "old_text": "раз", "new_text": "два" })).await;
    assert!(!err);
    let (_, found) = call(&b, "search_notes", json!({ "query": "ДВА" })).await;
    assert!(found.contains("Идеи- список.md"), "{found}");
    let (err, msg) = call(&b, "read_note", json!({ "path": "Нет такой" })).await;
    assert!(err && msg.contains("не найдена"));
    let (err, _) = call(&b, "write_note", json!({ "path": "../evil", "content": "x" })).await;
    assert!(err);
}
