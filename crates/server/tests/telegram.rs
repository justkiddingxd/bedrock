//! Telegram against a fake Bot API: the server's bot, bots users bring
//! themselves, reminders and edit digests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use bedrock_core::Vault;
use bedrock_server::{Config, Registration};
use bedrock_sync::proto::{MeSettings, ReminderItem, Role, VaultMode};
use bedrock_sync::{ApiClient, KdfParams, SyncEngine};
use serde_json::{json, Value};

/// A stand-in for the Bot API that serves any number of bots. Tokens
/// starting with "000:" are rejected like a revoked token.
#[derive(Default)]
struct FakeTelegram {
    updates: HashMap<String, Vec<Value>>,
    /// (bot token, chat, text)
    sent: Vec<(String, i64, String)>,
    next_id: i64,
}

type Shared = Arc<Mutex<FakeTelegram>>;

const SERVER_BOT: &str = "123:abc";
const BOB_BOT: &str = "777:bob-own-bot-token-abcdefghijklmnop";

async fn fake_method(State(t): State<Shared>, Path((bot, method)): Path<(String, String)>, body: Json<Value>) -> Json<Value> {
    let token = bot.trim_start_matches("bot").to_string();
    if token.starts_with("000:") {
        return Json(json!({ "ok": false, "description": "Unauthorized" }));
    }
    let result = match method.as_str() {
        "getMe" => {
            let name = if token == SERVER_BOT {
                "bedrock_test_bot".to_string()
            } else {
                format!("own_{}_bot", token.split(':').next().unwrap())
            };
            json!({ "id": 1, "is_bot": true, "username": name })
        }
        "getUpdates" => {
            let pending = std::mem::take(t.lock().unwrap().updates.entry(token).or_default());
            if pending.is_empty() {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Value::Array(pending)
        }
        "sendMessage" => {
            let text = body["text"].as_str().unwrap().to_string();
            t.lock().unwrap().sent.push((token, body["chat_id"].as_i64().unwrap(), text));
            json!({ "message_id": 1 })
        }
        _ => json!(null),
    };
    Json(json!({ "ok": true, "result": result }))
}

async fn fake_telegram() -> (Shared, String) {
    let fake: Shared = Arc::default();
    let app = Router::new().route("/{bot}/{method}", post(fake_method)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, url)
}

/// Someone writes `text` to the bot with `token` in chat `chat`.
fn say(t: &Shared, token: &str, chat: i64, text: &str) {
    let mut g = t.lock().unwrap();
    g.next_id += 1;
    let id = g.next_id;
    let update = json!({ "update_id": id, "message": { "chat": { "id": chat }, "text": text } });
    g.updates.entry(token.to_string()).or_default().push(update);
}

/// Waits for a message to `chat` containing `needle`; returns (bot token, text).
async fn expect(t: &Shared, chat: i64, needle: &str) -> (String, String) {
    for _ in 0..100 {
        if let Some((token, _, text)) = t.lock().unwrap().sent.iter().find(|(_, c, m)| *c == chat && m.contains(needle)) {
            return (token.clone(), text.clone());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no message to {chat} with {needle:?}; sent: {:?}", t.lock().unwrap().sent);
}

async fn start(tg_url: String, server_bot: Option<&str>) -> (String, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let mut cfg = Config::new("127.0.0.1:0".parse().unwrap(), data.path().to_path_buf(), Registration::Open);
    cfg.telegram_token = server_bot.map(str::to_string);
    cfg.telegram_api = tg_url;
    cfg.reminder_tick_secs = 1;
    cfg.change_delay_secs = 1;
    let app = bedrock_server::build_app(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), data)
}

async fn account(url: &str, name: &str) -> (ApiClient, String) {
    let c = ApiClient::new(url).unwrap();
    let token = c.register(name, "password1").await.unwrap().token;
    (c.with_token(&token), token)
}

async fn device(url: &str, user: &str, token: &str, vault_id: &str) -> (tempfile::TempDir, Vault, SyncEngine) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path()).unwrap();
    let e = SyncEngine::connect(v.clone(), url, user, token, vault_id, None).await.unwrap();
    (dir, v, e)
}

