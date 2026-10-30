//! Watches the vault folder for changes made outside the app (other editors,
//! sync, git) and keeps the index and the UI up to date.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant};

use bedrock_core::Vault;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

use crate::state::Workspace;
use crate::sync_task::SyncHandle;

const DEBOUNCE: Duration = Duration::from_millis(250);

pub fn start(
    app: AppHandle,
    ws: Arc<RwLock<Option<Workspace>>>,
    sync: Arc<SyncHandle>,
    vault: Vault,
) -> notify::Result<RecommendedWatcher> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(vault.root(), RecursiveMode::Recursive)?;

    std::thread::spawn(move || {
        // Ends when the watcher (and with it the sender) is dropped.
        while let Ok(first) = rx.recv() {
            let mut paths = BTreeSet::new();
            collect(first, &mut paths);
            let deadline = Instant::now() + DEBOUNCE;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(left) {
                    Ok(ev) => collect(ev, &mut paths),
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }

            let rels: Vec<String> = paths.iter().filter_map(|p| vault.rel(p)).collect();
            if rels.is_empty() {
                continue;
            }
            {
                let Ok(mut guard) = ws.write() else { return };
                match guard.as_mut() {
                    Some(w) if w.vault.root() == vault.root() => {
                        for rel in &rels {
                            w.index.update_path(&w.vault, rel);
                        }
                    }
                    _ => return, // another vault was opened
                }
            }
            let _ = app.emit("vault:changed", &rels);
            sync.kick();
        }
    });
    Ok(watcher)
}

fn collect(ev: notify::Result<Event>, out: &mut BTreeSet<PathBuf>) {
    if let Ok(ev) = ev {
        out.extend(ev.paths);
    }
}
