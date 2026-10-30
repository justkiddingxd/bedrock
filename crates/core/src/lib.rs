//! Bedrock core: a vault is a plain folder of markdown files. This crate knows
//! how to read and write it, parse notes (links, tags, tasks), keep an
//! in-memory index for search and backlinks, and render notes to HTML.

pub mod error;
pub mod fuzzy;
pub mod index;
pub mod merge;
pub mod ops;
pub mod parse;
pub mod recur;
pub mod render;
pub mod vault;

pub use error::{Error, Result};
pub use index::Index;
pub use vault::Vault;
