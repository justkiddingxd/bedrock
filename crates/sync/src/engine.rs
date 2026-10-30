//! The sync algorithm.
//!
//! 1. Pull every change with `seq > last_seq` and apply it:
//!    * untouched locally -> take the remote version;
//!    * changed on both sides -> three-way merge against the last synced
//!      version (the "base"); if lines collide, keep the remote version and
//!      save ours as a conflict copy, so nothing is ever lost.
//! 2. Push local changes with optimistic concurrency. If the server reports
//!    a conflict, apply its version as in step 1 and push the result again.
//!
//! Remote deletions move files to `.trash/`; an edit always wins over a
//! concurrent delete.

use std::collections::{BTreeMap, HashMap};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use bedrock_core::merge::{self, MergeOutcome};
use bedrock_core::vault::{self, Vault};
use bedrock_proto::{Change, PushItem, PushResult, VaultMode, VaultSummary};
use serde::Serialize;

use crate::crypto::{self, Cipher, KdfParams, VaultKey};
use crate::state::{self, FileState, Paths, SyncConfig, SyncState};
use crate::{ApiClient, Result, SyncError};

const PAGE: i64 = 500;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_BATCH_ITEMS: usize = 200;
/// Text files up to this size keep a merge base and get three-way merges.
const MAX_MERGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PUSH_ROUNDS: usize = 5;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pulled: usize,
    pub pushed: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    pub merged: usize,
    /// Conflict copies created in this sync.
    pub conflicts: Vec<String>,
    /// Per-file problems that did not stop the sync.
    pub errors: Vec<String>,
    pub head_seq: i64,
}

pub struct SyncEngine {
    vault: Vault,
    paths: Paths,
    cfg: SyncConfig,
    key: Cipher,
    client: ApiClient,
    state: SyncState,
    /// file_id -> path, mirrors `state.files`.
    ids: HashMap<String, String>,
    max_blob: Option<usize>,
    /// The role was re-read from the server in this session.
    role_checked: bool,
}

/// A local change waiting for the server's answer.
struct Pending {
    path: String,
    file_id: String,
    deleted: bool,
    hash: String,
    size: u64,
    mtime_ns: i64,
    /// Content to keep as merge base (text files only).
    base: Option<Vec<u8>>,
}

impl SyncEngine {
    /// Loads the sync setup of a vault, if it has one.
    pub fn load(vault: Vault) -> Result<Option<Self>> {
        let paths = Paths::new(&vault.meta_dir());
        let Some(cfg) = state::read_json::<SyncConfig>(&paths.config())? else {
            return Ok(None);
        };
        let state: SyncState = state::read_json(&paths.state())?.unwrap_or_default();
        let key = match cfg.mode {
            VaultMode::E2e => Cipher::E2e(VaultKey::from_base64(&cfg.key)?),
            VaultMode::Server => Cipher::Plain,
        };
        let client = ApiClient::new(&cfg.server)?.with_token(&cfg.token);
        let ids = state.files.iter().map(|(p, f)| (f.file_id.clone(), p.clone())).collect();
        Ok(Some(Self { vault, paths, cfg, key, client, state, ids, max_blob: None, role_checked: false }))
    }

    /// Creates an empty remote vault. `e2e` vaults are protected by `password`.
    pub async fn create_remote_vault(
        client: &ApiClient,
        name: &str,
        mode: VaultMode,
        password: Option<&str>,
        params: KdfParams,
    ) -> Result<VaultSummary> {
        let envelope = match mode {
            VaultMode::Server => None,
            VaultMode::E2e => {
                let password = password.filter(|p| !p.is_empty()).ok_or(SyncError::Other("нужен пароль хранилища".into()))?;
                let password = password.to_string();
                let (envelope, _) = tokio::task::spawn_blocking(move || crypto::create_envelope(&password, params))
                    .await
                    .map_err(|e| SyncError::Other(e.to_string()))??;
                Some(envelope)
            }
        };
        client.create_vault(name, mode, envelope).await
    }

