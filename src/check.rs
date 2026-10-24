//! `agno check`: static analysis of a site before it is deployed.
//!
//! The build already rejects duplicate output URLs; this command goes further
//! and resolves every internal link and image reference in the Markdown source
//! against the set of files the build will produce.

use crate::config::Config;
use crate::content::{self, Content};
use crate::markdown;
use crate::render::Templates;
use crate::site;
use anyhow::Result;
use pulldown_cmark::{Event, Parser, Tag};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Default)]
pub struct Report {
    pub pages: usize,
    pub links: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn check(root: &Path, drafts: bool) -> Result<Report> {
    let cfg = Config::load(root)?;
    cfg.validate()?;
    let content = content::load(&cfg, drafts)?;
    let templates = Templates::load(&cfg.templates())?;
    let mut report = Report {
        pages: content.pages.len(),
        ..Report::default()
    };

    if let Err(err) = site::validate_outputs(&cfg, &content) {
        report.errors.push(err.to_string());
    }

    let known = known_outputs(&cfg, &content);

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
    for id in raw_ids(&strip_code_fences(body)) {
        ids.insert(id);
    }
    ids
}

fn raw_ids(body: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (index, _) in body.match_indices("id=") {
        let rest = &body[index + 3..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let value = &rest[quote.len_utf8()..];
        if let Some(end) = value.find(quote) {
            ids.push(value[..end].to_string());
        }
    }
    ids
}

/// Every path the build will write, in `output_rel` form.
fn known_outputs(cfg: &Config, content: &Content) -> HashSet<String> {
    let mut known = HashSet::new();
    let mut add = |url: &str| {
        known.insert(site::output_rel(url));
    };

    add("/");
    add("/404.html");
    add("/style.css");
    add("/rss.xml");
    add("/sitemap.xml");
    add("/robots.txt");
    if cfg.search_index {
        add("/search-index.json");
    }
    for page in &content.pages {
        add(&page.url);
        for alias in &page.aliases {
            add(alias);
        }
    }
    for section in &content.sections {
        add(&section.url);
    }
    for tag in &content.tags {
        add(&tag.url);
    }
    for asset in &content.assets {
        add(&asset.url);
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for page in &content.pages {
        if let Some(section) = page.section.as_deref() {
            *counts.entry(section).or_default() += 1;
        }
    }
    for section in &content.sections {
        let Some(size) = section.paginate_by else {
            continue;
        };
        let total = counts
            .get(section.key.as_str())
            .copied()
            .unwrap_or(0)
            .div_ceil(size);
        for number in 2..=total {
            add(&format!(
                "{}/page/{number}/",
                section.url.trim_end_matches('/')
            ));
        }
    }

    if cfg.static_dir().is_dir() {
        for entry in walkdir::WalkDir::new(cfg.static_dir())
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            if let Ok(rel) = entry.path().strip_prefix(cfg.static_dir()) {
                let rel = rel
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                known.insert(rel);
            }
        }
    }

    known
}

/// Turn a link into the `output_rel` key of the file it should hit, or `None`
/// when the link is external, an in-page anchor, or otherwise not checkable.
pub fn resolve_link(from_url: &str, target: &str) -> Option<String> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with("//")
        || target.contains("://")
        || target.starts_with("mailto:")
        || target.starts_with("tel:")
        || target.starts_with("data:")
        || target.starts_with("javascript:")
    {
        return None;
    }
    let path = target.split(['#', '?']).next().unwrap_or("");
    if path.is_empty() {
        return None;
    }

    let mut segments: Vec<&str> = Vec::new();
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        let base = from_url
            .rsplit_once('/')
            .map(|(head, _)| head)
            .unwrap_or("");
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
    if path.ends_with('/') || !last.contains('.') {
        Some(format!("{key}/index.html"))
    } else {
        Some(key)
    }
}

/// Blank out fenced code so links inside examples are not reported.
fn strip_code_fences(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push('\n');
        } else {
            out.push_str(line);
        }
    }
    out
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
        let target = match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
            _ => continue,
        };
        if target.is_empty() {
            continue;
        }
        let line = line_starts.partition_point(|&start| start <= range.start);
        links.push((line, target.to_string()));
    }
    links
}

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
