//! Sync client for Bedrock vaults.
//!
//! * [`crypto`]: vault key, password envelope, file encryption.
//! * [`client`]: typed HTTP client for the sync server.
//! * [`engine`]: the sync algorithm (pull, three-way merge, push).

pub mod client;
pub mod crypto;
pub mod engine;
mod error;
mod state;

pub use client::ApiClient;
pub use crypto::{Cipher, KdfParams, VaultKey};
pub use engine::{SyncEngine, SyncReport};
pub use error::{Result, SyncError};
pub use bedrock_proto as proto;
pub use state::SyncConfig;