#[tokio::test]
async fn server_bot_own_bots_reminders_and_digests() {
    let (fake, tg_url) = fake_telegram().await;
    let (url, _data) = start(tg_url, Some(SERVER_BOT)).await;
    let (alice, alice_token) = account(&url, "alice").await;
    let (bob, bob_token) = account(&url, "bob").await;

    // The server bot is known once getMe answered.
    for _ in 0..50 {
        if alice.telegram().await.unwrap().available {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(alice.info().await.unwrap().telegram_bot.as_deref(), Some("bedrock_test_bot"));

    // Linking through /start <code>; a bad code is refused.
    say(&fake, SERVER_BOT, 7, "/start nope");
    expect(&fake, 7, "устарела").await;
    let link = alice.telegram_link().await.unwrap();
    assert!(link.url.starts_with("https://t.me/bedrock_test_bot?start="));
    say(&fake, SERVER_BOT, 100, &format!("/start {}", link.code));
    expect(&fake, 100, "«alice» подключён").await;
    assert!(alice.telegram().await.unwrap().linked);

    // Bob brings his own bot; his chat goes through it, not the server's.
    assert!(bob.telegram_set_bot("not a token").await.is_err());
    assert!(bob.telegram_set_bot("000:rejected-by-telegram-abcdefghij").await.is_err());
    let st = bob.telegram_set_bot(BOB_BOT).await.unwrap();
    assert_eq!((st.own_bot.as_deref(), st.bot.as_deref()), (Some("own_777_bot"), Some("own_777_bot")));
    assert!(alice.telegram_set_bot(BOB_BOT).await.is_err(), "one bot, one account");
    let bob_link = bob.telegram_link().await.unwrap();
    assert!(bob_link.url.starts_with("https://t.me/own_777_bot?start="));
    say(&fake, SERVER_BOT, 201, &format!("/start {}", bob_link.code));
    expect(&fake, 201, "для другого бота").await;
    say(&fake, BOB_BOT, 200, &format!("/start@own_777_bot {}", bob_link.code));
    let (via, _) = expect(&fake, 200, "«bob» подключён").await;
    assert_eq!(via, BOB_BOT);

    // A reminder written in a server vault, in the owner's local time.
    alice.set_me(&MeSettings { tz_offset_min: 180 }).await.unwrap();
    let family = SyncEngine::create_remote_vault(&alice, "Семья", VaultMode::Server, None, KdfParams::INSECURE_TEST).await.unwrap();
    let (_d, va, mut a) = device(&url, "alice", &alice_token, &family.id).await;
    let now_local = (chrono::Utc::now() + chrono::Duration::minutes(180)).format("%Y-%m-%d %H:%M");
    let later = (chrono::Utc::now() + chrono::Duration::minutes(180 + 120)).format("%Y-%m-%d %H:%M");
    va.write("Дела.md", format!("- [ ] Позвонить маме ⏰ {now_local}\n- [ ] Потом ⏰ {later}\n").as_bytes()).unwrap();
    a.sync().await.unwrap();
    let (via, msg) = expect(&fake, 100, "Позвонить маме").await;
    assert!(via == SERVER_BOT && msg.starts_with("⏰ ") && msg.contains("Семья"), "{msg}");

    // Sharing tells the new member through their own bot; edits come as digests.
    alice.add_member(&family.id, "bob", Role::Editor).await.unwrap();
    let (via, _) = expect(&fake, 200, "открыл(а) вам доступ к хранилищу «Семья»").await;
    assert_eq!(via, BOB_BOT);
    let (_e, vb, mut b) = device(&url, "bob", &bob_token, &family.id).await;
    b.sync().await.unwrap();
    vb.write("Покупки.md", b"- [ ] milk\n").unwrap();
    b.sync().await.unwrap();
    let (_, digest) = expect(&fake, 100, "✏️ bob").await;
    assert!(digest.contains("«Семья»: Покупки"), "{digest}");
    va.write("План.md", b"x\n").unwrap();
    a.sync().await.unwrap();
    let (via, _) = expect(&fake, 200, "✏️ alice").await;
    assert_eq!(via, BOB_BOT);

    // End-to-end vaults: the app hands over reminder texts.
    let secret = SyncEngine::create_remote_vault(&alice, "Личное", VaultMode::E2e, Some("pw"), KdfParams::INSECURE_TEST).await.unwrap();
    let item = |key: &str, at: i64| ReminderItem { key: key.into(), at, text: "Таблетки".into(), note: "Здоровье".into() };
    let soon = chrono::Utc::now().timestamp() - 5;
    alice.put_reminders(&secret.id, vec![item("k1", soon)]).await.unwrap();
    expect(&fake, 100, "Таблетки").await;
    assert!(alice.put_reminders(&family.id, vec![item("k2", soon)]).await.is_err());

    // /changes and /stop; removing the own bot unlinks its chat.
    say(&fake, SERVER_BOT, 100, "/changes");
    expect(&fake, 100, "выключены").await;
    assert!(!alice.telegram().await.unwrap().notify_changes);
    say(&fake, SERVER_BOT, 100, "/stop");
    expect(&fake, 100, "Чат отключён").await;
    assert!(!alice.telegram().await.unwrap().linked);
    let st = bob.telegram_remove_bot().await.unwrap();
    assert!(st.own_bot.is_none() && !st.linked && st.available, "{st:?}");
}

/// A server without a bot of its own: everyone can still use their own.
#[tokio::test]
async fn own_bot_without_server_bot() {
    let (fake, tg_url) = fake_telegram().await;
    let (url, _data) = start(tg_url, None).await;
    let (carol, _) = account(&url, "carol").await;

    let st = carol.telegram().await.unwrap();
    assert!(!st.available && st.server_bot.is_none());
    assert!(carol.telegram_link().await.is_err());

    let token = "888:carol-bot-token-abcdefghijklmnopq";
    carol.telegram_set_bot(token).await.unwrap();
    let link = carol.telegram_link().await.unwrap();
    say(&fake, token, 500, &format!("/start {}", link.code));
    expect(&fake, 500, "«carol» подключён").await;

    let vault = SyncEngine::create_remote_vault(&carol, "Моё", VaultMode::E2e, Some("pw"), KdfParams::INSECURE_TEST).await.unwrap();
    let at = chrono::Utc::now().timestamp() - 5;
    let item = ReminderItem { key: "x".into(), at, text: "Полить цветы".into(), note: "Дом".into() };
    carol.put_reminders(&vault.id, vec![item]).await.unwrap();
    let (via, msg) = expect(&fake, 500, "Полить цветы").await;
    assert_eq!(via, token, "{msg}");
}
