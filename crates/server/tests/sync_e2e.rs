//! End-to-end: a real server on a random port and two "devices" (vault
//! folders) syncing through it.

use std::path::Path;

use bedrock_core::Vault;
use bedrock_server::{Config, Registration};
use bedrock_sync::proto::VaultMode;
use bedrock_sync::{ApiClient, KdfParams, SyncEngine, SyncError};

async fn start_server(data: &Path, registration: Registration) -> String {
    let mut cfg = Config::new("127.0.0.1:0".parse().unwrap(), data.to_path_buf(), registration);
    cfg.max_file_mb = 16;
    let app = bedrock_server::build_app(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

const NOTE: &str = "# Hello\n\nline one\nline two\nline three\n";

struct Target {
    url: String,
    token: String,
    vault_id: String,
}

impl Target {
    async fn connect(&self, vault: &Vault, password: &str) -> Result<SyncEngine, SyncError> {
        SyncEngine::connect(vault.clone(), &self.url, "alice", &self.token, &self.vault_id, Some(password)).await
    }
}

#[tokio::test]
async fn two_devices_sync() {
    let data = tempfile::tempdir().unwrap();
    let url = start_server(data.path(), Registration::Open).await;

    let client = ApiClient::new(&url).unwrap();
    let auth = client.register("alice", "correct horse").await.unwrap();
    let client = client.with_token(&auth.token);
    let remote = SyncEngine::create_remote_vault(&client, "Notes", VaultMode::E2e, Some("vault-pass"), KdfParams::INSECURE_TEST)
        .await
        .unwrap();

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let va = Vault::open(dir_a.path()).unwrap();
    let vb = Vault::open(dir_b.path()).unwrap();
    va.write("Hello.md", NOTE.as_bytes()).unwrap();
    va.write("Folder/Tasks.md", "- [ ] buy milk\n".as_bytes()).unwrap();
    va.write("img.bin", &[0u8, 1, 2, 3, 255]).unwrap();

    let target = Target { url: url.clone(), token: auth.token.clone(), vault_id: remote.id.clone() };
    let mut a = target.connect(&va, "vault-pass").await.unwrap();
    let mut b = target.connect(&vb, "vault-pass").await.unwrap();

    // Wrong vault password is rejected before anything is written.
    let other = tempfile::tempdir().unwrap();
    let wrong = target.connect(&Vault::open(other.path()).unwrap(), "nope").await;
    assert!(matches!(wrong, Err(SyncError::WrongPassword)), "{:?}", wrong.err());

    // Initial upload and download.
    let r = a.sync().await.unwrap();
    assert_eq!(r.pushed, 3, "{r:?}");
    let r = b.sync().await.unwrap();
    assert_eq!(r.pulled, 3, "{r:?}");
    assert_eq!(vb.read_string("Hello.md").unwrap(), NOTE);
    assert_eq!(vb.read_bytes("img.bin").unwrap(), vec![0u8, 1, 2, 3, 255]);

    // Nothing to do on a second run.
    let r = a.sync().await.unwrap();
    assert_eq!((r.pulled, r.pushed), (0, 0), "{r:?}");

    // Concurrent edits to different lines merge cleanly.
    va.write("Hello.md", "# Hello\n\nline one (A)\nline two\nline three\n".as_bytes()).unwrap();
    vb.write("Hello.md", "# Hello\n\nline one\nline two\nline three (B)\n".as_bytes()).unwrap();
    a.sync().await.unwrap();
    let r = b.sync().await.unwrap();
    assert_eq!(r.merged, 1, "{r:?}");
    a.sync().await.unwrap();
    let merged = "# Hello\n\nline one (A)\nline two\nline three (B)\n";
    assert_eq!(va.read_string("Hello.md").unwrap(), merged);
    assert_eq!(vb.read_string("Hello.md").unwrap(), merged);

    // Edits to the same line: B keeps A's version and gets a conflict copy.
    va.write("Hello.md", merged.replace("line two", "line two by A").as_bytes()).unwrap();
    vb.write("Hello.md", merged.replace("line two", "line two by B!").as_bytes()).unwrap();
    a.sync().await.unwrap();
    let r = b.sync().await.unwrap();
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    let copy = r.conflicts[0].clone();
    assert!(copy.starts_with("Hello (conflict "), "{copy}");
    a.sync().await.unwrap();
    assert_eq!(va.read_string("Hello.md").unwrap(), vb.read_string("Hello.md").unwrap());
    assert!(va.read_string("Hello.md").unwrap().contains("line two by A"));
    assert!(va.read_string(&copy).unwrap().contains("line two by B!"));

    // Deletions propagate (into .trash, not gone for good).
    va.trash("Folder/Tasks.md").unwrap();
    let r = a.sync().await.unwrap();
    assert_eq!(r.deleted_remote, 1, "{r:?}");
    let r = b.sync().await.unwrap();
    assert_eq!(r.deleted_local, 1, "{r:?}");
    assert!(!vb.exists("Folder/Tasks.md"));
    assert!(dir_b.path().join(".trash/Folder/Tasks.md").exists());

    // A device that reconnects from scratch gets everything.
    let dir_c = tempfile::tempdir().unwrap();
    let vc = Vault::open(dir_c.path()).unwrap();
    let mut c = target.connect(&vc, "vault-pass").await.unwrap();
    c.sync().await.unwrap();
    assert_eq!(vc.read_string("Hello.md").unwrap(), va.read_string("Hello.md").unwrap());
    assert!(!vc.exists("Folder/Tasks.md"));

    // State survives a restart of the engine.
    drop(a);
    let mut a = SyncEngine::load(va.clone()).unwrap().expect("config saved");
    let r = a.sync().await.unwrap();
    assert_eq!((r.pulled, r.pushed), (0, 0), "{r:?}");

    // The server only ever stored ciphertext.
    for name in ["bedrock.db", "bedrock.db-wal"] {
        if let Ok(bytes) = std::fs::read(data.path().join(name)) {
            let haystack = String::from_utf8_lossy(&bytes);
            assert!(!haystack.contains("line two"), "plaintext leaked into {name}");
            assert!(!haystack.contains("Hello.md"), "file name leaked into {name}");
        }
    }
}

#[tokio::test]
async fn registration_modes_and_isolation() {
    let data = tempfile::tempdir().unwrap();
    let url = start_server(data.path(), Registration::FirstOnly).await;
    let client = ApiClient::new(&url).unwrap();

    assert!(client.info().await.unwrap().registration_open);
    let owner = client.register("owner", "password1").await.unwrap();
    assert!(!client.info().await.unwrap().registration_open);
    assert!(matches!(
        client.register("intruder", "password1").await,
        Err(SyncError::Server { status: 403, .. })
    ));
    assert!(matches!(client.login("owner", "wrong-pass").await, Err(SyncError::Unauthorized(_))));
    let again = client.login("owner", "password1").await.unwrap();
    assert_ne!(again.token, owner.token);

    // Vaults are private to their owner.
    let data2 = tempfile::tempdir().unwrap();
    let url2 = start_server(data2.path(), Registration::Open).await;
    let c = ApiClient::new(&url2).unwrap();
    let alice = c.clone().with_token(c.register("alice", "password1").await.unwrap().token);
    let bob = c.clone().with_token(c.register("bob", "password1").await.unwrap().token);
    let v = SyncEngine::create_remote_vault(&alice, "Private", VaultMode::E2e, Some("pw"), KdfParams::INSECURE_TEST).await.unwrap();
    assert!(matches!(bob.get_vault(&v.id).await, Err(SyncError::Server { status: 404, .. })));
    assert!(bob.list_vaults().await.unwrap().is_empty());
    assert_eq!(alice.list_vaults().await.unwrap().len(), 1);
}
