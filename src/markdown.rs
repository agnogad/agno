use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd, html};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();
/// `find_syntax_by_token` is a linear scan over every bundled grammar, so the
/// answer is memoised per language: a site with thousands of code blocks would
/// otherwise pay for that scan on every fence.
static SYNTAX_CACHE: OnceLock<RwLock<HashMap<String, &'static SyntaxReference>>> = OnceLock::new();

fn syntaxes() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_for(lang: &str) -> &'static SyntaxReference {
    let cache = SYNTAX_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(found) = cache.read().ok().and_then(|map| map.get(lang).copied()) {
        return found;
    }
    let set = syntaxes();
    let syntax = set
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| set.find_syntax_plain_text());
    if let Ok(mut map) = cache.write() {
        map.insert(lang.to_string(), syntax);
    }
    syntax
}

/// One heading in the table of contents.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TocEntry {
    pub level: u8,
    pub id: String,
    pub title: String,
    pub children: Vec<TocEntry>,
}

pub struct Rendered {
    pub html: String,
    pub summary: String,
    pub toc: Vec<TocEntry>,
    /// The table of contents pre-rendered as nested `<ul>` lists.
    pub toc_html: String,
}

/// Render Markdown to HTML, collecting the summary and the table of contents.
pub fn render(input: &str, highlight: bool) -> Rendered {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let mut events = Vec::with_capacity(input.len() / 24 + 8);
    let mut headings: Vec<(u8, String, String)> = Vec::new();
    let mut used_ids: HashMap<String, usize> = HashMap::new();
    let mut parser = Parser::new_ext(input, opts);
    while let Some(event) = parser.next() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::Text(text) => code.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let highlighted = highlight_code(&code, &lang, highlight);
                events.push(Event::Html(CowStr::Boxed(highlighted.into_boxed_str())));
            }
            Event::Start(Tag::Heading {
                level,
                id,
                classes,
                attrs,
            }) => {
                let mut inner = Vec::new();
                for event in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::Heading(_)) => break,
                        other => inner.push(other),
                    }
                }
                let text = heading_text(&inner);
                let base = id
                    .map(|id| id.to_string())
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| crate::content::slugify(&text));
                let base = if base.is_empty() {
                    "section".to_string()
                } else {
                    base
                };
                let unique = unique_id(&mut used_ids, base);
                headings.push((level as u8, unique.clone(), text));
                events.push(Event::Start(Tag::Heading {
                    level,
                    id: Some(CowStr::Boxed(unique.into_boxed_str())),
                    classes,
                    attrs,
                }));
                events.extend(inner);
                events.push(Event::End(TagEnd::Heading(level)));
            }
            other => events.push(other),
        }
    }

    let summary = first_paragraph(&events);
    let mut html = String::with_capacity(input.len() * 3 / 2);
    html::push_html(&mut html, events.into_iter());
    let toc = build_toc(&headings);
    let toc_html = render_toc(&toc);
    Rendered {
        html,
        summary,
        toc,
        toc_html,
    }
}

