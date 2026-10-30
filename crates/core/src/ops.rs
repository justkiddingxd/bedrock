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

/// Sets a task's status: 'x' completes it, ' ' reopens it, '/' marks it in progress.
pub fn set_task_status(vault: &Vault, rel: &str, line: usize, status: char, today: NaiveDate) -> Result<String> {
    let updated = text::set_task_status(&vault.read_string(rel)?, line, status, today)?;
    vault.write(rel, updated.as_bytes())?;
    Ok(updated)
}

/// Moves a repeating reminder line (not a task) past `after`. Returns whether the note changed.
pub fn advance_reminder(vault: &Vault, rel: &str, line: usize, after: chrono::NaiveDateTime) -> Result<bool> {
    let content = vault.read_string(rel)?;
    let Some(body) = content.lines().nth(line) else { return Ok(false) };
    let Some(moved) = text::advance_reminder(body, after) else { return Ok(false) };
    let updated = text::replace_line(&content, line, &moved)?;
    vault.write(rel, updated.as_bytes())?;
    Ok(true)
}

/// The task operations on note text, for callers without a local vault
/// (the sync server, MCP).
pub mod text {
    use std::sync::LazyLock;

    use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
    use regex::Regex;

    use super::{DUE_MARK, TASK_BOX};
    use crate::parse::DONE_DATE;
    use crate::recur::{self, RECUR};
    use crate::{Error, Result};

    static DUE_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(📅\x{FE0F}?\s*|due:)(\d{4}-\d{2}-\d{2})").unwrap());
    static REMIND_AT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(⏰\x{FE0F}?\s*)(\d{4}-\d{2}-\d{2})([ T])(\d{1,2}):(\d{2})").unwrap());

    fn date(s: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    }

    /// Replaces the text of `line`, keeping its line ending.
    pub fn replace_line(content: &str, line: usize, text: &str) -> Result<String> {
        edit_line(content, line, |_| Ok(text.to_string()))
    }

    fn insert_line(content: &str, at: usize, text: &str) -> String {
        let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
        let mut lines: Vec<String> = content.split_inclusive('\n').map(String::from).collect();
        lines.insert(at.min(lines.len()), format!("{text}{eol}"));
        lines.concat()
    }

    /// The next occurrence of a repeating task, as a new open task line: its due
    /// date moves by the rule (not into the past) and its reminder moves along.
    /// `None` when the line does not repeat.
    pub fn next_occurrence(body: &str, today: NaiveDate) -> Option<String> {
        let caps = TASK_BOX.captures(body)?;
        let rule = recur::parse_rule(&RECUR.captures(body)?[1])?;
        let status = caps.get(2).unwrap();
        let open = format!("{} {}", &body[..status.start()], &body[status.end()..]);
        let open = DONE_DATE.replace_all(&open, "").trim_end().to_string();
        let due = DUE_DATE.captures(&open).and_then(|c| date(&c[2]));
        let remind = REMIND_AT.captures(&open).and_then(|c| date(&c[2]));
        Some(match (due, remind) {
            (Some(due), _) => {
                let next = recur::next_from(rule, due, today);
                let shift = next - due;
                let moved = DUE_DATE.replace(&open, |c: &regex::Captures| format!("{}{}", &c[1], next.format("%Y-%m-%d")));
                REMIND_AT
                    .replace(&moved, |c: &regex::Captures| {
                        let at = date(&c[2]).map_or(c[2].to_string(), |d| (d + shift).format("%Y-%m-%d").to_string());
                        format!("{}{at}{}{}:{}", &c[1], &c[3], &c[4], &c[5])
                    })
                    .into_owned()
            }
            (None, Some(at)) => {
                let next = recur::next_from(rule, at, today + Duration::days(1));
                REMIND_AT
                    .replace(&open, |c: &regex::Captures| format!("{}{}{}{}:{}", &c[1], next.format("%Y-%m-%d"), &c[3], &c[4], &c[5]))
                    .into_owned()
            }
            (None, None) => format!("{open} 📅 {}", recur::next_after(rule, today).format("%Y-%m-%d")),
        })
    }

    /// A repeating reminder line that is not a task, moved to its first time after `after`.
    pub fn advance_reminder(body: &str, after: NaiveDateTime) -> Option<String> {
        if TASK_BOX.is_match(body) {
            return None;
        }
        let rule = recur::parse_rule(&RECUR.captures(body)?[1])?;
        let c = REMIND_AT.captures(body)?;
        let time = NaiveTime::from_hms_opt(c[4].parse().ok()?, c[5].parse().ok()?, 0)?;
        let mut day = recur::next_after(rule, date(&c[2])?);
        while day.and_time(time) <= after {
            day = recur::next_after(rule, day);
        }
        Some(REMIND_AT.replace(body, |c: &regex::Captures| format!("{}{}{}{}:{}", &c[1], day.format("%Y-%m-%d"), &c[3], &c[4], &c[5])).into_owned())
    }

