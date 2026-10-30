//! HTTP API of the Bedrock sync server.
//!
//! The server stores every file of a vault under a `file_id` as a blob, and
//! every write bumps the vault's monotonic `seq`. Clients pull everything with
//! `seq > last_seen` and push with optimistic concurrency: a push names the
//! `base_seq` it was made on top of, and the server rejects it with the
//! current version if someone else got there first.
//!
//! A vault is one of two kinds ([`VaultMode`]):
//!
//! * `e2e`: blobs are encrypted on the client and `file_id` is an HMAC of the
//!   path, so the server sees neither names nor content.
//! * `server`: blobs use the open [`plain`] format, so the server can read
//!   notes and offer the notes API, reminders in Telegram and remote MCP.
//!
//! Binary values (blobs, salts, nonces) are standard base64 strings.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub registration_open: bool,
    pub max_file_mb: u64,
    /// Username of the server's Telegram bot, if it has one.
    #[serde(default)]
    pub telegram_bot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub username: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultMode {
    /// End-to-end encrypted: the server stores only ciphertext.
    #[default]
    E2e,
    /// Readable by the server, which enables the notes API, bot and MCP.
    Server,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Owner,
    Editor,
    Viewer,
}

impl Role {
    pub fn can_write(self) -> bool {
        self != Role::Viewer
    }
}

/// The vault key, encrypted with a key derived from the vault password.
/// The server stores it but can never open it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEnvelope {
    pub kdf: String,
    pub salt: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub nonce: String,
    pub wrapped_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVaultRequest {
    pub name: String,
    #[serde(default)]
    pub mode: VaultMode,
    /// Required for `e2e` vaults, absent for `server` ones.
    #[serde(default)]
    pub key_envelope: Option<KeyEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSummary {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub head_seq: i64,
    #[serde(default)]
    pub mode: VaultMode,
    /// The caller's role in this vault.
    #[serde(default)]
    pub role: Role,
    /// Username of the owner.
    #[serde(default)]
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultDetails {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub head_seq: i64,
    #[serde(default)]
    pub mode: VaultMode,
    #[serde(default)]
    pub role: Role,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub key_envelope: Option<KeyEnvelope>,
}

/// Someone with access to a vault (the owner is listed too).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub username: String,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    pub username: String,
    /// `editor` or `viewer`.
    pub role: Role,
}

/// Latest version of one file. `blob` is `None` for deletions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub file_id: String,
    pub seq: i64,
    pub deleted: bool,
    pub blob: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesResponse {
    pub changes: Vec<Change>,
    pub head_seq: i64,
    /// More changes are available; ask again with `since` = last change's seq.
    pub more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushItem {
    pub file_id: String,
    /// Seq of the version this change was based on (0 for a new file).
    pub base_seq: i64,
    /// Encrypted content, or `None` to delete the file.
    pub blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub items: Vec<PushItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PushResult {
    Ok { file_id: String, seq: i64 },
    Conflict { file_id: String, current: Change },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub results: Vec<PushResult>,
    pub head_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

// ---------------------------------------------------------------- account

/// Per-account settings the client keeps up to date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeSettings {
    /// Local time minus UTC, in minutes: reminders in server vaults are
    /// written in the owner's local time.
    pub tz_offset_min: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramStatus {
    /// There is a bot to link: the user's own or the server's.
    pub available: bool,
    /// The bot a new link goes through (own bot first).
    pub bot: Option<String>,
    pub linked: bool,
    /// Also report edits other people make in shared vaults.
    pub notify_changes: bool,
    /// The user's own bot, if they added one.
    #[serde(default)]
    pub own_bot: Option<String>,
    /// The bot of the server, if it has one.
    #[serde(default)]
    pub server_bot: Option<String>,
    /// The bot the linked chat goes through.
    #[serde(default)]
    pub linked_via: Option<String>,
}

/// A bot the user made with @BotFather; the server runs it for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBotRequest {
    pub token: String,
}

/// Opening `url` in Telegram and pressing Start links the account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramLink {
    pub url: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSettings {
    pub notify_changes: bool,
}

/// A named long-lived token for scripts, the notes API and remote MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
}

/// Returned once, when the token is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedToken {
    pub id: i64,
    pub name: String,
    pub token: String,
}

/// A reminder an app found in an end-to-end encrypted vault and hands to the
/// server so the bot can send it. Only these texts leave the device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderItem {
    /// Stable id chosen by the client (same reminder, same key).
    pub key: String,
    /// Unix time, seconds.
    pub at: i64,
    pub text: String,
    /// Title of the note it is in.
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemindersRequest {
    /// The full set of upcoming reminders; earlier ones are replaced.
    pub items: Vec<ReminderItem>,
}

/// A URL that also gets every reminder of the account's server vaults, as
/// signed JSON (for the user's own agent, bot or automation). The secret is
/// never sent back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderWebhook {
    pub configured: bool,
    pub url: Option<String>,
    /// Outcome of the last delivery attempt, in words.
    pub last_status: Option<String>,
    /// Unix time of the last delivery attempt.
    pub last_delivery_at: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SetWebhookRequest {
    pub url: String,
    /// At least 32 characters. Left out: the saved one stays.
    #[serde(default)]
    pub secret: Option<String>,
}

impl std::fmt::Debug for SetWebhookRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetWebhookRequest").field("url", &self.url).finish_non_exhaustive()
    }
}

/// What the receiver answered to a test event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTestResult {
    pub delivered: bool,
    /// HTTP status of the answer; `None` when the receiver was not reached.
    pub status: Option<u16>,
    /// The start of the answer's body.
    pub body: String,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- notes API (server vaults)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub path: String,
    pub size: i64,
    pub seq: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteContent {
    pub path: String,
    pub content: String,
    pub seq: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteNoteRequest {
    pub content: String,
    /// When set, the write fails with 409 if the note changed since this seq.
    #[serde(default)]
    pub base_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResult {
    pub path: String,
    pub seq: i64,
}

/// Open blob format of `server` vaults: `0x00 || path_len(u32 LE) || path || content`.
/// The file id is a plain SHA-256 of the path, so the server can check it.
pub mod plain {
    use sha2::{Digest, Sha256};

    pub const VERSION: u8 = 0;

    pub fn file_id(path: &str) -> String {
        let mut h = Sha256::new();
        h.update(b"bedrock-plain-v1:");
        h.update(path.as_bytes());
        hex::encode(h.finalize())
    }

    pub fn encode(path: &str, content: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + path.len() + content.len());
        out.push(VERSION);
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(content);
        out
    }

    /// Returns `(path, content)` of a well-formed blob.
    pub fn decode(blob: &[u8]) -> Option<(String, Vec<u8>)> {
        if blob.len() < 5 || blob[0] != VERSION {
            return None;
        }
        let len = u32::from_le_bytes(blob[1..5].try_into().ok()?) as usize;
        let path = std::str::from_utf8(blob.get(5..5 + len)?).ok()?.to_string();
        Some((path, blob[5 + len..].to_vec()))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn roundtrip() {
            let blob = super::encode("Папка/Заметка.md", "текст".as_bytes());
            let (path, content) = super::decode(&blob).unwrap();
            assert_eq!(path, "Папка/Заметка.md");
            assert_eq!(content, "текст".as_bytes());
            assert_eq!(super::file_id(&path).len(), 64);
            assert!(super::decode(&blob[..3]).is_none());
        }
    }
}
