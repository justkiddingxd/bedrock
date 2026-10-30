//! Extracts structure from a markdown note: frontmatter, headings, wikilinks,
//! markdown links, tags, tasks and reminders. Fenced code blocks and inline
//! code are skipped. Line numbers are 0-based.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

pub(crate) static WIKILINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!)?\[\[([^\[\]\|#\n]*)(?:#([^\[\]\|\n]*))?(?:\|([^\[\]\n]*))?\]\]").unwrap()
});
static MDLINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(!)?\[([^\]\n]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());
pub(crate) static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s(,;])#([\p{L}\p{N}_/\-]+)").unwrap());
pub(crate) static TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)(?:[-*+]|\d+[.)])\s+\[(.)\](?:\s(.*))?$").unwrap());
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static DUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:📅\x{FE0F}?\s*|due:)(\d{4}-\d{2}-\d{2})").unwrap());
static SCHEDULED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"⏳\x{FE0F}?\s*(\d{4}-\d{2}-\d{2})").unwrap());
pub(crate) static DONE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*✅\x{FE0F}?\s*(\d{4}-\d{2}-\d{2})").unwrap());
/// Reminder marker, as the Obsidian Reminder plugin writes it: "⏰ 2026-10-30 15:00".
pub(crate) static REMINDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*⏰\x{FE0F}?\s*(\d{4}-\d{2}-\d{2})[ T](\d{1,2}):(\d{2})").unwrap());
static LIST_MARK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:(?:[-*+]|\d+[.)])\s+(?:\[.\]\s+)?|#{1,6}\s+|>\s*)").unwrap());
static TASK_META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*(?:⏰\x{FE0F}?\s*\d{4}-\d{2}-\d{2}[ T]\d{1,2}:\d{2}|(?:📅|⏳|✅|🛫|➕)\x{FE0F}?\s*\d{4}-\d{2}-\d{2}|due:\d{4}-\d{2}-\d{2}|[⏫🔼🔽🔺⏬]\x{FE0F}?)")
        .unwrap()
});

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// Link target as written: "Note", "Folder/Note", "image.png", "Note.md".
    pub target: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
    pub embed: bool,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

/// Ordered so that sorting ascending puts the most important first.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Medium,
    #[default]
    Normal,
    Low,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub line: usize,
    pub indent: usize,
    /// The character between the brackets: ' ', 'x', '/', '-', ...
    pub status: char,
    pub done: bool,
    /// Task text without dates and priority markers.
    pub text: String,
    pub due: Option<String>,
    pub scheduled: Option<String>,
    pub done_date: Option<String>,
    /// Reminder time on the line, "YYYY-MM-DD HH:MM".
    pub reminder: Option<String>,
    pub priority: Priority,
    pub tags: Vec<String>,
}

/// A line with a reminder marker. `at` is local time, "YYYY-MM-DD HH:MM".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub line: usize,
    pub at: String,
    /// The line without list markers, dates and the marker itself.
    pub text: String,
    /// The line is a completed task: no need to remind.
    pub done: bool,
}

/// A scalar frontmatter field other than title, tags and aliases.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parsed {
    /// Title from frontmatter or the first H1.
    pub title: Option<String>,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub properties: Vec<Property>,
    pub links: Vec<Link>,
    pub headings: Vec<Heading>,
    pub tasks: Vec<Task>,
    pub reminders: Vec<Reminder>,
    pub word_count: usize,
}

pub fn parse(content: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut tags = TagSet::default();
    let lines: Vec<&str> = content.lines().collect();

    let mut start = 0;
    if lines.first().map(|l| l.trim_end()) == Some("---") {
        if let Some(end) = lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")) {
            let close = end + 1;
            parse_frontmatter(&lines[1..close], &mut out, &mut tags);
            start = close + 1;
        }
    }

    let mut fence: Option<(char, usize)> = None;
    for (i, raw) in lines.iter().enumerate().skip(start) {
        let trimmed = raw.trim_start();
        if let Some((ch, len)) = fence {
            if is_fence_close(trimmed, ch, len) {
                fence = None;
            }
            continue;
        }
        if let Some(f) = fence_open(trimmed) {
            fence = Some(f);
            continue;
        }

        out.word_count += raw.split_whitespace().count();

        if let Some(c) = HEADING.captures(raw) {
            let level = c[1].len() as u8;
            let text = c[2].to_string();
            if level == 1 && out.title.is_none() {
                out.title = Some(text.clone());
            }
            out.headings.push(Heading { level, text, line: i });
        }

        let line = INLINE_CODE.replace_all(raw, |c: &regex::Captures| " ".repeat(c[0].len()));

        for c in WIKILINK.captures_iter(&line) {
            let target = c[2].trim().to_string();
            let heading = c.get(3).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty());
            if target.is_empty() && heading.is_none() {
                continue;
            }
            out.links.push(Link {
                target,
                heading,
                alias: c.get(4).map(|m| m.as_str().trim().to_string()),
                embed: c.get(1).is_some(),
                line: i,
            });
        }

        for c in MDLINK.captures_iter(&line) {
            let dest = &c[3];
            if !is_local_dest(dest) {
                continue;
            }
            let (target, heading) = split_fragment(&percent_decode(dest));
            if target.is_empty() {
                continue;
            }
            out.links.push(Link {
                target,
                heading,
                alias: Some(c[2].to_string()).filter(|s| !s.is_empty()),
                embed: c.get(1).is_some(),
                line: i,
            });
        }

        for tag in tags_in(&line) {
            tags.add(tag, &mut out.tags);
        }

        let task = parse_task(i, raw);
        if let Some(r) = parse_reminder(i, raw, task.as_ref()) {
            out.reminders.push(r);
        }
        if let Some(task) = task {
            out.tasks.push(task);
        }
    }
    out
}

