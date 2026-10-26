//! `agno check`: static analysis of a site before it is deployed.
//!
//! The build already rejects duplicate output URLs; this command goes further
//! and resolves every internal link and image reference in the Markdown source
//! against the set of files the build will produce.

use crate::config::Config;
use crate::content::{self, Content, Drafts};
use crate::markdown;
use crate::render::Templates;
use crate::site;
use anyhow::Result;
use pulldown_cmark::{Event, Parser, Tag};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Default)]
pub struct Report {
    pub pages: usize,
    pub links: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Resolve every internal link, image, anchor and template reference against
/// the files the build will produce.
///
/// # Errors
///
/// Fails when the site cannot be loaded: a missing root, unreadable content or
/// invalid frontmatter. Broken links are findings in the returned [`Report`],
/// not errors — the caller decides what to do with them.
#[allow(clippy::too_many_lines)]
pub fn check(root: &Path, drafts: Drafts) -> Result<Report> {
    let cfg = Config::load(root)?;
    cfg.validate()?;
    let content = content::load(&cfg, drafts)?;
    let templates = Templates::load(&cfg.templates())?;
    let mut report = Report {
        pages: content.pages.len(),
        ..Report::default()
    };

    let static_paths = site::static_paths(&cfg)?;
    if let Err(err) = site::validate_outputs(&cfg, &content, &static_paths) {
        report.errors.push(err.to_string());
    }

    let known = match site::expected_outputs(&cfg, &content, &static_paths) {
        Ok(known) => known,
        Err(err) => {
            // A paginated URL that collides with another output is a build
            // error too; report it instead of guessing at the link set.
            report.errors.push(err.to_string());
            return Ok(report);
        }
    };

    // Everything whose Markdown body can carry links: regular pages, the home
    // page and section indexes. `_index.md` bodies were never scanned before,
    // so a broken link on a landing page went unreported.
    let bodies = bodies(&content);

    for page in &content.pages {
        if let Some(template) = page.template.as_deref()
            && !templates.has(template)
        {
            report.errors.push(format!(
                "{}: template {template} does not exist",
                page.source.display()
            ));
        }
        if page.title.trim().is_empty() {
            report
                .warnings
                .push(format!("{}: empty title", page.source.display()));
        }
        if page.date.is_none() {
            report.warnings.push(format!(
                "{}: no date, so the page is excluded from feeds and search",
                page.source.display()
            ));
        }
    }

    // The home page and section indexes take `template` too, so a typo there
    // would otherwise be a silent fallback to the default layout.
    let mut overrides: Vec<(&Path, &str)> = Vec::new();
    if let Some(template) = content.home.template.as_deref() {
        overrides.push((&content.home.source, template));
    }
    for section in &content.sections {
        if let Some(template) = section.template.as_deref() {
            overrides.push((&section.source, template));
        }
    }
    for (source, template) in overrides {
        if !templates.has(template) {
            report.errors.push(format!(
                "{}: template {template} does not exist",
                source.display()
            ));
        }
    }

    // Anchors are validated after the first pass: they need the heading ids of
    // the page they point at, and those pages are only rendered on demand.
    let mut anchors: Vec<(String, &Path, usize, String)> = Vec::new();

    for (source, url, body, body_line) in &bodies {
        for (line, target) in extract_links(body) {
            let line = line + body_line - 1;
            let (path, anchor) = split_anchor(&target);
            if path.is_empty() {
                if let Some(anchor) = anchor {
                    report.links += 1;
                    anchors.push((site::output_rel(url), source, line, anchor.to_string()));
                }
                continue;
            }
            let Some(key) = resolve_link(url, path) else {
                continue;
            };
            report.links += 1;
            if !known.contains(&key) {
                report.errors.push(format!(
                    "{}:{line}: broken link {target} (resolves to /{key})",
                    source.display()
                ));
            } else if let Some(anchor) = anchor {
                anchors.push((key, source, line, anchor.to_string()));
            }
        }
    }

    if !anchors.is_empty() {
        let mut ids: HashMap<String, HashSet<String>> = HashMap::new();
        {
            let wanted: HashSet<&str> = anchors.iter().map(|(key, ..)| key.as_str()).collect();
            for (_, url, body, _) in &bodies {
                let key = site::output_rel(url);
                if wanted.contains(key.as_str()) {
                    ids.insert(key, heading_ids(body));
                }
            }
        }
        for (key, source, line, anchor) in anchors {
            // `#top` is the one fragment browsers resolve without an element:
            // it scrolls to the start of the document, so it is never broken.
            if anchor.eq_ignore_ascii_case("top") {
                continue;
            }
            // A link into a non-Markdown output (an image, a stylesheet) has no
            // heading set to check against.
            let Some(set) = ids.get(&key) else {
                continue;
            };
            if !set.contains(&anchor) {
                report.errors.push(format!(
                    "{}:{line}: broken anchor #{anchor} (no heading or id with that name)",
                    source.display()
                ));
            }
        }
    }

    for item in &cfg.menu {
        let Some(key) = resolve_link("/", &item.url) else {
            continue;
        };
        report.links += 1;
        if !known.contains(&key) {
            report.errors.push(format!(
                "agno.toml: menu item {:?} points at missing {key}",
                item.title
            ));
        }
    }

    Ok(report)
}

/// Every Markdown body that can contain links, with the URL it is served from
/// and the source line the body starts on.
fn bodies(content: &Content) -> Vec<(&Path, &str, &str, usize)> {
    let mut out: Vec<(&Path, &str, &str, usize)> =
        Vec::with_capacity(content.pages.len() + content.sections.len() + 1);
    for page in &content.pages {
        out.push((&page.source, &page.url, &page.body, page.body_line));
    }
    if !content.home.body.is_empty() {
        out.push((
            &content.home.source,
            "/",
            &content.home.body,
            content.home.body_line,
        ));
    }
    for section in &content.sections {
        if !section.body.is_empty() {
            out.push((
                &section.source,
                &section.url,
                &section.body,
                section.body_line,
            ));
        }
    }
    out
}

/// Split a link target into its path and its `#fragment`.
fn split_anchor(target: &str) -> (&str, Option<&str>) {
    match target.split_once('#') {
        Some((path, anchor)) => (path.split('?').next().unwrap_or(""), Some(anchor)),
        None => (target.split('?').next().unwrap_or(""), None),
    }
}

/// Every id a rendered page exposes: heading anchors plus ids written directly
/// in the Markdown as raw HTML.
fn heading_ids(body: &str) -> HashSet<String> {
    let rendered = markdown::render(body, false);
    let mut ids: HashSet<String> = rendered.anchors().iter().cloned().collect();
    for id in raw_ids(body) {
        ids.insert(id);
    }
    ids
}

/// Ids written as raw HTML. Parsing events instead of scanning the source keeps
/// `id=` strings inside inline code and fenced blocks from counting as anchors.
fn raw_ids(body: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for event in Parser::new_ext(body, markdown::options()) {
        let (Event::Html(html) | Event::InlineHtml(html)) = event else {
            continue;
        };
        for (index, _) in html.match_indices("id=") {
            let rest = &html[index + 3..];
            let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                continue;
            };
            let value = &rest[quote.len_utf8()..];
            if let Some(end) = value.find(quote) {
                ids.push(value[..end].to_string());
            }
        }
    }
    ids
}

