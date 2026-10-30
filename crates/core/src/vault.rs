//! Paths inside a vault are always relative, '/'-separated strings
//! ("Projects/Bedrock.md"). Anything whose path has a component starting with
//! '.' (".bedrock", ".trash", ".git", ".obsidian") is invisible to the vault API.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use walkdir::WalkDir;

use crate::{Error, Result};

/// Folder inside the vault where Bedrock keeps its own state.
pub const META_DIR: &str = ".bedrock";
/// Deleted files are moved here instead of being removed.
pub const TRASH_DIR: &str = ".trash";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Modification time, milliseconds since the Unix epoch.
    pub mtime: i64,
    /// Modification time, nanoseconds since the Unix epoch (for change detection).
    #[serde(skip)]
    pub mtime_ns: i64,
}

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(Error::NotFound(root.display().to_string()));
        }
        let root = dunce::canonicalize(root)?;
        fs::create_dir_all(root.join(META_DIR))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn meta_dir(&self) -> PathBuf {
        self.root.join(META_DIR)
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Validates a vault-relative path and returns the absolute path on disk.
    pub fn abs(&self, rel: &str) -> Result<PathBuf> {
        let rel = normalize(rel)?;
        if is_hidden(&rel) {
            return Err(Error::InvalidPath(rel));
        }
        Ok(self.root.join(to_os(&rel)))
    }

    /// Converts an absolute path back to a vault path. Returns `None` for
    /// paths outside the vault and for hidden ones.
    pub fn rel(&self, abs: &Path) -> Option<String> {
        let stripped = abs.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for c in stripped.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_str()?.to_string()),
                _ => return None,
            }
        }
        if parts.is_empty() {
            return None;
        }
        let rel = parts.join("/");
        (!is_hidden(&rel)).then_some(rel)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.abs(rel).map(|p| p.exists()).unwrap_or(false)
    }

    pub fn read_bytes(&self, rel: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.abs(rel)?)?)
    }

    /// Reads a file, returning `None` if it does not exist.
    pub fn read_opt(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        match fs::read(self.abs(rel)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn read_string(&self, rel: &str) -> Result<String> {
        Ok(bytes_to_string(self.read_bytes(rel)?))
    }

    pub fn read_string_opt(&self, rel: &str) -> Result<Option<String>> {
        Ok(self.read_opt(rel)?.map(bytes_to_string))
    }

    /// Atomically replaces the file: writes a temp file, then renames it over
    /// the target, so a crash never leaves a half-written note behind.
    pub fn write(&self, rel: &str, data: &[u8]) -> Result<()> {
        let path = self.abs(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_dir = self.meta_dir().join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&tmp_dir)?;
        tmp.write_all(data)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&path).map_err(|e| Error::Io(e.error))?;
        Ok(())
    }

    pub fn create_dir(&self, rel: &str) -> Result<()> {
        fs::create_dir_all(self.abs(rel)?)?;
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let src = self.abs(from)?;
        let dst = self.abs(to)?;
        if !src.exists() {
            return Err(Error::NotFound(from.to_string()));
        }
        let case_only = from.to_lowercase() == to.to_lowercase();
        if dst.exists() && !case_only {
            return Err(Error::AlreadyExists(to.to_string()));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(src, dst)?;
        Ok(())
    }

    /// Moves a file or folder into `.trash/`, keeping its relative path.
    /// Returns where it went, relative to `.trash/` (for [`Vault::restore`]).
    pub fn trash(&self, rel: &str) -> Result<String> {
        let src = self.abs(rel)?;
        if !src.exists() {
            return Err(Error::NotFound(rel.to_string()));
        }
        let rel = normalize(rel)?;
        let trash = self.root.join(TRASH_DIR);
        let mut target = rel.clone();
        if trash.join(to_os(&target)).exists() {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let (dir, stem, ext) = split(&rel);
            target = join(dir, &format!("{stem} {stamp}{ext}"));
        }
        let dst = trash.join(to_os(&target));
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(src, dst)?;
        Ok(target)
    }

    /// Deletes a file only if it holds nothing but whitespace, such as an
    /// untitled note that was opened and left. Returns whether it was deleted.
    pub fn remove_if_blank(&self, rel: &str) -> Result<bool> {
        let path = self.abs(rel)?;
        if !path.is_file() {
            return Ok(false);
        }
        if !bytes_to_string(fs::read(&path)?).trim().is_empty() {
            return Ok(false);
        }
        fs::remove_file(path)?;
        Ok(true)
    }

    /// Moves something from `.trash/` back into the vault at `rel` (or a free
    /// name next to it). Returns the path it was restored to.
    pub fn restore(&self, trashed: &str, rel: &str) -> Result<String> {
        let src = self.root.join(TRASH_DIR).join(to_os(&normalize(trashed)?));
        if !src.exists() {
            return Err(Error::NotFound(trashed.to_string()));
        }
        let rel = self.unique_path(rel)?;
        let dst = self.abs(&rel)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(src, dst)?;
        Ok(rel)
    }

    /// Returns `rel` if it is free, otherwise "name 1.ext", "name 2.ext", ...
    pub fn unique_path(&self, rel: &str) -> Result<String> {
        let rel = normalize(rel)?;
        if !self.abs(&rel)?.exists() {
            return Ok(rel);
        }
        let (dir, stem, ext) = split(&rel);
        for i in 1.. {
            let candidate = join(dir, &format!("{stem} {i}{ext}"));
            if !self.abs(&candidate)?.exists() {
                return Ok(candidate);
            }
        }
        unreachable!()
    }

    /// All visible files and folders in the vault.
    pub fn walk(&self) -> Vec<Entry> {
        self.walk_under(&self.root)
    }

    /// All visible files and folders below a folder of the vault.
    pub fn walk_dir(&self, rel: &str) -> Vec<Entry> {
        match self.abs(rel) {
            Ok(p) => self.walk_under(&p),
            Err(_) => Vec::new(),
        }
    }

    fn walk_under(&self, start: &Path) -> Vec<Entry> {
        WalkDir::new(start)
            .min_depth(1)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = self.rel(e.path())?;
                let meta = e.metadata().ok()?;
                let is_dir = meta.is_dir();
                Some(Entry {
                    path,
                    is_dir,
                    size: if is_dir { 0 } else { meta.len() },
                    mtime: mtime_ns(&meta) / 1_000_000,
                    mtime_ns: mtime_ns(&meta),
                })
            })
            .collect()
    }
}