/// Finds a reminder marker on a line. `task` is the line's task, if any.
pub fn parse_reminder(line: usize, raw: &str, task: Option<&Task>) -> Option<Reminder> {
    let c = REMINDER.captures(raw)?;
    let (hour, minute): (u32, u32) = (c[2].parse().ok()?, c[3].parse().ok()?);
    if hour > 23 || minute > 59 {
        return None;
    }
    let text = match task {
        Some(t) => t.text.clone(),
        None => TASK_META.replace_all(&LIST_MARK.replace(raw, ""), "").trim().to_string(),
    };
    Some(Reminder { line, at: format!("{} {hour:02}:{minute:02}", &c[1]), text, done: task.is_some_and(|t| t.done) })
}

/// Parses a task line like "- [ ] Call Bob 📅 2026-11-01 ⏫ #work".
pub fn parse_task(line: usize, raw: &str) -> Option<Task> {
    let c = TASK.captures(raw)?;
    let indent = c[1].chars().map(|ch| if ch == '\t' { 4 } else { 1 }).sum();
    let status = c[2].chars().next()?;
    let body = c.get(3).map(|m| m.as_str()).unwrap_or("");
    let priority = if body.contains('⏫') || body.contains('🔺') {
        Priority::High
    } else if body.contains('🔼') {
        Priority::Medium
    } else if body.contains('🔽') || body.contains('⏬') {
        Priority::Low
    } else {
        Priority::Normal
    };
    Some(Task {
        line,
        indent,
        status,
        done: matches!(status, 'x' | 'X' | '-'),
        text: TASK_META.replace_all(body, "").trim().to_string(),
        due: DUE.captures(body).map(|c| c[1].to_string()),
        scheduled: SCHEDULED.captures(body).map(|c| c[1].to_string()),
        done_date: DONE_DATE.captures(body).map(|c| c[1].to_string()),
        reminder: REMINDER.captures(body).map(|c| format!("{} {:0>2}:{}", &c[1], &c[2], &c[3])),
        priority,
        tags: tags_in(&INLINE_CODE.replace_all(body, "")).map(str::to_string).collect(),
    })
}

/// Tags in a single line (code spans must already be removed).
pub(crate) fn tags_in(line: &str) -> impl Iterator<Item = &str> {
    TAG.captures_iter(line)
        .map(|c| c.get(1).unwrap().as_str().trim_end_matches(['/', '-']))
        .filter(|t| is_valid_tag(t))
}

/// Obsidian rule: a tag must contain at least one non-numeric character.
pub(crate) fn is_valid_tag(tag: &str) -> bool {
    !tag.is_empty() && !tag.chars().all(|c| c.is_numeric() || c == '/' || c == '-' || c == '_')
}

#[derive(Default)]
struct TagSet(HashSet<String>);

impl TagSet {
    fn add(&mut self, tag: &str, out: &mut Vec<String>) {
        let tag = tag.trim().trim_start_matches('#');
        if is_valid_tag(tag) && self.0.insert(tag.to_lowercase()) {
            out.push(tag.to_string());
        }
    }
}

fn parse_frontmatter(lines: &[&str], out: &mut Parsed, tags: &mut TagSet) {
    let mut list_key: Option<String> = None;
    for line in lines {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(item) = t.strip_prefix('-') {
            if let Some(key) = &list_key {
                add_fm_value(key, &unquote(item.trim()), out, tags);
            }
            continue;
        }
        let Some((k, v)) = t.split_once(':') else { continue };
        let key = k.trim().to_lowercase();
        let v = v.trim();
        list_key = None;
        match key.as_str() {
            "title" if !v.is_empty() => out.title = Some(unquote(v)),
            "tags" | "tag" | "aliases" | "alias" => {
                if v.is_empty() {
                    list_key = Some(key);
                } else {
                    let inner = v.trim_start_matches('[').trim_end_matches(']');
                    let sep: &[char] = if inner.contains(',') { &[','] } else { &[' '] };
                    for item in inner.split(sep) {
                        add_fm_value(&key, &unquote(item.trim()), out, tags);
                    }
                }
            }
            _ if !v.is_empty() && !line.starts_with(char::is_whitespace) => {
                out.properties.push(Property { key: k.trim().to_string(), value: unquote(v) });
            }
            _ => {}
        }
    }
}

