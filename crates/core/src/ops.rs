//! Higher-level vault operations that combine the file system and the index.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

use crate::index::Index;
use crate::parse::WIKILINK;
use crate::vault::{self, Vault};
use crate::Result;

static TASK_BOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*(?:[-*+]|\d+[.)])\s+\[)(.)(\])").unwrap());

/// Creates an empty note with a free name in `folder`.
pub fn create_note(vault: &Vault, folder: Option<&str>, name: Option<&str>) -> Result<String> {
    let name = name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Без названия");
    let file = if vault::is_note(name) { name.to_string() } else { format!("{name}.md") };
    let rel = vault.unique_path(&vault::join(folder.unwrap_or("").trim_matches('/'), &file))?;
    vault.write(&rel, b"")?;
    Ok(rel)
}

/// Returns the daily note for `date`, creating it from a template if needed.
pub fn daily_note(vault: &Vault, date: NaiveDate) -> Result<String> {
    let rel = daily_path(date);
    if !vault.exists(&rel) {
        vault.write(&rel, b"")?;
    }
    Ok(rel)
}

/// Where the daily note for `date` lives.
pub fn daily_path(date: NaiveDate) -> String {
    format!("{DAILY_DIR}/{}.md", date.format("%Y-%m-%d"))
}

pub const DAILY_DIR: &str = "Дневник";

static DUE_MARK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*(?:📅\x{FE0F}?\s*|due:)\d{4}-\d{2}-\d{2}").unwrap());

/// Sets or clears the due date of the task on `line`, keeping the marker
/// style the line already uses ("📅 2026-10-29" or "due:2026-10-29").
pub fn set_task_due(vault: &Vault, rel: &str, line: usize, due: Option<NaiveDate>) -> Result<String> {
    let updated = text::set_task_due(&vault.read_string(rel)?, line, due)?;
    vault.write(rel, updated.as_bytes())?;
    Ok(updated)
}

/// Flips a task checkbox on `line` ("[ ]" <-> "[x]") and stamps or removes
/// the completion date. Returns the new file content.
pub fn toggle_task(vault: &Vault, rel: &str, line: usize, today: NaiveDate) -> Result<String> {
    let updated = text::toggle_task(&vault.read_string(rel)?, line, today)?;
    vault.write(rel, updated.as_bytes())?;
    Ok(updated)
}

/// The task operations on note text, for callers without a local vault
/// (the sync server, MCP).
pub mod text {
    use chrono::NaiveDate;

    use super::{DUE_MARK, TASK_BOX};
    use crate::parse::DONE_DATE;
    use crate::{Error, Result};

    /// Applies `f` to the body of `line` (without its line ending).
    fn edit_line(content: &str, line: usize, f: impl FnOnce(&str) -> Result<String>) -> Result<String> {
        let mut lines: Vec<String> = content.split_inclusive('\n').map(String::from).collect();
        let raw = lines.get_mut(line).ok_or_else(|| Error::Other(format!("нет строки {line}")))?;
        let eol_len = raw.len() - raw.trim_end_matches(['\n', '\r']).len();
        let (body, eol) = raw.split_at(raw.len() - eol_len);
        *raw = format!("{}{eol}", f(body)?);
        Ok(lines.concat())
    }

    pub fn set_task_due(content: &str, line: usize, due: Option<NaiveDate>) -> Result<String> {
        edit_line(content, line, |body| {
            if !TASK_BOX.is_match(body) {
                return Err(Error::Other("в строке нет задачи".into()));
            }
            let due_style = body.contains("due:");
            let stripped = DUE_MARK.replace_all(body, "");
            let stripped = stripped.trim_end();
            Ok(match due {
                Some(d) if due_style => format!("{stripped} due:{}", d.format("%Y-%m-%d")),
                Some(d) => format!("{stripped} 📅 {}", d.format("%Y-%m-%d")),
                None => stripped.to_string(),
            })
        })
    }

