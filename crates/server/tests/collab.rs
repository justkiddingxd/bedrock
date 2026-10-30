//! Shared vaults, server vaults, the notes API and remote MCP, on a real server.

use bedrock_core::Vault;
use bedrock_server::{Config, Registration};
use bedrock_sync::proto::{Role, VaultMode};
use bedrock_sync::{ApiClient, KdfParams, SyncEngine, SyncError};
use serde_json::{json, Value};

async fn start(cfg: Config) -> String {
    let app = bedrock_server::build_app(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn config(data: &std::path::Path) -> Config {
    Config::new("127.0.0.1:0".parse().unwrap(), data.to_path_buf(), Registration::Open)
}

async fn account(url: &str, name: &str) -> (ApiClient, String) {
    let c = ApiClient::new(url).unwrap();
    let token = c.register(name, "password1").await.unwrap().token;
    (c.with_token(&token), token)
}

async fn device(url: &str, user: &str, token: &str, vault_id: &str, password: Option<&str>) -> (tempfile::TempDir, Vault, SyncEngine) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let e = SyncEngine::connect(v.clone(), url, user, token, vault_id, password).await.unwrap();
    (dir, v, e)
}

#[tokio::test]
async fn shared_server_vault_api_and_mcp() {
    let data = tempfile::tempdir().unwrap();
    let url = start(config(data.path())).await;
    let (alice, alice_token) = account(&url, "alice").await;
    let (bob, bob_token) = account(&url, "bob").await;
    let (carol, carol_token) = account(&url, "carol").await;

    let family = SyncEngine::create_remote_vault(&alice, "Семья", VaultMode::Server, None, KdfParams::INSECURE_TEST).await.unwrap();
    assert_eq!(family.mode, VaultMode::Server);
    let (_a, va, mut a) = device(&url, "alice", &alice_token, &family.id, None).await;
    va.write("Покупки.md", "# Покупки\n\n## Задачи\n- [ ] Молоко\n".as_bytes()).unwrap();
    assert_eq!(a.sync().await.unwrap().pushed, 1);

    // Only the owner shares; members see the vault with their role.
    assert!(matches!(bob.get_vault(&family.id).await, Err(SyncError::Server { status: 404, .. })));
    assert!(matches!(bob.add_member(&family.id, "carol", Role::Viewer).await, Err(SyncError::Server { status: 404, .. })));
    alice.add_member(&family.id, "bob", Role::Editor).await.unwrap();
    let members = alice.add_member(&family.id, "carol", Role::Viewer).await.unwrap();
    assert_eq!(members.iter().map(|m| m.username.as_str()).collect::<Vec<_>>(), ["alice", "bob", "carol"]);
    assert!(alice.add_member(&family.id, "nobody", Role::Editor).await.is_err());
    let listed = bob.list_vaults().await.unwrap();
    assert_eq!((listed[0].role, listed[0].owner.as_str()), (Role::Editor, "alice"));

    // An editor syncs both ways; a viewer only receives.
    let (_b, vb, mut b) = device(&url, "bob", &bob_token, &family.id, None).await;
    b.sync().await.unwrap();
    assert!(vb.read_string("Покупки.md").unwrap().contains("Молоко"));
    vb.write("Покупки.md", "# Покупки\n\n## Задачи\n- [ ] Молоко\n- [ ] Хлеб\n".as_bytes()).unwrap();
    assert_eq!(b.sync().await.unwrap().pushed, 1);
    let (_c, vc, mut c) = device(&url, "carol", &carol_token, &family.id, None).await;
    c.sync().await.unwrap();
    vc.write("Мои.md", b"local only").unwrap();
    assert_eq!(c.sync().await.unwrap().pushed, 0);
    a.sync().await.unwrap();
    assert!(va.read_string("Покупки.md").unwrap().contains("Хлеб"));
    assert!(!va.exists("Мои.md"));

    // Notes API with a named token.
    let http = reqwest::Client::new();
    let api_token = alice.create_token("скрипты").await.unwrap();
    assert_eq!(alice.tokens().await.unwrap().len(), 1);
    let api = |method: reqwest::Method, path: &str| {
        http.request(method, format!("{url}/api/vaults/{}{path}", family.id)).bearer_auth(&api_token.token)
    };
    let notes: Value = api(reqwest::Method::GET, "/notes").send().await.unwrap().json().await.unwrap();
    assert_eq!(notes[0]["path"], "Покупки.md");
    let note: Value = api(reqwest::Method::GET, "/notes/Покупки.md").send().await.unwrap().json().await.unwrap();
    assert!(note["content"].as_str().unwrap().contains("Хлеб"));
    let stale = api(reqwest::Method::PUT, "/notes/Покупки.md").json(&json!({ "content": "x", "base_seq": 1 })).send().await.unwrap();
    assert_eq!(stale.status(), 409);
    let put = api(reqwest::Method::PUT, "/notes/Идеи/Отпуск.md").json(&json!({ "content": "Море\n" })).send().await.unwrap();
    assert!(put.status().is_success());
    let done: Value = api(reqwest::Method::POST, "/tasks/complete")
        .json(&json!({ "path": "Покупки", "text": "молоко" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(done["done"], true, "{done}");
    let open: Value = api(reqwest::Method::GET, "/tasks").send().await.unwrap().json().await.unwrap();
    assert_eq!(open["tasks"].as_array().unwrap().len(), 1, "{open}");
    // The apps pull API writes like any other change.
    b.sync().await.unwrap();
    assert!(vb.read_string("Покупки.md").unwrap().contains("- [x] Молоко ✅ "));
    assert_eq!(vb.read_string("Идеи/Отпуск.md").unwrap(), "Море\n");

    // Remote MCP: bearer header or token in the URL.
    let rpc = |body: Value| http.post(format!("{url}/mcp")).bearer_auth(&bob_token).json(&body).send();
    let init: Value = rpc(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })).await.unwrap().json().await.unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "bedrock");
    let tools: Value = rpc(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await.unwrap().json().await.unwrap();
    assert_eq!(tools["result"]["tools"][0]["name"], "list_vaults");
    let added: Value = http
        .post(format!("{url}/mcp/{bob_token}"))
        .json(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "add_task", "arguments": { "text": "Сыр", "path": "Покупки" } } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(added["result"]["isError"], false, "{added}");
    let note = http.post(format!("{url}/mcp")).json(&json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" })).send().await.unwrap();
    assert_eq!(note.status(), 401);
    // Clients send odd headers: empty, padded, quoted, or the whole connector URL.
    let ping = json!({ "jsonrpc": "2.0", "id": 6, "method": "ping" });
    let with_header = |value: String| http.post(format!("{url}/mcp")).header("Authorization", value).json(&ping).send();
    let empty = with_header("Bearer ".into()).await.unwrap();
    assert_eq!(empty.status(), 401);
    assert!(empty.text().await.unwrap().contains("нужен API-токен"));
    let wrong = with_header("Bearer nope".into()).await.unwrap();
    assert!(wrong.text().await.unwrap().contains("не подошёл"));
    for value in [format!("bearer   \"{bob_token}\" "), format!("Bearer {url}/mcp/{bob_token}")] {
        assert_eq!(with_header(value.clone()).await.unwrap().status(), 200, "{value}");
    }
    a.sync().await.unwrap();
    assert!(va.read_string("Покупки.md").unwrap().contains("- [ ] Сыр"));
    let viewer_write: Value = http
        .post(format!("{url}/mcp"))
        .bearer_auth(&carol_token)
        .json(&json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "add_task", "arguments": { "text": "Нельзя" } } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(viewer_write["result"]["isError"], true);

    // End-to-end vaults stay closed to the API; removed members lose access.
    let secret = SyncEngine::create_remote_vault(&alice, "Личное", VaultMode::E2e, Some("pw"), KdfParams::INSECURE_TEST).await.unwrap();
    let closed = http.get(format!("{url}/api/vaults/{}/notes", secret.id)).bearer_auth(&alice_token).send().await.unwrap();
    assert_eq!(closed.status(), 400);
    alice.remove_member(&family.id, "bob").await.unwrap();
    assert!(matches!(bob.get_vault(&family.id).await, Err(SyncError::Server { status: 404, .. })));
    carol.remove_member(&family.id, "carol").await.unwrap();
    assert!(carol.list_vaults().await.unwrap().is_empty());
}