    /// Sets the status character: 'x' completes the task (with its date and,
    /// if it repeats, the next occurrence), ' ' reopens it, '/' marks it in progress.
    pub fn set_task_status(content: &str, line: usize, status: char, today: NaiveDate) -> Result<String> {
        if matches!(status, 'x' | 'X') {
            return set_task_done(content, line, true, today);
        }
        let reopened = set_task_done(content, line, false, today)?;
        edit_line(&reopened, line, |body| {
            let caps = TASK_BOX.captures(body).ok_or_else(|| Error::Other("в строке нет задачи".into()))?;
            let s = caps.get(2).unwrap();
            Ok(format!("{}{status}{}", &body[..s.start()], &body[s.end()..]))
        })
    }

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

    /// Marks the task on `line` done (with today's ✅ date) or not done. A
    /// repeating task that gets done is followed, above it, by its next occurrence.
    pub fn set_task_done(content: &str, line: usize, done: bool, today: NaiveDate) -> Result<String> {
        let mut next = None;
        let updated = edit_line(content, line, |body| {
            let caps = TASK_BOX.captures(body).ok_or_else(|| Error::Other("в строке нет задачи".into()))?;
            let status = caps.get(2).unwrap();
            let was_done = matches!(status.as_str(), "x" | "X" | "-");
            if was_done == done {
                return Ok(body.to_string());
            }
            let flipped = format!("{}{}{}", &body[..status.start()], if done { "x" } else { " " }, &body[status.end()..]);
            Ok(if done {
                next = next_occurrence(body, today);
                format!("{} ✅ {}", flipped.trim_end(), today.format("%Y-%m-%d"))
            } else {
                DONE_DATE.replace_all(&flipped, "").into_owned()
            })
        })?;
        Ok(match next {
            Some(n) => insert_line(&updated, line, &n),
            None => updated,
        })
    }

    /// One task line marked done or not done, with the next occurrence of a
    /// repeating task (for an editor that applies the change itself).
    pub fn line_done(body: &str, done: bool, today: NaiveDate) -> Result<(String, Option<String>)> {
        let updated = set_task_done(body, 0, done, today)?;
        let mut lines = updated.lines();
        Ok(match (lines.next(), lines.next()) {
            (Some(next), Some(line)) => (line.to_string(), Some(next.to_string())),
            (Some(line), None) => (line.to_string(), None),
            _ => (body.to_string(), None),
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
    fn repeating_tasks() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(); // a Friday
        let note = "# План\n- [ ] Созвон 🔁 каждый понедельник 📅 2026-10-26 ⏰ 2026-10-26 10:00 ⏫\n- [ ] Разовая\n";
        let done = text::set_task_done(note, 1, true, today).unwrap();
        assert_eq!(
            done,
            "# План\n- [ ] Созвон 🔁 каждый понедельник 📅 2026-11-02 ⏰ 2026-11-02 10:00 ⏫\n\
             - [x] Созвон 🔁 каждый понедельник 📅 2026-10-26 ⏰ 2026-10-26 10:00 ⏫ ✅ 2026-10-30\n- [ ] Разовая\n"
        );
        // Without a date the next one is put on the calendar; a plain task does not repeat.
        let (line, next) = text::line_done("- [ ] Полить цветы 🔁 каждые 3 дня", true, today).unwrap();
        assert_eq!(line, "- [x] Полить цветы 🔁 каждые 3 дня ✅ 2026-10-30");
        assert_eq!(next.as_deref(), Some("- [ ] Полить цветы 🔁 каждые 3 дня 📅 2026-11-02"));
        assert_eq!(text::line_done("- [ ] Разовая", true, today).unwrap().1, None);
        // Only a reminder: it moves to the next day of the rule after today.
        let next = text::next_occurrence("- [ ] Аренда 🔁 1-го числа ⏰ 2026-10-01 09:00", today).unwrap();
        assert_eq!(next, "- [ ] Аренда 🔁 1-го числа ⏰ 2026-11-01 09:00");
        // Reopening keeps the task; the board's statuses.
        assert_eq!(text::set_task_status("- [x] A ✅ 2026-10-29", 0, '/', today).unwrap(), "- [/] A");
        assert_eq!(text::set_task_status("- [/] A", 0, ' ', today).unwrap(), "- [ ] A");
        assert_eq!(text::set_task_status("- [/] A", 0, 'x', today).unwrap(), "- [x] A ✅ 2026-10-30");
        // A repeating reminder line moves past the given time.
        let after = today.and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(
            text::advance_reminder("Полить цветы ⏰ 2026-10-23 09:00 🔁 каждую неделю", after).as_deref(),
            Some("Полить цветы ⏰ 2026-11-06 09:00 🔁 каждую неделю")
        );
        assert_eq!(text::advance_reminder("- [ ] Задача ⏰ 2026-10-23 09:00 🔁 каждую неделю", after), None);
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