    /// Links a local vault to a remote one. For `e2e` vaults, fails with
    /// `WrongPassword` if the password does not open the vault key. Any
    /// previous sync state of the local vault is discarded.
    pub async fn connect(
        vault: Vault,
        server: &str,
        username: &str,
        token: &str,
        vault_id: &str,
        password: Option<&str>,
    ) -> Result<Self> {
        let client = ApiClient::new(server)?.with_token(token);
        let details = client.get_vault(vault_id).await?;
        let key = match details.mode {
            VaultMode::Server => Cipher::Plain,
            VaultMode::E2e => {
                let envelope = details.key_envelope.clone().ok_or(SyncError::Other("у хранилища нет ключа".into()))?;
                let password = password.unwrap_or_default().to_string();
                Cipher::E2e(
                    tokio::task::spawn_blocking(move || crypto::open_envelope(&envelope, &password))
                        .await
                        .map_err(|e| SyncError::Other(e.to_string()))??,
                )
            }
        };

        let cfg = SyncConfig {
            server: client.base().to_string(),
            username: username.to_string(),
            token: token.to_string(),
            vault_id: vault_id.to_string(),
            vault_name: details.name,
            mode: details.mode,
            read_only: !details.role.can_write(),
            share_reminders: false,
            key: match &key {
                Cipher::E2e(k) => k.to_base64(),
                Cipher::Plain => String::new(),
            },
            device: device_name(),
        };
        let paths = Paths::new(&vault.meta_dir());
        if paths.dir.exists() {
            std::fs::remove_dir_all(&paths.dir)?;
        }
        state::write_json(&paths.config(), &cfg)?;
        let engine = Self {
            vault,
            paths,
            cfg,
            key,
            client,
            state: SyncState::default(),
            ids: HashMap::new(),
            max_blob: None,
            role_checked: true,
        };
        engine.save_state()?;
        Ok(engine)
    }

    /// Removes the local sync setup. Files stay where they are.
    pub fn disconnect(self) -> Result<()> {
        if self.paths.dir.exists() {
            std::fs::remove_dir_all(&self.paths.dir)?;
        }
        Ok(())
    }

    pub fn config(&self) -> &SyncConfig {
        &self.cfg
    }

    /// Changes and saves the connection settings.
    pub fn update_config(&mut self, f: impl FnOnce(&mut SyncConfig)) -> Result<()> {
        f(&mut self.cfg);
        state::write_json(&self.paths.config(), &self.cfg)
    }

    pub fn client(&self) -> &ApiClient {
        &self.client
    }

    pub fn last_sync(&self) -> Option<i64> {
        self.state.last_sync
    }

    pub fn last_seq(&self) -> i64 {
        self.state.last_seq
    }