pub fn mtime_ns(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn bytes_to_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Normalizes a vault path: '/' separators, no empty or "." parts, and rejects
/// "..", absolute paths and characters that are invalid in Windows file names.
pub fn normalize(rel: &str) -> Result<String> {
    let mut parts = Vec::new();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return Err(Error::InvalidPath(rel.to_string())),
            p if p
                .chars()
                .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()) =>
            {
                return Err(Error::InvalidPath(rel.to_string()));
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(Error::InvalidPath(rel.to_string()));
    }
    Ok(parts.join("/"))
}

pub fn is_hidden(rel: &str) -> bool {
    rel.split('/').any(|p| p.starts_with('.'))
}

fn to_os(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

/// Splits "dir/name.ext" into ("dir", "name", ".ext").
pub fn split(rel: &str) -> (&str, &str, &str) {
    let (dir, name) = match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    match name.rfind('.') {
        Some(i) if i > 0 => (dir, &name[..i], &name[i..]),
        _ => (dir, name, ""),
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

pub fn parent(rel: &str) -> &str {
    split(rel).0
}

pub fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

pub fn stem(rel: &str) -> &str {
    split(rel).1
}

pub fn extension(rel: &str) -> String {
    split(rel).2.trim_start_matches('.').to_lowercase()
}

pub fn is_note(rel: &str) -> bool {
    matches!(extension(rel).as_str(), "md" | "markdown")
}

pub fn is_image(rel: &str) -> bool {
    matches!(
        extension(rel).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif"
    )
}

/// Path without the ".md" extension, used as link text.
pub fn strip_note_ext(rel: &str) -> &str {
    if is_note(rel) {
        let (_, _, ext) = split(rel);
        &rel[..rel.len() - ext.len()]
    } else {
        rel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("a\\b/./c.md").unwrap(), "a/b/c.md");
        assert!(normalize("../x.md").is_err());
        assert!(normalize("a/../../x").is_err());
        assert!(normalize("C:/x").is_err());
        assert!(normalize("").is_err());
        assert!(is_hidden(".bedrock/sync/state.json"));
        assert!(!is_hidden("notes/a.md"));
    }

    #[test]
    fn split_names() {
        assert_eq!(split("a/b/note.md"), ("a/b", "note", ".md"));
        assert_eq!(split("note"), ("", "note", ""));
        assert_eq!(strip_note_ext("a/Note.md"), "a/Note");
        assert_eq!(strip_note_ext("img.png"), "img.png");
    }

    #[test]
    fn write_read_unique_trash() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.write("folder/Note.md", b"hello").unwrap();
        assert_eq!(v.read_string("folder/Note.md").unwrap(), "hello");
        assert_eq!(v.unique_path("folder/Note.md").unwrap(), "folder/Note 1.md");
        assert!(v.write(".bedrock/evil", b"x").is_err());
        let trashed = v.trash("folder/Note.md").unwrap();
        assert!(!v.exists("folder/Note.md"));
        let paths: Vec<_> = v.walk().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, vec!["folder".to_string()]);

        // Undo puts it back where it was.
        assert_eq!(v.restore(&trashed, "folder/Note.md").unwrap(), "folder/Note.md");
        assert_eq!(v.read_string("folder/Note.md").unwrap(), "hello");
        // A second trash of the same path gets a distinct name.
        let a = v.trash("folder/Note.md").unwrap();
        v.write("folder/Note.md", b"again").unwrap();
        let b = v.trash("folder/Note.md").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn remove_only_blank() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.write("Blank.md", b" \n\n").unwrap();
        v.write("Text.md", b"x").unwrap();
        v.create_dir("folder").unwrap();
        assert!(v.remove_if_blank("Blank.md").unwrap());
        assert!(!v.exists("Blank.md"));
        assert!(!v.remove_if_blank("Text.md").unwrap());
        assert!(v.exists("Text.md"));
        assert!(!v.remove_if_blank("folder").unwrap());
        assert!(!v.remove_if_blank("Missing.md").unwrap());
    }
}
