//! Markdown -> sanitized HTML for the preview pane. Wikilinks, tags and local
//! links become `<a data-target>` / `<a data-tag>` elements the UI handles;
//! local images become `<img data-src>` that the UI maps to the asset protocol.

use std::sync::LazyLock;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::{Captures, Regex};

use crate::index::Index;
use crate::parse::{self, WIKILINK};
use crate::vault;

/// Task marks in text (due, scheduled and done dates, reminder, priority, repeat rule).
static MARK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:(📅|⏳|✅)\x{FE0F}?\s*(\d{4}-\d{2}-\d{2}))|(?:⏰\x{FE0F}?\s*(\d{4}-\d{2}-\d{2}[ T]\d{1,2}:\d{2}))|([⏫🔺🔼🔽⏬])\x{FE0F}?|(?:🔁\x{FE0F}?\s*([^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n]*[^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n\s]))").unwrap()
});

pub fn render(content: &str, index: &Index, from: &str) -> String {
    let (body, body_offset) = split_frontmatter(content);
    let line_starts: Vec<usize> =
        std::iter::once(0).chain(content.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |offset: usize| line_starts.partition_point(|&s| s <= offset) - 1;

    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;

    let mut events: Vec<Event> = Vec::new();
    let mut text = String::new();
    let mut in_code_block = false;
    let mut link_stack: Vec<bool> = Vec::new();
    let mut image: Option<(String, String)> = None;

    for (ev, range) in Parser::new_ext(body, opts).into_offset_iter() {
        // Collect the alt text of a local image until its end tag.
        if let Some((_, alt)) = image.as_mut() {
            match ev {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = image.take().unwrap();
                    events.push(Event::InlineHtml(img_html(&src, &alt).into()));
                }
                _ => {}
            }
            continue;
        }
        if let Event::Text(t) = &ev {
            if !in_code_block {
                text.push_str(t);
                continue;
            }
        }
        flush_text(&mut text, &mut events, index, from);

        match ev {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                events.push(ev);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                events.push(ev);
            }
            Event::TaskListMarker(checked) => {
                let line = line_of(body_offset + range.start);
                let checked = if checked { " checked" } else { "" };
                events.push(Event::InlineHtml(
                    format!(r#"<input type="checkbox" class="task-checkbox" data-line="{line}"{checked}>"#).into(),
                ));
            }
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                if parse::is_local_dest(&dest_url) {
                    let (target, heading) = parse::split_fragment(&parse::percent_decode(&dest_url));
                    let resolved = index.resolve(&target, from).is_some();
                    events.push(Event::InlineHtml(link_open(&target, heading.as_deref(), resolved).into()));
                    link_stack.push(true);
                } else {
                    events.push(Event::Start(Tag::Link { link_type, dest_url, title, id }));
                    link_stack.push(false);
                }
            }
            Event::End(TagEnd::Link) => {
                if link_stack.pop() == Some(true) {
                    events.push(Event::InlineHtml("</a>".into()));
                } else {
                    events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                let local = parse::is_local_dest(&dest_url)
                    .then(|| index.resolve(&parse::percent_decode(&dest_url), from))
                    .flatten();
                match local {
                    Some(path) => image = Some((path, String::new())),
                    None => events.push(Event::Start(Tag::Image { link_type, dest_url, title, id })),
                }
            }
            other => events.push(other),
        }
    }
    flush_text(&mut text, &mut events, index, from);

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    sanitize(&html)
}

fn sanitize(html: &str) -> String {
    ammonia::Builder::default()
        .add_tags(&["input"])
        .add_tag_attributes("input", &["type", "checked", "disabled"])
        .add_generic_attributes(&["class"])
        .add_generic_attribute_prefixes(&["data-"])
        .clean(html)
        .to_string()
}

/// Turns plain text into events, replacing wikilinks and tags with HTML.
fn flush_text<'a>(buf: &mut String, events: &mut Vec<Event<'a>>, index: &Index, from: &str) {
    if buf.is_empty() {
        return;
    }
    let text = std::mem::take(buf);
    let mut last = 0;
    for c in WIKILINK.captures_iter(&text) {
        let whole = c.get(0).unwrap();
        push_tags(&text[last..whole.start()], events);
        let target = c[2].trim();
        let heading = c.get(3).map(|m| m.as_str().trim()).filter(|s| !s.is_empty());
        let alias = c.get(4).map(|m| m.as_str().trim()).filter(|s| !s.is_empty());
        let resolved = index.resolve(target, from);
        let html = match (&resolved, c.get(1).is_some()) {
            (Some(path), true) if vault::is_image(path) => img_html(path, alias.unwrap_or(target)),
            _ => {
                let label = match (alias, heading) {
                    (Some(a), _) => a.to_string(),
                    (None, Some(h)) if target.is_empty() => h.to_string(),
                    (None, Some(h)) => format!("{target} › {h}"),
                    (None, None) => target.to_string(),
                };
                format!("{}{}</a>", link_open(target, heading, resolved.is_some()), esc(&label))
            }
        };
        events.push(Event::InlineHtml(html.into()));
        last = whole.end();
    }
    push_tags(&text[last..], events);
}

