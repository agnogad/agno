use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd, html};
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

/// Render Markdown to HTML. Returns (html, plain-text summary of the first paragraph).
pub fn render(input: &str, highlight: bool) -> (String, String) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);

    let mut events = Vec::new();
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
            other => events.push(other),
        }
    }

    let summary = first_paragraph(&events);
    let mut out = String::with_capacity(input.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    (out, summary)
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

    let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let syntax = syntaxes
        .find_syntax_by_token(&lang)
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let theme = THEME.get_or_init(|| {
        ThemeSet::load_defaults()
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_default()
    });
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = String::from("<pre class=\"code\"><code>");
    for line in LinesWithEndings::from(code) {
        match highlighter.highlight_line(line, syntaxes) {
            Ok(ranges) => match syntect::html::styled_line_to_highlighted_html(
                &ranges[..],
                syntect::html::IncludeBackground::No,
            ) {
                Ok(html) => out.push_str(&html),
                Err(_) => out.push_str(&escape_html(line)),
            },
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_gfm_and_summary() {
        let (html, summary) = render(
            "# Hi\n\nFirst **bold** paragraph.\n\n- [x] task\n\n|a|b|\n|-|-|\n|1|2|",
            false,
        );
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<table>"));
        assert!(summary.starts_with("First bold paragraph."));
    }

    #[test]
    fn highlights_fenced_code_when_enabled() {
        let (html, _) = render("```rust\nfn main() {}\n```", true);
        assert!(html.contains("<span style="));
        assert!(!html.contains("```"));
    }

    #[test]
    fn escapes_html_in_code_when_disabled() {
        let (html, _) = render("```\n<div>\n```", false);
        assert!(html.contains("&lt;div&gt;"));
    }
}