    /// Marks the task on `line` done (with today's ✅ date) or not done.
    pub fn set_task_done(content: &str, line: usize, done: bool, today: NaiveDate) -> Result<String> {
        edit_line(content, line, |body| {
            let caps = TASK_BOX.captures(body).ok_or_else(|| Error::Other("в строке нет задачи".into()))?;
            let status = caps.get(2).unwrap();
            let was_done = matches!(status.as_str(), "x" | "X" | "-");
            if was_done == done {
                return Ok(body.to_string());
            }
            let flipped = format!("{}{}{}", &body[..status.start()], if done { "x" } else { " " }, &body[status.end()..]);
            Ok(if done {
                format!("{} ✅ {}", flipped.trim_end(), today.format("%Y-%m-%d"))
            } else {
                DONE_DATE.replace_all(&flipped, "").into_owned()
            })
        })
    }

    pub fn toggle_task(content: &str, line: usize, today: NaiveDate) -> Result<String> {
        let body = content.lines().nth(line).ok_or_else(|| Error::Other(format!("нет строки {line}")))?;
        let caps = TASK_BOX.captures(body).ok_or_else(|| Error::Other("в строке нет задачи".into()))?;
        let done = matches!(&caps[2], "x" | "X" | "-");
        set_task_done(content, line, !done, today)
    }

    /// Adds "- [ ] task" to a note: at the end of its "Задачи" section if it
    /// has one, otherwise at the end of the note.
    pub fn insert_task(content: &str, task: &str) -> String {
        let new_line = format!("- [ ] {task}");
        let mut lines: Vec<&str> = content.split('\n').collect();
        let heading = lines.iter().position(|l| {
            let t = l.trim();
            t.starts_with('#') && t.trim_start_matches('#').trim().to_lowercase() == "задачи"
        });
        let Some(h) = heading else {
            let body = content.trim_end();
            return if body.is_empty() { format!("{new_line}\n") } else { format!("{body}\n{new_line}\n") };
        };
        let is_item = |l: &str| l.trim().is_empty() || TASK_BOX.is_match(l);
        let mut end = h + 1;
        while end < lines.len() && is_item(lines[end]) {
            end += 1;
        }
        while end > h + 1 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        lines.insert(end, &new_line);
        lines.join("\n")
    }

    /// Appends text on a new line at the end of a note.
    pub fn append(content: &str, text: &str) -> String {
        let body = content.trim_end_matches(['\n', '\r']);
        if body.is_empty() { format!("{}\n", text.trim_end()) } else { format!("{body}\n{}\n", text.trim_end()) }
    }
}

