use std::sync::{Arc, Mutex, RwLock};

use bedrock_core::{Index, Vault};

use crate::sync_task::SyncHandle;

/// The open vault and its index.
pub struct Workspace {
    pub vault: Vault,
    pub index: Index,
}

#[derive(Default)]
pub struct AppState {
    pub ws: Arc<RwLock<Option<Workspace>>>,
    /// Dropping the watcher stops it (and its event thread).
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub sync: Arc<SyncHandle>,
}
