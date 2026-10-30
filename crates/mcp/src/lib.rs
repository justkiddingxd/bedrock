//! Bedrock notes and tasks for AI assistants, over the Model Context Protocol.
//!
//! The tools ([`tools`]) work on any [`Store`]: a vault folder on this machine
//! ([`local`], served over stdio by the `bedrock-mcp` binary) or a server
//! vault (the sync server implements a store over its database and serves
//! the same tools over HTTP). [`protocol`] is the JSON-RPC layer.

use std::future::Future;

use chrono::NaiveDate;
use serde::Serialize;

pub mod local;
pub mod protocol;
pub mod tools;

#[derive(Debug, Clone)]
pub struct ToolError(pub String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<bedrock_core::Error> for ToolError {
    fn from(e: bedrock_core::Error) -> Self {
        ToolError(e.to_string())
    }
}

impl From<String> for ToolError {
    fn from(s: String) -> Self {
        ToolError(s)
    }
}

impl From<&str> for ToolError {
    fn from(s: &str) -> Self {
        ToolError(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ToolError>;

#[derive(Debug, Clone, Serialize)]
pub struct FileMeta {
    pub path: String,
    pub size: u64,
    /// Unix time, seconds.
    pub updated: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    /// "owner", "editor" or "viewer".
    pub role: String,
}

/// The files of one vault.
pub trait Store: Send + Sync {
    fn list(&self) -> impl Future<Output = Result<Vec<FileMeta>>> + Send;
    fn read(&self, path: &str) -> impl Future<Output = Result<Option<String>>> + Send;
    fn write(&self, path: &str, content: &str) -> impl Future<Output = Result<()>> + Send;
    fn delete(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
    fn can_write(&self) -> bool;
    /// Today in the vault owner's time zone (for daily notes and ✅ dates).
    fn today(&self) -> NaiveDate;
}

/// Where the tools find vaults: one local vault, or the vaults of a server account.
pub trait Backend: Send + Sync {
    type Store: Store;
    /// Several vaults: the tools then take a `vault` argument.
    fn multi_vault(&self) -> bool;
    fn vaults(&self) -> impl Future<Output = Result<Vec<VaultInfo>>> + Send;
    /// Opens a vault by id or name (`None`: the only one there is).
    fn open(&self, vault: Option<&str>) -> impl Future<Output = Result<Self::Store>> + Send;
}
