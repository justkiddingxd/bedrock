//! Three-way text merge used when the same note changed in two places
//! (editor vs. disk, or this device vs. the sync server).

use crate::vault::{self, Vault};
use crate::Result;

#[derive(Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean(String),
    Conflict,
}

pub fn merge3(base: &str, ours: &str, theirs: &str) -> MergeOutcome {
    if ours == theirs || base == theirs {
        return MergeOutcome::Clean(ours.to_string());
    }
    if base == ours {
        return MergeOutcome::Clean(theirs.to_string());
    }
    match diffy::merge(base, ours, theirs) {
        Ok(merged) => MergeOutcome::Clean(merged),
        Err(_) => MergeOutcome::Conflict,
    }
}

/// Free path for a conflict copy: "Note (conflict laptop 2026-10-29 14-05).md".
pub fn conflict_copy_path(vault: &Vault, rel: &str, label: &str) -> Result<String> {
    let (dir, stem, ext) = vault::split(rel);
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M");
    let label = label.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "");
    let name = if label.is_empty() {
        format!("{stem} (conflict {stamp}){ext}")
    } else {
        format!("{stem} (conflict {label} {stamp}){ext}")
    };
    vault.unique_path(&vault::join(dir, &name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_disjoint_edits() {
        let base = "a\nb\nc\nd\n";
        let ours = "A\nb\nc\nd\n";
        let theirs = "a\nb\nc\nD\n";
        assert_eq!(merge3(base, ours, theirs), MergeOutcome::Clean("A\nb\nc\nD\n".into()));
    }

    #[test]
    fn detects_conflicts() {
        assert_eq!(merge3("a\n", "b\n", "c\n"), MergeOutcome::Conflict);
    }
}