/// Renames a file or folder. For files, wikilinks pointing at the old path are
/// rewritten across the vault. Returns the notes whose content changed.
pub fn rename_with_links(vault: &Vault, index: &mut Index, from: &str, to: &str) -> Result<Vec<String>> {
    let from = vault::normalize(from)?;
    let to = vault::normalize(to)?;
    let is_dir = vault.abs(&from)?.is_dir();

    // Record which link targets point at `from` while the old path still
    // resolves: note path -> lowercase targets.
    let mut sources: HashMap<String, HashSet<String>> = HashMap::new();
    if !is_dir {
        for note in index.notes() {
            for l in &note.parsed.links {
                if index.resolve(&l.target, &note.path).as_deref() == Some(from.as_str()) {
                    sources.entry(note.path.clone()).or_default().insert(l.target.trim().to_lowercase());
                }
            }
        }
    }

    vault.rename(&from, &to)?;
    index.remove_path(&from);
    index.update_path(vault, &to);
    if is_dir {
        return Ok(Vec::new());
    }

    let mut updated = Vec::new();
    for (src, targets) in sources {
        let current = if src == from { to.clone() } else { src };
        let content = vault.read_string(&current)?;
        let mut changed = false;
        let new_content = WIKILINK.replace_all(&content, |c: &regex::Captures| {
            let target = c[2].trim();
            if target.is_empty() || !targets.contains(&target.to_lowercase()) {
                return c[0].to_string();
            }
            changed = true;
            let new_target = if target.contains('/') {
                vault::strip_note_ext(&to).to_string()
            } else {
                index.link_text_for(&to)
            };
            let mut s = String::new();
            if c.get(1).is_some() {
                s.push('!');
            }
            s.push_str("[[");
            s.push_str(&new_target);
            if let Some(h) = c.get(3) {
                s.push('#');
                s.push_str(h.as_str());
            }
            if let Some(a) = c.get(4) {
                s.push('|');
                s.push_str(a.as_str());
            }
            s.push_str("]]");
            s
        });
        if changed {
            vault.write(&current, new_content.as_bytes())?;
            index.update_path(vault, &current);
            updated.push(current);
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reschedules_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.write("t.md", "- [ ] a 📅 2026-10-20 ⏫\n- [ ] b due:2026-10-01\n- [ ] c\n".as_bytes()).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        set_task_due(&v, "t.md", 0, Some(day)).unwrap();
        set_task_due(&v, "t.md", 1, Some(day)).unwrap();
        let s = set_task_due(&v, "t.md", 2, Some(day)).unwrap();
        assert_eq!(s, "- [ ] a ⏫ 📅 2026-10-29\n- [ ] b due:2026-10-29\n- [ ] c 📅 2026-10-29\n");
        let s = set_task_due(&v, "t.md", 2, None).unwrap();
        assert!(s.ends_with("- [ ] c\n"), "{s:?}");
    }

    #[test]
    fn toggles_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.write("t.md", "# T\r\n- [ ] one\r\n- [x] two ✅ 2026-01-01\r\n".as_bytes()).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let s = toggle_task(&v, "t.md", 1, day).unwrap();
        assert_eq!(s, "# T\r\n- [x] one ✅ 2026-10-29\r\n- [x] two ✅ 2026-01-01\r\n");
        let s = toggle_task(&v, "t.md", 2, day).unwrap();
        assert!(s.ends_with("- [ ] two\r\n"), "{s:?}");
        assert!(toggle_task(&v, "t.md", 0, day).is_err());
    }

    #[test]
    fn rename_updates_links() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.write("Old.md", b"self [[Old#Top]]").unwrap();
        v.write("A.md", b"see [[Old]] and [[Old|alias]] and [[Other]]").unwrap();
        v.write("sub/B.md", b"![[Old]] [[Old.md]]").unwrap();
        v.write("Other.md", b"x").unwrap();
        let mut idx = Index::build(&v);
        let mut changed = rename_with_links(&v, &mut idx, "Old.md", "New name.md").unwrap();
        changed.sort();
        assert_eq!(changed, vec!["A.md", "New name.md", "sub/B.md"]);
        assert_eq!(v.read_string("A.md").unwrap(), "see [[New name]] and [[New name|alias]] and [[Other]]");
        assert_eq!(v.read_string("sub/B.md").unwrap(), "![[New name]] [[New name]]");
        assert_eq!(v.read_string("New name.md").unwrap(), "self [[New name#Top]]");
        assert_eq!(idx.backlinks("New name.md").len(), 4);
    }

    #[test]
    fn text_ops() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let note = "# День\n\n## Задачи\n- [ ] one\n\n## Заметки\ntext\n";
        let with = text::insert_task(note, "two");
        assert_eq!(with, "# День\n\n## Задачи\n- [ ] one\n- [ ] two\n\n## Заметки\ntext\n");
        assert_eq!(text::insert_task("", "x"), "- [ ] x\n");
        assert_eq!(text::insert_task("plain", "x"), "plain\n- [ ] x\n");
        // Marking done twice changes nothing the second time.
        let done = text::set_task_done(&with, 4, true, day).unwrap();
        assert!(done.contains("- [x] two ✅ 2026-10-29\n"));
        assert_eq!(text::set_task_done(&done, 4, true, day).unwrap(), done);
        assert!(text::set_task_done(&done, 4, false, day).unwrap().contains("- [ ] two\n"));
        assert_eq!(text::append("a\n\n", "b"), "a\nb\n");
    }
}
