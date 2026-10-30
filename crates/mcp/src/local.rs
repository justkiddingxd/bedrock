//! A vault folder on this machine. Changes land on disk, where the Bedrock
//! app (and its sync) picks them up like any other edit.

use bedrock_core::Vault;
use chrono::NaiveDate;

use crate::{Backend, FileMeta, Result, Store, VaultInfo};

#[derive(Clone)]
pub struct LocalStore {
    vault: Vault,
}

impl LocalStore {
    pub fn new(vault: Vault) -> Self {
        Self { vault }
    }
}

impl Store for LocalStore {
    async fn list(&self) -> Result<Vec<FileMeta>> {
        Ok(self
            .vault
            .walk()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| FileMeta { path: e.path, size: e.size, updated: e.mtime / 1000 })
            .collect())
    }

    async fn read(&self, path: &str) -> Result<Option<String>> {
        Ok(self.vault.read_string_opt(path)?)
    }

    async fn write(&self, path: &str, content: &str) -> Result<()> {
        Ok(self.vault.write(path, content.as_bytes())?)
    }

    /// Moves the note to the vault trash, so it can be restored.
    async fn delete(&self, path: &str) -> Result<()> {
        self.vault.trash(path)?;
        Ok(())
    }

    fn can_write(&self) -> bool {
        true
    }

    fn today(&self) -> NaiveDate {
        chrono::Local::now().date_naive()
    }
}

pub struct LocalBackend {
    store: LocalStore,
    name: String,
}

impl LocalBackend {
    pub fn new(vault: Vault) -> Self {
        let name = vault.name();
        Self { store: LocalStore::new(vault), name }
    }
}

impl Backend for LocalBackend {
    type Store = LocalStore;

    fn multi_vault(&self) -> bool {
        false
    }

    async fn vaults(&self) -> Result<Vec<VaultInfo>> {
        Ok(vec![VaultInfo { id: "local".into(), name: self.name.clone(), role: "owner".into() }])
    }

    async fn open(&self, _vault: Option<&str>) -> Result<LocalStore> {
        Ok(self.store.clone())
    }
}
