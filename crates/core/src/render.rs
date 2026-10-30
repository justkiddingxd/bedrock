//! Markdown -> sanitized HTML for the preview pane. Wikilinks, tags and local
//! links become `<a data-target>` / `<a data-tag>` elements the UI handles;
//! local images become `<img data-src>` that the UI maps to the asset protocol.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::index::Index;
use crate::parse::{self, WIKILINK};
use crate::vault;

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
            events.push(Event::Text(s[last..hash].to_string().into()));
        }
        events.push(Event::InlineHtml(
            format!(r##"<a class="tag" href="#" data-tag="{t}">#{t}</a>"##, t = esc(tag)).into(),
        ));
        last = m.start() + tag.len();
    }
    if last < s.len() {
        events.push(Event::Text(s[last..].to_string().into()));
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
}
