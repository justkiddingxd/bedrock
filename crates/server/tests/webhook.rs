//! Reminder webhooks against a fake receiver: the settings API, the test
//! event, signatures, retries with a fresh timestamp and the same id, and how
//! the receiver's answers are treated.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use bedrock_server::{Config, Registration};
use bedrock_sync::proto::{MeSettings, VaultMode};
use bedrock_sync::{ApiClient, KdfParams, SyncEngine};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SECRET: &str = "hermes-shared-secret-0123456789abcdef";

#[derive(Clone, Copy)]
enum Answer {
    Status(u16),
    /// Answers later than the server waits.
    Hang,
}

#[derive(Default)]
struct Receiver {
    script: VecDeque<Answer>,
    got: Vec<(HeaderMap, Bytes)>,
}

type Shared = Arc<Mutex<Receiver>>;

async fn receive(State(r): State<Shared>, headers: HeaderMap, body: Bytes) -> (StatusCode, &'static str) {
    let answer = {
        let mut r = r.lock().unwrap();
        r.got.push((headers, body));
        r.script.pop_front().unwrap_or(Answer::Status(200))
    };
    match answer {
        Answer::Status(s) if s < 300 => (StatusCode::from_u16(s).unwrap(), r#"{"status":"ok"}"#),
        Answer::Status(s) => (StatusCode::from_u16(s).unwrap(), "nope"),
        Answer::Hang => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            (StatusCode::OK, "late")
        }
    }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// Checks the signature like the receiver does and returns (event id, timestamp, body).
fn verify(headers: &HeaderMap, body: &[u8]) -> (String, i64, Value) {
    let ts = headers["x-webhook-timestamp"].to_str().unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{ts}.").as_bytes());
    mac.update(body);
    assert_eq!(headers["x-webhook-signature-v2"].to_str().unwrap(), hex::encode(mac.finalize().into_bytes()));
    assert_eq!(headers["content-type"], "application/json");
    let ts: i64 = ts.parse().unwrap();
    assert!((ts - chrono::Utc::now().timestamp()).abs() < 300, "timestamp {ts} is off");
    let id = headers["x-request-id"].to_str().unwrap().to_string();
    let event: Value = serde_json::from_slice(body).unwrap();
    assert_eq!(event["id"], id.as_str());
    assert_eq!(event["event_type"], "reminder");
    (id, ts, event)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn reminder_webhook() {
    let receiver = Shared::default();
    let hook = serve(Router::new().route("/hook", post(receive)).route("/hook2", post(receive)).with_state(receiver.clone())).await;
    let got = |r: &Shared| r.lock().unwrap().got.clone();
    let reset = |r: &Shared, script: &[Answer]| {
        let mut r = r.lock().unwrap();
        r.got.clear();
        r.script = script.iter().copied().collect();
    };

    let data = tempfile::tempdir().unwrap();
    let mut cfg = Config::new("127.0.0.1:0".parse().unwrap(), data.path().to_path_buf(), Registration::Open);
    cfg.reminder_tick_secs = 1;
    cfg.webhook_allow_hosts = vec!["127.0.0.1".into()];
    cfg.webhook_retry_secs = vec![1, 1, 1];
    cfg.webhook_timeout_secs = 1;
    let url = serve(bedrock_server::build_app(cfg).await.unwrap()).await;

    let anon = ApiClient::new(&url).unwrap();
    let token = anon.register("alice", "password1").await.unwrap().token;
    let alice = anon.with_token(&token);
    alice.set_me(&MeSettings { tz_offset_min: 180 }).await.unwrap();
    let http = reqwest::Client::new();
    let call = |method: reqwest::Method, path: &str| http.request(method, format!("{url}{path}")).bearer_auth(&token);
    let settings = || async {
        let resp = call(reqwest::Method::GET, "/api/reminder-webhook").send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let text = resp.text().await.unwrap();
        assert!(!text.contains(SECRET) && !text.contains("secret"), "{text}");
        serde_json::from_str::<Value>(&text).unwrap()
    };

    // Settings: refused addresses and secrets, then a good one; the secret never comes back.
    assert_eq!(settings().await["configured"], false);
    for (body, why) in [
        (json!({ "url": "ftp://example.com/hook", "secret": SECRET }), "http"),
        (json!({ "url": format!("{hook}/hook"), "secret": "short" }), "32"),
        (json!({ "url": format!("{hook}/hook") }), "секрет"),
        (json!({ "url": format!("{}/hook", hook.replace("127.0.0.1", "localhost")), "secret": SECRET }), "частную сеть"),
        (json!({ "url": "http://10.0.0.1/hook", "secret": SECRET }), "частную сеть"),
    ] {
        let resp = call(reqwest::Method::PUT, "/api/reminder-webhook").json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 400, "{body}");
        assert!(resp.text().await.unwrap().contains(why), "{body}");
    }
    let put = call(reqwest::Method::PUT, "/api/reminder-webhook")
        .json(&json!({ "url": format!("{hook}/hook"), "secret": format!("  {SECRET} ") }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200);
    assert!(!put.text().await.unwrap().contains(SECRET));
    let s = settings().await;
    assert_eq!((s["configured"].as_bool(), s["url"].as_str()), (Some(true), Some(format!("{hook}/hook").as_str())));

    // The test event: same format, "test": true, a new id every time.
    let mut test_ids = Vec::new();
    for _ in 0..2 {
        let res: Value = call(reqwest::Method::POST, "/api/reminder-webhook/test").send().await.unwrap().json().await.unwrap();
        assert_eq!((res["delivered"].as_bool(), res["status"].as_u64()), (Some(true), Some(200)), "{res}");
        assert!(res["body"].as_str().unwrap().contains("ok"));
        let (headers, body) = got(&receiver).pop().unwrap();
        let (id, _, event) = verify(&headers, &body);
        assert_eq!(event["test"], true);
        assert_eq!(event["task"]["text"], "Тестовое напоминание из Bedrock");
        assert_eq!(event["task"]["path"], "Тест.md");
        test_ids.push(id);
    }
    assert_ne!(test_ids[0], test_ids[1]);
    assert!(settings().await["last_status"].as_str().unwrap().starts_with("проверка: доставлено"));

    // A new URL without a secret keeps the saved secret.
    call(reqwest::Method::PUT, "/api/reminder-webhook").json(&json!({ "url": format!("{hook}/hook2") })).send().await.unwrap();
    let res: Value = call(reqwest::Method::POST, "/api/reminder-webhook/test").send().await.unwrap().json().await.unwrap();
    assert_eq!(res["delivered"], true);
    let (headers, body) = got(&receiver).pop().unwrap();
    verify(&headers, &body);

    // A reminder that comes due: the receiver fails once, the retry has a new
    // timestamp and signature, the same id and the same bytes.
    let vault = SyncEngine::create_remote_vault(&alice, "main", VaultMode::Server, None, KdfParams::INSECURE_TEST).await.unwrap();
    let local = chrono::Utc::now() + chrono::Duration::minutes(180);
    let at = local.format("%Y-%m-%d %H:%M").to_string();
    let at_utc = (local - chrono::Duration::minutes(180)).format("%Y-%m-%dT%H:%M:00Z").to_string();
    let write = |path: &str, content: String| {
        call(reqwest::Method::PUT, &format!("/api/vaults/{}/notes/{path}", vault.id)).json(&json!({ "content": content })).send()
    };
    reset(&receiver, &[Answer::Status(503)]);
    let path = "Дневник/2026-10-31.md";
    let put = write(path, format!("# День\n\n- [ ] Позвонить маме 📅 2026-10-31 ⏫ ⏰ {at}\n")).await.unwrap();
    assert!(put.status().is_success());
    until("the retry", || got(&receiver).len() >= 2).await;
    let attempts = got(&receiver);
    let (id1, ts1, event) = verify(&attempts[0].0, &attempts[0].1);
    let (id2, ts2, _) = verify(&attempts[1].0, &attempts[1].1);
    let expected_id = hex::encode(Sha256::digest(format!("{}|{path}|Позвонить маме|{at}", vault.id).as_bytes()));
    assert_eq!((id1.as_str(), id2.as_str()), (expected_id.as_str(), expected_id.as_str()));
    assert!(ts2 > ts1, "{ts1} {ts2}");
    assert_eq!(attempts[0].1, attempts[1].1);
    assert_eq!(
        event,
        json!({
            "event_type": "reminder", "id": expected_id, "test": false, "at": at, "at_utc": at_utc,
            "vault": { "id": vault.id, "name": "main" },
            "task": { "text": "Позвонить маме", "path": path, "line": 2, "due": "2026-10-31", "priority": "high" },
        })
    );
    let start = Instant::now();
    while !settings().await["last_status"].as_str().unwrap_or_default().starts_with("доставлено") {
        assert!(start.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(settings().await["last_delivery_at"].as_i64().is_some());
    // Delivered means done: nothing more is sent.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(got(&receiver).len(), 2);

    // A permanent error is not repeated.
    reset(&receiver, &[Answer::Status(401)]);
    write("Планы.md", format!("Купить билеты ⏰ {at}\n")).await.unwrap();
    until("the rejected delivery", || !got(&receiver).is_empty()).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let attempts = got(&receiver);
    assert_eq!(attempts.len(), 1);
    let (_, _, event) = verify(&attempts[0].0, &attempts[0].1);
    assert_eq!(event["task"], json!({ "text": "Купить билеты", "path": "Планы.md", "line": 0, "due": null, "priority": null }));
    let status = settings().await["last_status"].as_str().unwrap().to_string();
    assert!(status.contains("401") && status.contains("повторов не будет"), "{status}");

    // A timeout and server errors are retried three times, then given up.
    reset(&receiver, &[Answer::Hang, Answer::Status(503), Answer::Status(502), Answer::Status(500)]);
    write("Дом.md", format!("- [ ] Полить цветы ⏰ {at}\n")).await.unwrap();
    until("four attempts", || got(&receiver).len() >= 4).await;
    let start = Instant::now();
    while !settings().await["last_status"].as_str().unwrap_or_default().starts_with("не доставлено") {
        assert!(start.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let attempts = got(&receiver);
    assert_eq!(attempts.len(), 4);
    let ids: Vec<String> = attempts.iter().map(|(h, b)| verify(h, b).0).collect();
    assert!(ids.iter().all(|id| *id == ids[0]));
    assert!(settings().await["last_status"].as_str().unwrap().contains("попыток: 4"));

    // Turned off: nothing configured, nothing sent.
    let off: Value = call(reqwest::Method::DELETE, "/api/reminder-webhook").send().await.unwrap().json().await.unwrap();
    assert_eq!((off["configured"].as_bool(), off["url"].as_str()), (Some(false), None));
    let test = call(reqwest::Method::POST, "/api/reminder-webhook/test").send().await.unwrap();
    assert_eq!(test.status(), 400);
}
