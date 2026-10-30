//! In-memory index of a vault: parsed notes, name lookup tables for link
//! resolution, and the queries built on top (backlinks, tasks, tags, search).

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;
use serde::Serialize;

use crate::fuzzy;
use crate::parse::{self, Heading, Parsed, Property, Reminder, Task, WIKILINK};
use crate::vault::{self, Vault};

pub struct Note {
    pub path: String,
    pub title: String,
    pub content: String,
    lower: String,
    pub parsed: Parsed,
    pub mtime: i64,
}

impl Note {
    pub fn new(path: String, content: String, mtime: i64) -> Self {
        let parsed = parse::parse(&content);
        let title = parsed.title.clone().unwrap_or_else(|| vault::stem(&path).to_string());
        let lower = content.to_lowercase();
        Self { path, title, content, lower, parsed, mtime }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRef {
    pub path: String,
    pub title: String,
    /// Shortest text that links to this note unambiguously.
    pub link_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlink {
    pub source: String,
    pub title: String,
    pub line: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutLink {
    pub target: String,
    pub heading: Option<String>,
    pub resolved: Option<String>,
    pub line: usize,
    pub embed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub path: String,
    pub note_title: String,
    #[serde(flatten)]
    pub task: Task,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderItem {
    pub path: String,
    pub note_title: String,
    #[serde(flatten)]
    pub reminder: Reminder,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snippet {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub score: i64,
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchHit {
    pub path: String,
    pub title: String,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteInfo {
    pub path: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub headings: Vec<Heading>,
    pub backlinks: Vec<Backlink>,
    pub outgoing: Vec<OutLink>,
    pub word_count: usize,
    pub mtime: i64,
    pub properties: Vec<Property>,
    pub tasks_total: usize,
    pub tasks_done: usize,
}

/// One row of a note list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    pub path: String,
    pub title: String,
    pub mtime: i64,
    /// First lines of text, markup stripped.
    pub excerpt: String,
    pub tags: Vec<String>,
    pub open_tasks: usize,
}

#[derive(Default)]
pub struct Index {
    notes: HashMap<String, Note>,
    /// Non-note files (attachments) and their mtime.
    files: HashMap<String, i64>,
    /// Lowercase path -> real path, for every indexed file.
    lower_paths: HashMap<String, String>,
    /// Lowercase note stem ("my note") -> note paths.
    by_stem: HashMap<String, Vec<String>>,
    /// Lowercase file name ("my note.md", "pic.png") -> paths.
    by_name: HashMap<String, Vec<String>>,
}

impl Index {
    pub fn build(vault: &Vault) -> Self {
        let entries = vault.walk();
        let notes: Vec<Note> = entries
            .par_iter()
            .filter(|e| !e.is_dir && vault::is_note(&e.path))
            .filter_map(|e| load_note(vault, &e.path, e.mtime))
            .collect();
        let mut index = Index::default();
        for e in entries.iter().filter(|e| !e.is_dir && !vault::is_note(&e.path)) {
            index.insert_file(&e.path, e.mtime);
        }
        for note in notes {
            index.insert_note(note);
        }
        index
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Re-reads a path from disk: a file, a folder (recursively) or something
    /// that no longer exists.
    pub fn update_path(&mut self, vault: &Vault, rel: &str) {
        let Ok(rel) = vault::normalize(rel) else { return };
        let Ok(abs) = vault.abs(&rel) else { return };
        match std::fs::metadata(&abs) {
            Ok(meta) if meta.is_dir() => {
                let entries = vault.walk_dir(&rel);
                let present: HashSet<&str> = entries.iter().map(|e| e.path.as_str()).collect();
                let prefix = format!("{rel}/");
                let stale: Vec<String> = self
                    .all_paths()
                    .filter(|p| p.starts_with(&prefix) && !present.contains(p.as_str()))
                    .cloned()
                    .collect();
                for p in stale {
                    self.remove_path(&p);
                }
                for e in entries.iter().filter(|e| !e.is_dir) {
                    self.update_file(vault, &e.path, e.mtime);
                }
            }
            Ok(meta) => self.update_file(vault, &rel, vault::mtime_ns(&meta) / 1_000_000),
            Err(_) => self.remove_path(&rel),
        }
    }

    /// Indexes note content directly (used by tests and right after saving).
    pub fn insert_content(&mut self, path: &str, content: String, mtime: i64) {
        self.insert_note(Note::new(path.to_string(), content, mtime));
    }

    pub fn remove_path(&mut self, rel: &str) {
        if self.notes.remove(rel).is_some() || self.files.remove(rel).is_some() {
            self.remove_names(rel);
            return;
        }
        let prefix = format!("{rel}/");
        let under: Vec<String> = self.all_paths().filter(|p| p.starts_with(&prefix)).cloned().collect();
        for p in under {
            self.notes.remove(&p);
            self.files.remove(&p);
            self.remove_names(&p);
        }
    }

    fn update_file(&mut self, vault: &Vault, rel: &str, mtime: i64) {
        if vault::is_note(rel) {
            if let Some(note) = load_note(vault, rel, mtime) {
                self.insert_note(note);
            }
        } else {
            self.insert_file(rel, mtime);
        }
    }

    fn all_paths(&self) -> impl Iterator<Item = &String> {
        self.notes.keys().chain(self.files.keys())
    }

    fn insert_note(&mut self, note: Note) {
        let path = note.path.clone();
        if self.notes.insert(path.clone(), note).is_none() {
            self.add_names(&path);
        }
    }

    fn insert_file(&mut self, path: &str, mtime: i64) {
        if self.files.insert(path.to_string(), mtime).is_none() {
            self.add_names(path);
        }
    }

    fn add_names(&mut self, path: &str) {
        self.lower_paths.insert(path.to_lowercase(), path.to_string());
        self.by_name
            .entry(vault::file_name(path).to_lowercase())
            .or_default()
            .push(path.to_string());
        if vault::is_note(path) {
            self.by_stem.entry(vault::stem(path).to_lowercase()).or_default().push(path.to_string());
        }
    }

    fn remove_names(&mut self, path: &str) {
        self.lower_paths.remove(&path.to_lowercase());
        remove_from(&mut self.by_name, &vault::file_name(path).to_lowercase(), path);
        if vault::is_note(path) {
            remove_from(&mut self.by_stem, &vault::stem(path).to_lowercase(), path);
        }
    }

    pub fn note(&self, path: &str) -> Option<&Note> {
        self.notes.get(path)
    }

    pub fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    /// Resolves a link target the way Obsidian does: path relative to the
    /// linking note, exact vault path, then by file name (preferring the same
    /// folder, then the shortest path).
    pub fn resolve(&self, target: &str, from: &str) -> Option<String> {
        let target = target.trim().replace('\\', "/");
        if target.is_empty() {
            return self.notes.contains_key(from).then(|| from.to_string());
        }
        let lower = target.to_lowercase();
        let lower = lower.trim_start_matches('/');
        let dir = vault::parent(from);

        if let Some(joined) = join_relative(&dir.to_lowercase(), lower) {
            if let Some(p) = self.lookup_exact(&joined) {
                return Some(p);
            }
        }
        if let Some(p) = self.lookup_exact(lower) {
            return Some(p);
        }

        let name = lower.rsplit('/').next().unwrap_or(lower);
        let mut candidates: Vec<&String> = Vec::new();
        candidates.extend(self.by_stem.get(name).into_iter().flatten());
        candidates.extend(self.by_name.get(name).into_iter().flatten());
        if lower.contains('/') {
            let suffix = format!("/{lower}");
            candidates.retain(|p| {
                let pl = p.to_lowercase();
                pl.ends_with(&suffix) || vault::strip_note_ext(&pl).ends_with(&suffix)
            });
        }
        candidates.sort_by_key(|p| (vault::parent(p) != dir, p.matches('/').count(), p.to_lowercase()));
        candidates.first().map(|p| (*p).clone())
    }

    fn lookup_exact(&self, lower: &str) -> Option<String> {
        self.lower_paths
            .get(lower)
            .or_else(|| self.lower_paths.get(&format!("{lower}.md")))
            .cloned()
    }

    /// Shortest unambiguous link text for a path: the bare name when it is
    /// unique in the vault, otherwise the full path (without ".md").
    pub fn link_text_for(&self, path: &str) -> String {
        let note = vault::is_note(path);
        let name = if note { vault::stem(path) } else { vault::file_name(path) };
        let map = if note { &self.by_stem } else { &self.by_name };
        let unique = map.get(&name.to_lowercase()).is_none_or(|v| v.len() <= 1);
        if unique { name.to_string() } else { vault::strip_note_ext(path).to_string() }
    }

    pub fn all_notes(&self) -> Vec<NoteRef> {
        let mut out: Vec<NoteRef> = self
            .notes
            .values()
            .map(|n| NoteRef {
                path: n.path.clone(),
                title: n.title.clone(),
                link_text: self.link_text_for(&n.path),
            })
            .collect();
        out.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
        out
    }

    pub fn note_info(&self, path: &str) -> Option<NoteInfo> {
        let n = self.notes.get(path)?;
        let outgoing = n
            .parsed
            .links
            .iter()
            .map(|l| OutLink {
                target: l.target.clone(),
                heading: l.heading.clone(),
                resolved: self.resolve(&l.target, path),
                line: l.line,
                embed: l.embed,
            })
            .collect();
        Some(NoteInfo {
            path: path.to_string(),
            title: n.title.clone(),
            aliases: n.parsed.aliases.clone(),
            tags: n.parsed.tags.clone(),
            headings: n.parsed.headings.clone(),
            backlinks: self.backlinks(path),
            outgoing,
            word_count: n.parsed.word_count,
            mtime: n.mtime,
            properties: n.parsed.properties.clone(),
            tasks_total: n.parsed.tasks.iter().filter(|t| !t.text.is_empty()).count(),
            tasks_done: n.parsed.tasks.iter().filter(|t| t.done && !t.text.is_empty()).count(),
        })
    }

    /// All notes, most recently modified first.
    pub fn summaries(&self) -> Vec<NoteSummary> {
        let mut out: Vec<NoteSummary> = self
            .notes
            .values()
            .map(|n| NoteSummary {
                path: n.path.clone(),
                title: n.title.clone(),
                mtime: n.mtime,
                excerpt: note_preview(&n.content, 180),
                tags: n.parsed.tags.clone(),
                open_tasks: n.parsed.tasks.iter().filter(|t| !t.done && !t.text.is_empty()).count(),
            })
            .collect();
        out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
        out
    }

    pub fn backlinks(&self, path: &str) -> Vec<Backlink> {
        let mut out: Vec<Backlink> = self
            .notes
            .par_iter()
            .filter(|(src, _)| src.as_str() != path)
            .flat_map_iter(|(src, note)| {
                note.parsed
                    .links
                    .iter()
                    .filter(|l| self.resolve(&l.target, src).as_deref() == Some(path))
                    .map(|l| Backlink {
                        source: src.clone(),
                        title: note.title.clone(),
                        line: l.line,
                        context: note.content.lines().nth(l.line).map(|s| clip(s.trim(), 240)).unwrap_or_default(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        out.sort_by(|a, b| a.title.cmp(&b.title).then(a.line.cmp(&b.line)));
        out
    }

    pub fn tasks(&self, include_done: bool) -> Vec<TaskItem> {
        let mut out: Vec<TaskItem> = self
            .notes
            .values()
            .flat_map(|n| {
                // Empty checkboxes are placeholders (e.g. in a daily note template).
                n.parsed.tasks.iter().filter(|t| (include_done || !t.done) && !t.text.is_empty()).map(|t| TaskItem {
                    path: n.path.clone(),
                    note_title: n.title.clone(),
                    task: t.clone(),
                })
            })
            .collect();
        out.sort_by(|a, b| {
            let key = |t: &TaskItem| (t.task.done, t.task.due.is_none(), t.task.due.clone(), t.task.priority);
            key(a).cmp(&key(b)).then_with(|| a.path.cmp(&b.path)).then(a.task.line.cmp(&b.task.line))
        });
        out
    }

    /// Reminders of the vault that are not done, soonest first.
    pub fn reminders(&self) -> Vec<ReminderItem> {
        let mut out: Vec<ReminderItem> = self
            .notes
            .values()
            .flat_map(|n| {
                n.parsed.reminders.iter().filter(|r| !r.done).map(|r| ReminderItem {
                    path: n.path.clone(),
                    note_title: n.title.clone(),
                    reminder: r.clone(),
                })
            })
            .collect();
        out.sort_by(|a, b| a.reminder.at.cmp(&b.reminder.at).then_with(|| a.path.cmp(&b.path)));
        out
    }

    pub fn tags(&self) -> Vec<TagCount> {
        let mut counts: HashMap<String, TagCount> = HashMap::new();
        for n in self.notes.values() {
            for tag in &n.parsed.tags {
                counts
                    .entry(tag.to_lowercase())
                    .or_insert_with(|| TagCount { tag: tag.clone(), count: 0 })
                    .count += 1;
            }
        }
        let mut out: Vec<TagCount> = counts.into_values().collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
        out
    }

    /// Full-text search. Every word must occur in the note (content, title or
    /// path); words starting with '#' filter by tag (nested tags included).
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        let mut tags = Vec::new();
        let mut terms = Vec::new();
        for w in q.split_whitespace() {
            match w.strip_prefix('#') {
                Some(t) if !t.is_empty() => tags.push(t),
                Some(_) => {}
                None => terms.push(w),
            }
        }
        if tags.is_empty() && terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = self
            .notes
            .par_iter()
            .filter_map(|(path, n)| {
                let has_tags = tags.iter().all(|t| {
                    let nested = format!("{t}/");
                    n.parsed.tags.iter().any(|nt| {
                        let nt = nt.to_lowercase();
                        nt == *t || nt.starts_with(&nested)
                    })
                });
                if !has_tags {
                    return None;
                }
                let title = n.title.to_lowercase();
                let path_l = path.to_lowercase();
                let mut score = tags.len() as i64;
                for t in &terms {
                    let in_title = title.contains(t) || path_l.contains(t);
                    let count = n.lower.matches(t).count();
                    if !in_title && count == 0 {
                        return None;
                    }
                    score += if in_title { 20 } else { 0 } + count.min(20) as i64;
                }
                Some(SearchHit {
                    path: path.clone(),
                    title: n.title.clone(),
                    score,
                    snippets: snippets(&n.content, &terms, 3),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        hits.truncate(limit);
        hits
    }

    /// Fuzzy note lookup by path, title and aliases. An empty query returns
    /// the most recently modified notes.
    pub fn quick_switch(&self, query: &str, limit: usize) -> Vec<SwitchHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            let mut recent: Vec<&Note> = self.notes.values().collect();
            recent.sort_by(|a, b| b.mtime.cmp(&a.mtime));
            return recent
                .into_iter()
                .take(limit)
                .map(|n| SwitchHit { path: n.path.clone(), title: n.title.clone(), score: 0 })
                .collect();
        }
        let mut hits: Vec<SwitchHit> = self
            .notes
            .values()
            .filter_map(|n| {
                let mut best = fuzzy::score(&q, vault::strip_note_ext(&n.path));
                for alt in n.parsed.aliases.iter().chain(std::iter::once(&n.title)) {
                    if let Some(s) = fuzzy::score(&q, alt) {
                        best = Some(best.map_or(s, |b| b.max(s)));
                    }
                }
                let mut score = best?;
                let stem = vault::stem(&n.path).to_lowercase();
                if stem == q {
                    score += 100;
                } else if stem.starts_with(&q) {
                    score += 50;
                } else if stem.contains(&q) {
                    score += 25;
                }
                Some(SwitchHit { path: n.path.clone(), title: n.title.clone(), score })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.len().cmp(&b.path.len())));
        hits.truncate(limit);
        hits
    }
}

fn load_note(vault: &Vault, rel: &str, mtime: i64) -> Option<Note> {
    let content = vault.read_string(rel).ok()?;
    Some(Note::new(rel.to_string(), content, mtime))
}

fn remove_from(map: &mut HashMap<String, Vec<String>>, key: &str, path: &str) {
    if let Some(v) = map.get_mut(key) {
        v.retain(|p| p != path);
        if v.is_empty() {
            map.remove(key);
        }
    }
}

/// Joins a lowercase link target to the linking note's folder, handling
/// "./" and "../". Returns `None` when there is nothing relative to do.
fn join_relative(dir: &str, target: &str) -> Option<String> {
    let explicit = target.starts_with("./") || target.starts_with("../");
    if dir.is_empty() && !explicit {
        return None;
    }
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn snippets(content: &str, terms: &[&str], max: usize) -> Vec<Snippet> {
    let mut out = Vec::new();
    if terms.is_empty() {
        return out;
    }
    for (i, line) in content.lines().enumerate() {
        let lower = line.to_lowercase();
        if let Some(pos) = terms.iter().filter_map(|t| lower.find(t)).min() {
            out.push(Snippet { line: i, text: excerpt(line, lower[..pos].chars().count(), 160) });
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

fn excerpt(line: &str, at_char: usize, width: usize) -> String {
    let chars: Vec<char> = line.trim_end().chars().collect();
    if chars.len() <= width {
        return chars.iter().collect::<String>().trim().to_string();
    }
    let start = at_char.saturating_sub(width / 3).min(chars.len() - width);
    let end = (start + width).min(chars.len());
    let mut s = String::new();
    if start > 0 {
        s.push('…');
    }
    s.extend(&chars[start..end]);
    if end < chars.len() {
        s.push('…');
    }
    s
}

/// Plain-text preview of a note: skips frontmatter, headings, code and empty
/// lines, strips list/quote/task markers and link brackets.
fn note_preview(content: &str, max_chars: usize) -> String {
    let mut lines = content.lines().peekable();
    if lines.peek().map(|l| l.trim_end()) == Some("---") {
        lines.next();
        for l in lines.by_ref() {
            if matches!(l.trim_end(), "---" | "...") {
                break;
            }
        }
    }
    let mut out = String::new();
    let mut in_code = false;
    for line in lines {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() || t.starts_with('#') || t.starts_with('|') || t == "---" {
            continue;
        }
        let mut t = t;
        for prefix in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "- ", "* ", "+ ", "> "] {
            if let Some(rest) = t.strip_prefix(prefix) {
                t = rest;
                break;
            }
        }
        let t = WIKILINK.replace_all(t, |c: &regex::Captures| {
            c.get(4).or(c.get(2)).map(|m| m.as_str().trim().to_string()).unwrap_or_default()
        });
        let t = t.replace(['*', '`'], "");
        if t.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(t.trim());
        if out.chars().count() >= max_chars {
            break;
        }
    }
    clip(&out, max_chars)
}

fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(notes: &[(&str, &str)]) -> Index {
        let mut idx = Index::default();
        for (p, c) in notes {
            idx.insert_content(p, c.to_string(), 0);
        }
        idx.insert_file("assets/pic.png", 0);
        idx
    }

    #[test]
    fn resolves_links() {
        let idx = index(&[
            ("Home.md", "[[Ideas]] [[Projects/Bedrock]] ![[pic.png]]"),
            ("Ideas.md", "back to [[home]]"),
            ("Projects/Bedrock.md", "# Bedrock\n[[Plan]] - [ ] ship it 📅 2026-11-01"),
            ("Projects/Plan.md", "plan"),
            ("Archive/Plan.md", "old plan #old"),
        ]);
        assert_eq!(idx.resolve("Ideas", "Home.md").as_deref(), Some("Ideas.md"));
        assert_eq!(idx.resolve("home", "Ideas.md").as_deref(), Some("Home.md"));
        assert_eq!(idx.resolve("Projects/Bedrock", "Home.md").as_deref(), Some("Projects/Bedrock.md"));
        assert_eq!(idx.resolve("pic.png", "Home.md").as_deref(), Some("assets/pic.png"));
        // Same folder wins when names collide.
        assert_eq!(idx.resolve("Plan", "Projects/Bedrock.md").as_deref(), Some("Projects/Plan.md"));
        assert_eq!(idx.resolve("Archive/Plan", "Home.md").as_deref(), Some("Archive/Plan.md"));
        assert_eq!(idx.resolve("Missing", "Home.md"), None);

        assert_eq!(idx.link_text_for("Ideas.md"), "Ideas");
        assert_eq!(idx.link_text_for("Archive/Plan.md"), "Archive/Plan");

        let back = idx.backlinks("Home.md");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].source, "Ideas.md");
    }

    #[test]
    fn search_tasks_tags() {
        let idx = index(&[
            ("A.md", "Rust is fast #lang/rust\n- [ ] learn traits 📅 2026-11-02"),
            ("B.md", "# Rust notes\nnothing else\n- [x] done task"),
            ("C.md", "Go is fine #lang"),
        ]);
        let hits = idx.search("rust", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "B.md"); // title match ranks first
        assert_eq!(idx.search("#lang", 10).len(), 2);
        assert_eq!(idx.search("#lang fast", 10).len(), 1);

        assert_eq!(idx.tasks(false).len(), 1);
        assert_eq!(idx.tasks(true).len(), 2);
        assert_eq!(idx.tags().len(), 2);

        let sw = idx.quick_switch("b", 5);
        assert_eq!(sw[0].path, "B.md");
    }

    #[test]
    fn summaries_have_excerpts() {
        let idx = index(&[(
            "Note.md",
            "---\nstatus: draft\n---\n# Title\n\n- [ ] see [[Other|the other]] and **bold**\n```\ncode\n```\nTail",
        )]);
        let s = idx.summaries();
        assert_eq!(s[0].excerpt, "see the other and bold Tail");
        assert_eq!(s[0].open_tasks, 1);
        let info = idx.note_info("Note.md").unwrap();
        assert_eq!(info.properties[0].key, "status");
        assert_eq!((info.tasks_total, info.tasks_done), (1, 0));
    }
}
