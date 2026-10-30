//! Local sync state, kept in `<vault>/.bedrock/sync/` (never synced itself).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bedrock_proto::VaultMode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Result;

/// Connection settings. Contains the auth token and the vault key, so it lives
/// next to the (unencrypted) notes it protects and nowhere else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub server: String,
    pub username: String,
    pub token: String,
    pub vault_id: String,
    pub vault_name: String,
    #[serde(default)]
    pub mode: VaultMode,
    /// Viewers of a shared vault only pull.
    #[serde(default)]
    pub read_only: bool,
    /// E2e vaults: hand reminder texts to the server so its Telegram bot can send them.
    #[serde(default)]
    pub share_reminders: bool,
    /// Base64 vault key (empty for `server` vaults).
    pub key: String,
    /// Name used in conflict copies ("Note (conflict laptop ...).md").
    pub device: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// Every change with seq <= last_seq has been applied.
    pub last_seq: i64,
    /// Last synced version of each file.
    pub files: BTreeMap<String, FileState>,
    pub last_sync: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    pub file_id: String,
    /// Server seq of the synced version.
    pub seq: i64,
    /// blake3 of the synced content.
    pub hash: String,
    /// Size and mtime of the local file when it matched `hash`; lets a sync
    /// skip hashing untouched files. `mtime_ns == -1` forces a re-check.
    pub size: u64,
    pub mtime_ns: i64,
}

pub(crate) struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new(meta_dir: &Path) -> Self {
        Self { dir: meta_dir.join("sync") }
    }
    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    /// Copy of the last synced content of a text file (merge base).
    pub fn base(&self, file_id: &str) -> PathBuf {
        self.dir.join("base").join(file_id)
    }
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().expect("state files live in a directory");
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(data)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