/// Turn a link into the `output_rel` key of the file it should hit, or `None`
/// when the link is external, an in-page anchor, or otherwise not checkable.
#[must_use]
pub fn resolve_link(from_url: &str, target: &str) -> Option<String> {
    let target = target.trim();
    if target.is_empty() || target.starts_with('#') || target.starts_with("//") {
        return None;
    }
    // Any RFC 3986 scheme (`https:`, `mailto:`, `sms:`, `data:`, ...) points
    // outside the site. Checking for a scheme instead of a fixed list keeps
    // app-specific links such as `sms:` from being reported as broken pages.
    if has_scheme(target) {
        return None;
    }
    let path = target.split(['#', '?']).next().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    // A link written as `my%20file.pdf` must resolve to the file the build
    // writes as `my file.pdf`. A stray `%` is not an escape: fall back to the
    // literal path so a file genuinely named `100%.pdf` still resolves.
    let path = site::percent_decode(path).unwrap_or_else(|| path.to_string());

    let mut segments: Vec<&str> = Vec::new();
    let joined = if path.starts_with('/') {
        path
    } else {
        let base = from_url.rsplit_once('/').map_or("", |(head, _)| head);
        format!("{base}/{path}")
    };
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let key = segments.join("/");
    if key.is_empty() {
        return Some("index.html".to_string());
    }
    let last = segments.last().copied().unwrap_or("");
    if joined.ends_with('/') || !last.contains('.') {
        Some(format!("{key}/index.html"))
    } else {
        Some(key)
    }
}