    pub async fn sync(&mut self) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        if !self.role_checked {
            // The owner may have changed our role since the last session.
            let details = self.client.get_vault(&self.cfg.vault_id).await?;
            self.role_checked = true;
            let read_only = !details.role.can_write();
            if read_only != self.cfg.read_only || details.name != self.cfg.vault_name {
                self.cfg.read_only = read_only;
                self.cfg.vault_name = details.name;
                state::write_json(&self.paths.config(), &self.cfg)?;
            }
        }
        self.pull(&mut report).await?;
        if !self.cfg.read_only {
            self.push(&mut report).await?;
        }
        self.state.last_sync = Some(chrono::Utc::now().timestamp());
        self.save_state()?;
        report.head_seq = self.state.last_seq;
        Ok(report)
    }

    async fn pull(&mut self, report: &mut SyncReport) -> Result<()> {
        loop {
            let resp = self.client.changes(&self.cfg.vault_id, self.state.last_seq, PAGE).await?;
            for change in &resp.changes {
                self.apply_remote(change, report)?;
            }
            let max_seen = resp.changes.last().map_or(0, |c| c.seq);
            self.state.last_seq = if resp.more { max_seen } else { resp.head_seq.max(max_seen) };
            self.save_state()?;
            if !resp.more {
                return Ok(());
            }
        }
    }

    async fn push(&mut self, report: &mut SyncReport) -> Result<()> {
        if self.max_blob.is_none() {
            self.max_blob = Some(self.client.info().await?.max_file_mb as usize * 1024 * 1024);
        }
        for _ in 0..MAX_PUSH_ROUNDS {
            let pending = self.collect_local_changes(report)?;
            if pending.is_empty() {
                return Ok(());
            }
            let mut conflicts = 0;
            for batch in batches(pending) {
                let (items, metas): (Vec<PushItem>, Vec<Pending>) = batch.into_iter().unzip();
                let resp = self.client.push(&self.cfg.vault_id, items).await?;
                for (result, meta) in resp.results.into_iter().zip(metas) {
                    match result {
                        PushResult::Ok { seq, .. } => self.accept_push(meta, seq, report)?,
                        PushResult::Conflict { current, .. } => {
                            conflicts += 1;
                            self.apply_remote(&current, report)?;
                        }
                    }
                }
                self.save_state()?;
            }
            if conflicts == 0 {
                return Ok(());
            }
        }
        Ok(())
    }

    fn collect_local_changes(&mut self, report: &mut SyncReport) -> Result<Vec<(PushItem, Pending)>> {
        let max_blob = self.max_blob.unwrap_or(usize::MAX);
        let local: BTreeMap<String, vault::Entry> = self
            .vault
            .walk()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| (e.path.clone(), e))
            .collect();

        let mut out = Vec::new();
        for (path, entry) in &local {
            let known = self.state.files.get(path).map(|k| (k.size, k.mtime_ns, k.hash.clone(), k.seq));
            if let Some((size, mtime, _, _)) = &known {
                if *size == entry.size && *mtime == entry.mtime_ns {
                    continue;
                }
            }
            let Some(data) = self.vault.read_opt(path)? else { continue };
            let hash = hash(&data);
            if let Some((_, _, known_hash, _)) = &known {
                if *known_hash == hash {
                    // Touched but unchanged: remember the new mtime.
                    if let Some(k) = self.state.files.get_mut(path) {
                        k.size = entry.size;
                        k.mtime_ns = entry.mtime_ns;
                    }
                    continue;
                }
            }
            let file_id = self.key.file_id(path);
            let blob = self.key.seal(&self.cfg.vault_id, &file_id, path, &data);
            if blob.len() > max_blob {
                report.errors.push(format!("{path}: файл больше лимита сервера"));
                continue;
            }
            let item = PushItem {
                file_id: file_id.clone(),
                base_seq: known.map_or(0, |k| k.3),
                blob: Some(B64.encode(&blob)),
            };
            let base = text_of(&data).is_some().then_some(data);
            out.push((
                item,
                Pending { path: path.clone(), file_id, deleted: false, hash, size: entry.size, mtime_ns: entry.mtime_ns, base },
            ));
        }

        for (path, known) in &self.state.files {
            if !local.contains_key(path) {
                let item = PushItem { file_id: known.file_id.clone(), base_seq: known.seq, blob: None };
                out.push((
                    item,
                    Pending {
                        path: path.clone(),
                        file_id: known.file_id.clone(),
                        deleted: true,
                        hash: String::new(),
                        size: 0,
                        mtime_ns: 0,
                        base: None,
                    },
                ));
            }
        }
        Ok(out)
    }

    fn accept_push(&mut self, p: Pending, seq: i64, report: &mut SyncReport) -> Result<()> {
        if p.deleted {
            self.forget(&p.path)?;
            report.deleted_remote += 1;
        } else {
            self.store_base(&p.file_id, p.base.as_deref())?;
            self.remember(p.path, FileState { file_id: p.file_id, seq, hash: p.hash, size: p.size, mtime_ns: p.mtime_ns });
            report.pushed += 1;
        }
        Ok(())
    }

    fn apply_remote(&mut self, ch: &Change, report: &mut SyncReport) -> Result<()> {
        let known_path = self.ids.get(&ch.file_id).cloned();
        if let Some(p) = &known_path {
            if self.state.files.get(p).is_some_and(|f| f.seq >= ch.seq) {
                return Ok(()); // already have it (e.g. our own push)
            }
        }

        if ch.deleted {
            let Some(path) = known_path else { return Ok(()) };
            let known = self.state.files[&path].clone();
            match self.vault.read_opt(&path)? {
                None => self.forget(&path)?,
                Some(data) if hash(&data) == known.hash => {
                    self.vault.trash(&path)?;
                    self.forget(&path)?;
                    report.deleted_local += 1;
                }
                Some(_) => {
                    // Edited here, deleted there: keep ours and upload it on
                    // top of the deletion.
                    self.remember(path, FileState { seq: ch.seq, hash: String::new(), mtime_ns: -1, ..known });
                }
            }
            return Ok(());
        }

        let Some(blob) = &ch.blob else { return Ok(()) };
        let opened = B64
            .decode(blob)
            .ok()
            .and_then(|b| self.key.open(&self.cfg.vault_id, &ch.file_id, &b).ok());
        let Some((path, remote)) = opened else {
            report.errors.push(format!("не удалось расшифровать файл {}", &ch.file_id[..8.min(ch.file_id.len())]));
            return Ok(());
        };
        match vault::normalize(&path) {
            Ok(p) if p == path && !vault::is_hidden(&p) => {}
            _ => {
                report.errors.push(format!("пропущен недопустимый путь: {path}"));
                return Ok(());
            }
        }

        let remote_hash = hash(&remote);
        let known = self.state.files.get(&path).cloned();
        let mut local_is_remote = true;
        match self.vault.read_opt(&path)? {
            None => {
                self.vault.write(&path, &remote)?;
                report.pulled += 1;
            }
            Some(local) => {
                let local_hash = hash(&local);
                if local_hash == remote_hash {
                    // Same content on both sides already.
                } else if known.as_ref().is_some_and(|k| k.hash == local_hash) {
                    self.vault.write(&path, &remote)?;
                    report.pulled += 1;
                } else {
                    let base = known.as_ref().and_then(|k| self.load_base(&k.file_id)).unwrap_or_default();
                    let merged = match (text_of(&local), text_of(&remote), std::str::from_utf8(&base)) {
                        (Some(ours), Some(theirs), Ok(base)) => match merge::merge3(base, ours, theirs) {
                            MergeOutcome::Clean(m) => Some(m),
                            MergeOutcome::Conflict => None,
                        },
                        _ => None,
                    };
                    match merged {
                        Some(m) => {
                            self.vault.write(&path, m.as_bytes())?;
                            local_is_remote = m.as_bytes() == remote.as_slice();
                            report.merged += 1;
                        }
                        None => {
                            let copy = merge::conflict_copy_path(&self.vault, &path, &self.cfg.device)?;
                            self.vault.write(&copy, &local)?;
                            self.vault.write(&path, &remote)?;
                            report.conflicts.push(copy);
                        }
                    }
                }
            }
        }

        // A merged file differs from the remote version and must be pushed:
        // record an impossible mtime so the next scan re-hashes it.
        let (size, mtime_ns) = if local_is_remote { self.local_meta(&path) } else { (0, -1) };
        self.store_base(&ch.file_id, text_of(&remote).map(str::as_bytes))?;
        self.remember(path, FileState { file_id: ch.file_id.clone(), seq: ch.seq, hash: remote_hash, size, mtime_ns });
        Ok(())
    }

    fn remember(&mut self, path: String, file: FileState) {
        self.ids.insert(file.file_id.clone(), path.clone());
        self.state.files.insert(path, file);
    }

    fn forget(&mut self, path: &str) -> Result<()> {
        if let Some(f) = self.state.files.remove(path) {
            self.ids.remove(&f.file_id);
            self.store_base(&f.file_id, None)?;
        }
        Ok(())
    }

    fn store_base(&self, file_id: &str, content: Option<&[u8]>) -> Result<()> {
        let path = self.paths.base(file_id);
        match content {
            Some(data) => state::write_atomic(&path, data),
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
        }
    }

    fn load_base(&self, file_id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.paths.base(file_id)).ok()
    }

    fn local_meta(&self, path: &str) -> (u64, i64) {
        self.vault
            .abs(path)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.len(), vault::mtime_ns(&m)))
            .unwrap_or((0, -1))
    }

    fn save_state(&self) -> Result<()> {
        state::write_json(&self.paths.state(), &self.state)
    }
}

fn hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// The content as text, if it is small UTF-8 without NUL bytes.
fn text_of(data: &[u8]) -> Option<&str> {
    if data.len() > MAX_MERGE_BYTES || data.contains(&0) {
        return None;
    }
    std::str::from_utf8(data).ok()
}

fn batches(items: Vec<(PushItem, Pending)>) -> Vec<Vec<(PushItem, Pending)>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut bytes = 0;
    for item in items {
        let size = item.0.blob.as_ref().map_or(0, String::len);
        if !current.is_empty() && (bytes + size > MAX_BATCH_BYTES || current.len() >= MAX_BATCH_ITEMS) {
            out.push(std::mem::take(&mut current));
            bytes = 0;
        }
        bytes += size;
        current.push(item);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "device".into())
}