fn push_tags<'a>(s: &str, events: &mut Vec<Event<'a>>) {
    let mut last = 0;
    for c in parse::TAG.captures_iter(s) {
        let m = c.get(1).unwrap();
        let tag = m.as_str().trim_end_matches(['/', '-']);
        if !parse::is_valid_tag(tag) {
            continue;
        }
        let hash = m.start() - 1;
        if hash > last {
            push_marks(&s[last..hash], events);
        }
        events.push(Event::InlineHtml(
            format!(r##"<a class="tag" href="#" data-tag="{t}">#{t}</a>"##, t = esc(tag)).into(),
        ));
        last = m.start() + tag.len();
    }
    if last < s.len() {
        push_marks(&s[last..], events);
    }
}

fn push_marks<'a>(s: &str, events: &mut Vec<Event<'a>>) {
    let mut last = 0;
    for c in MARK.captures_iter(s) {
        let whole = c.get(0).unwrap();
        if whole.start() > last {
            events.push(Event::Text(s[last..whole.start()].to_string().into()));
        }
        events.push(Event::InlineHtml(mark_html(&c).into()));
        last = whole.end();
    }
    if last < s.len() {
        events.push(Event::Text(s[last..].to_string().into()));
    }
}

/// A task mark, drawn as in the editor. The icon stays empty here, since
/// sanitized HTML has no SVG: the preview fills it in from `data-icon`.
fn mark_html(c: &Captures) -> String {
    let icon = |name: &str, class: &str, title: &str| format!(r#"<span class="task-mark {class}" data-icon="{name}" title="{title}"></span>"#);
    let value = |class: &str, text: &str| format!(r#"<span class="task-value {class}">{}</span>"#, esc(text));
    if let Some(kind) = c.get(1) {
        match kind.as_str() {
            "📅" => icon("calendar", "accent", "срок") + &value("", &c[2]),
            "⏳" => icon("clock", "", "запланировано") + &value("", &c[2]),
            _ => icon("check", "ok", "выполнено") + &value("faint", &c[2]),
        }
    } else if let Some(at) = c.get(3) {
        icon("bell", "accent", "напоминание") + &value("accent", at.as_str())
    } else if let Some(priority) = c.get(4) {
        match priority.as_str() {
            "⏫" | "🔺" => icon("flag", "high", "высокий приоритет"),
            "🔼" => icon("flag", "medium", "средний приоритет"),
            _ => icon("flag", "low", "низкий приоритет"),
        }
    } else {
        icon("repeat", "accent", "повторяется") + &value("", &c[5])
    }
}

fn link_open(target: &str, heading: Option<&str>, resolved: bool) -> String {
    let class = if resolved { "wikilink" } else { "wikilink unresolved" };
    format!(
        r##"<a class="{class}" href="#" data-target="{}" data-heading="{}">"##,
        esc(target),
        esc(heading.unwrap_or(""))
    )
}

fn img_html(path: &str, alt: &str) -> String {
    format!(r#"<img class="embed" data-src="{}" alt="{}">"#, esc(path), esc(alt))
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Returns the note body without YAML frontmatter and the body's byte offset.
fn split_frontmatter(content: &str) -> (&str, usize) {
    let mut lines = content.split_inclusive('\n');
    let Some(first) = lines.next() else { return (content, 0) };
    if first.trim_end() != "---" {
        return (content, 0);
    }
    let mut offset = first.len();
    for line in lines {
        offset += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            return (&content[offset..], offset);
        }
    }
    (content, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_links_tags_tasks() {
        let mut idx = Index::default();
        idx.insert_content("Other.md", "x".into(), 0);
        let src = "---\ntitle: t\n---\nSee [[Other]], [[Nope|missing]] and #tag.\n\n- [ ] todo\n- [x] done\n\n`[[code]]`\n\n<script>alert(1)</script>";
        let html = render(src, &idx, "Note.md");
        assert!(html.contains(r##"class="wikilink" href="#" data-target="Other""##), "{html}");
        assert!(html.contains(r#"class="wikilink unresolved""#));
        assert!(html.contains(">missing</a>"));
        assert!(html.contains(r#"data-tag="tag""#));
        assert!(html.contains(r#"data-line="5""#), "{html}");
        assert!(html.contains("checked"));
        assert!(html.contains("<code>[[code]]</code>"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("title: t"));
    }

    #[test]
    fn task_marks_become_icons() {
        let src = "- [ ] ПТ 📅 2026-10-31 ⏰ 2026-10-31 03:50 🔁 каждый день ⏫ #работа\n- [x] Бег ✅ 2026-11-01\n\nПочему `📅 2026-01-01` в коде\n\n🔁 <b>каждый</b>";
        let html = render(src, &Index::default(), "Note.md");
        for mark in ["calendar", "bell", "repeat", "flag", "check"] {
            assert!(html.contains(&format!(r#"data-icon="{mark}""#)), "{mark}: {html}");
        }
        assert!(html.contains(r#"<span class="task-value ">2026-10-31</span>"#), "{html}");
        assert!(html.contains(r#"<span class="task-value accent">2026-10-31 03:50</span>"#), "{html}");
        assert!(html.contains(r#"<span class="task-value ">каждый день</span>"#), "{html}");
        assert!(html.contains(r#"data-tag="работа""#), "{html}");
        // The emoji are gone from the text, but code stays as written.
        assert!(html.contains("<code>📅 2026-01-01</code>"), "{html}");
        assert_eq!(html.matches('📅').count(), 1, "{html}");
        assert!(!html.contains('⏰') && !html.contains('⏫') && !html.contains('✅'), "{html}");
    }
}