fn has_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Collect Markdown link and image targets with their body-relative line
/// numbers. Parsing instead of scanning means reference-style links resolve and
/// links written inside inline code are not mistaken for real ones.
fn extract_links(body: &str) -> Vec<(usize, String)> {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(body.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let mut links = Vec::new();
    for (event, range) in Parser::new_ext(body, markdown::options()).into_offset_iter() {
        let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event else {
            continue;
        };
        if dest_url.is_empty() {
            continue;
        }
        let line = line_starts.partition_point(|&start| start <= range.start);
        links.push((line, dest_url.to_string()));
    }
    links
}

#[must_use]
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    for warning in &report.warnings {
        out.push_str("warning: ");
        out.push_str(warning);
        out.push('\n');
    }
    for error in &report.errors {
        out.push_str("error: ");
        out.push_str(error);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_absolute_relative_and_directory_links() {
        assert_eq!(
            resolve_link("/blog/post/", "/about/").as_deref(),
            Some("about/index.html")
        );
        assert_eq!(
            resolve_link("/blog/post/", "../other/").as_deref(),
            Some("blog/other/index.html")
        );
        assert_eq!(
            resolve_link("/blog/post/", "/feed.xml").as_deref(),
            Some("feed.xml")
        );
        assert_eq!(
            resolve_link("/blog/post/", "image.png").as_deref(),
            Some("blog/post/image.png")
        );
        assert_eq!(resolve_link("/blog/post/", "#section"), None);
        assert_eq!(resolve_link("/blog/post/", "https://example.com"), None);
        assert_eq!(
            resolve_link("/blog/post/", "/my%20file.pdf").as_deref(),
            Some("my file.pdf")
        );
        assert_eq!(
            resolve_link("/blog/post/", "/100%.pdf").as_deref(),
            Some("100%.pdf")
        );
        assert_eq!(resolve_link("/blog/post/", "sms:+15551234"), None);
        assert_eq!(resolve_link("/blog/post/", "mailto:a@example.com"), None);
        assert_eq!(resolve_link("/blog/post/", "custom-scheme:thing"), None);
    }

    #[test]
    fn ignores_links_inside_code_fences() {
        let body = "```\n[a](/nope/)\n```\n\n[b](/yes/)\n";
        let links = extract_links(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1, "/yes/");
    }

    #[test]
    fn resolves_reference_style_links_and_ignores_inline_code() {
        let body = "[ref][target] and `[inline](/nope/)`\n\n[target]: /real/\n";
        let links = extract_links(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], (1, "/real/".to_string()));
    }

    #[test]
    fn link_lines_are_body_relative() {
        let body = "intro\n\n![alt](/image.png)\n";
        assert_eq!(extract_links(body), [(3, "/image.png".to_string())]);
    }
}