fn heading_text(events: &[Event<'_>]) -> String {
    let mut text = String::new();
    for event in events {
        match event {
            Event::Text(t) | Event::Code(t) => text.push_str(t),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unique_id(used: &mut HashMap<String, usize>, base: String) -> String {
    if !used.contains_key(&base) {
        used.insert(base.clone(), 0);
        return base;
    }
    let mut counter = used[&base];
    loop {
        counter += 1;
        let candidate = format!("{base}-{counter}");
        if !used.contains_key(&candidate) {
            used.insert(base, counter);
            used.insert(candidate.clone(), 0);
            return candidate;
        }
    }
}

fn build_toc(flat: &[(u8, String, String)]) -> Vec<TocEntry> {
    fn children_at<'a>(
        roots: &'a mut Vec<TocEntry>,
        path: &[(usize, u8)],
    ) -> &'a mut Vec<TocEntry> {
        let mut current = roots;
        for &(index, _) in path {
            current = &mut current[index].children;
        }
        current
    }

    let mut roots: Vec<TocEntry> = Vec::new();
    let mut path: Vec<(usize, u8)> = Vec::new();
    for (level, id, title) in flat {
        while let Some(&(_, open_level)) = path.last() {
            if open_level >= *level {
                path.pop();
            } else {
                break;
            }
        }
        let siblings = children_at(&mut roots, &path);
        let index = siblings.len();
        siblings.push(TocEntry {
            level: *level,
            id: id.clone(),
            title: title.clone(),
            children: Vec::new(),
        });
        path.push((index, *level));
    }
    roots
}

fn render_toc(entries: &[TocEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul>");
    for entry in entries {
        out.push_str("<li><a href=\"#");
        out.push_str(&entry.id);
        out.push_str("\">");
        out.push_str(&escape_html(&entry.title));
        out.push_str("</a>");
        out.push_str(&render_toc(&entry.children));
        out.push_str("</li>");
    }
    out.push_str("</ul>");
    out
}

fn first_paragraph(events: &[Event<'_>]) -> String {
    let mut text = String::new();
    let mut capturing = false;
    for event in events {
        match event {
            Event::Start(Tag::Paragraph) if text.is_empty() => capturing = true,
            Event::End(TagEnd::Paragraph) if capturing => break,
            Event::Text(t) if capturing => text.push_str(t),
            Event::Code(t) if capturing => {
                text.push('`');
                text.push_str(t);
                text.push('`');
            }
            Event::SoftBreak | Event::HardBreak if capturing => text.push(' '),
            _ => {}
        }
    }
    truncate(&text, 200)
}

fn truncate(input: &str, max_chars: usize) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(max_chars).collect();
    out.push('…');
    out
}

fn highlight_code(code: &str, lang: &str, enabled: bool) -> String {
    let lang = sanitize_lang(lang);
    if !enabled || lang.is_empty() {
        let class = if lang.is_empty() {
            String::new()
        } else {
            format!(" class=\"language-{lang}\"")
        };
        return format!("<pre><code{class}>{}</code></pre>", escape_html(code));
    }

    let syntax = syntax_for(&lang);
    let theme = THEME.get_or_init(|| {
        ThemeSet::load_defaults()
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_default()
    });
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = String::with_capacity(code.len() * 4 + 64);
    out.push_str("<pre class=\"code\"><code>");
    for line in LinesWithEndings::from(code) {
        match highlighter.highlight_line(line, syntaxes()) {
            Ok(ranges) => {
                if syntect::html::append_highlighted_html_for_styled_line(
                    &ranges[..],
                    syntect::html::IncludeBackground::No,
                    &mut out,
                )
                .is_err()
                {
                    out.push_str(&escape_html(line));
                }
            }
            Err(_) => out.push_str(&escape_html(line)),
        }
    }
    out.push_str("</code></pre>");
    out
}

fn sanitize_lang(lang: &str) -> String {
    lang.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '#'))
        .collect()
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    push_escaped(&mut out, input);
    out
}

/// Append `input` to `out`, escaping the five HTML-significant characters.
pub fn push_escaped(out: &mut String, input: &str) {
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_gfm_and_summary() {
        let rendered = render(
            "# Hi\n\nFirst **bold** paragraph.\n\n- [x] task\n\n|a|b|\n|-|-|\n|1|2|",
            false,
        );
        assert!(rendered.html.contains("<strong>bold</strong>"));
        assert!(rendered.html.contains("<table>"));
        assert!(rendered.summary.starts_with("First bold paragraph."));
    }

    #[test]
    fn highlights_fenced_code_when_enabled() {
        let rendered = render("```rust\nfn main() {}\n```", true);
        assert!(rendered.html.contains("<span style="));
        assert!(!rendered.html.contains("```"));
    }

    #[test]
    fn escapes_html_in_code_when_disabled() {
        let rendered = render("```\n<div>\n```", false);
        assert!(rendered.html.contains("&lt;div&gt;"));
    }

    #[test]
    fn headings_get_stable_unique_anchors() {
        let rendered = render("# Intro\n\n## Setup\n\n## Setup\n\n### Setup\n", false);
        assert!(rendered.html.contains("<h1 id=\"intro\">"));
        assert!(rendered.html.contains("<h2 id=\"setup\">"));
        assert!(rendered.html.contains("<h2 id=\"setup-1\">"));
        assert!(rendered.html.contains("<h3 id=\"setup-2\">"));
    }

    #[test]
    fn explicit_heading_ids_win() {
        let rendered = render("## Install {#how}\n", false);
        assert!(rendered.html.contains("<h2 id=\"how\">"));
        assert!(rendered.html.contains("Install"));
    }

    #[test]
    fn toc_nests_by_heading_level() {
        let rendered = render("# A\n\n## B\n\n### C\n\n## D\n", false);
        assert_eq!(rendered.toc.len(), 1);
        let a = &rendered.toc[0];
        assert_eq!(a.title, "A");
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.children[0].title, "B");
        assert_eq!(a.children[0].children[0].title, "C");
        assert_eq!(a.children[1].title, "D");
        assert_eq!(
            rendered.toc_html,
            "<ul><li><a href=\"#a\">A</a><ul><li><a href=\"#b\">B</a><ul><li><a href=\"#c\">C</a></li></ul></li><li><a href=\"#d\">D</a></li></ul></li></ul>"
        );
    }
}