fn add_fm_value(key: &str, value: &str, out: &mut Parsed, tags: &mut TagSet) {
    if value.is_empty() {
        return;
    }
    if key.starts_with("tag") {
        tags.add(value, &mut out.tags);
    } else {
        out.aliases.push(value.to_string());
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let quoted = s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')));
    if quoted { s[1..s.len() - 1].to_string() } else { s.to_string() }
}

fn fence_open(trimmed: &str) -> Option<(char, usize)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|&c| c == ch).count();
    (len >= 3).then_some((ch, len))
}

fn is_fence_close(trimmed: &str, ch: char, len: usize) -> bool {
    let n = trimmed.chars().take_while(|&c| c == ch).count();
    n >= len && trimmed[n * ch.len_utf8()..].trim().is_empty()
}

pub(crate) fn is_local_dest(dest: &str) -> bool {
    !(dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with("data:") || dest.starts_with('#'))
}

/// "Note.md#Heading" -> ("Note.md", Some("Heading"))
pub(crate) fn split_fragment(dest: &str) -> (String, Option<String>) {
    match dest.split_once('#') {
        Some((t, h)) => (t.to_string(), Some(h.to_string()).filter(|h| !h.is_empty())),
        None => (dest.to_string(), None),
    }
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = |b: u8| (b as char).to_digit(16).unwrap() as u8;
            out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = concat!(
        "---\n",
        "title: \"My note\"\n",
        "Статус: черновик\n",
        "tags: [work, idea]\n",
        "aliases:\n",
        "  - Моя заметка\n",
        "---\n",
        "# Heading one\n",
        "Text with [[Other note]] and [[Folder/Deep#Section|alias]] and ![[pic.png]].\n",
        "A [md link](Some%20Note.md#part) and [web](https://example.com).\n",
        "Tags: #project/bedrock #задачи #123 `#notatag` [[#local]]\n",
        "```\n",
        "#code [[not a link]]\n",
        "- [ ] not a task\n",
        "```\n",
        "- [ ] Call Bob 📅 2026-11-01 ⏫ #work\n",
        "  - [x] Done thing ✅ 2026-10-28\n",
        "1. [ ] numbered\n",
    );

    #[test]
    fn parses_everything() {
        let p = parse(NOTE);
        assert_eq!(p.title.as_deref(), Some("My note"));
        assert_eq!(p.aliases, vec!["Моя заметка"]);
        assert_eq!(p.properties, vec![Property { key: "Статус".into(), value: "черновик".into() }]);
        assert_eq!(p.tags, vec!["work", "idea", "project/bedrock", "задачи"]);
        assert_eq!(p.headings.len(), 1);

        let targets: Vec<_> = p.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, vec!["Other note", "Folder/Deep", "pic.png", "Some Note.md", ""]);
        assert_eq!(p.links[1].heading.as_deref(), Some("Section"));
        assert_eq!(p.links[1].alias.as_deref(), Some("alias"));
        assert!(p.links[2].embed);
        assert_eq!(p.links[3].heading.as_deref(), Some("part"));
        assert_eq!(p.links[4].heading.as_deref(), Some("local"));

        assert_eq!(p.tasks.len(), 3);
        let t = &p.tasks[0];
        assert_eq!(t.text, "Call Bob #work");
        assert_eq!(t.due.as_deref(), Some("2026-11-01"));
        assert_eq!(t.priority, Priority::High);
        assert_eq!(t.tags, vec!["work"]);
        assert!(!t.done);
        assert!(p.tasks[1].done);
        assert_eq!(p.tasks[1].indent, 2);
        assert_eq!(p.tasks[1].done_date.as_deref(), Some("2026-10-28"));
        assert_eq!(p.tasks[2].text, "numbered");
    }

    #[test]
    fn reminders() {
        let p = parse(concat!(
            "Позвонить маме ⏰ 2026-10-30 9:05\n",
            "- [ ] Купить хлеб 📅 2026-10-30 ⏰ 2026-10-30 18:00\n",
            "- [x] Отправить счёт ⏰ 2026-10-29 10:00 ✅ 2026-10-29\n",
            "Не напоминание ⏰ 2026-10-30 25:00\n",
        ));
        assert_eq!(p.reminders.len(), 3);
        assert_eq!(p.reminders[0], Reminder { line: 0, at: "2026-10-30 09:05".into(), text: "Позвонить маме".into(), done: false });
        assert_eq!(p.reminders[1].text, "Купить хлеб");
        assert_eq!(p.tasks[0].text, "Купить хлеб");
        assert_eq!(p.tasks[0].reminder.as_deref(), Some("2026-10-30 18:00"));
        assert!(p.reminders[2].done);
    }

    #[test]
    fn h1_is_title_without_frontmatter() {
        let p = parse("intro\n# Real title\n## Sub\n");
        assert_eq!(p.title.as_deref(), Some("Real title"));
        assert_eq!(p.headings.len(), 2);
    }
}
