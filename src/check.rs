//! `agno check`: static analysis of a site before it is deployed.
//!
//! The build already rejects duplicate output URLs; this command goes further
//! and resolves every internal link and image reference in the Markdown source
//! against the set of files the build will produce.

use crate::config::Config;
use crate::content::{self, Content};
use crate::render::Templates;
use crate::site;
use anyhow::Result;
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

        let body = strip_code_fences(&page.body);
        for (line, target) in extract_links(&body) {
            let Some(key) = resolve_link(&page.url, &target) else {
                continue;
            };
            report.links += 1;
            if !known.contains(&key) {
                report.errors.push(format!(
                    "{}:{line}: broken link {target} (resolves to /{key})",
                    page.source.display()
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

/// Collect inline Markdown link and image targets with their line numbers.
fn extract_links(body: &str) -> Vec<(usize, String)> {
    let bytes = body.as_bytes();
    let mut links = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if bytes[i] == b']' && bytes.get(i + 1) == Some(&b'(') {
            let start = i + 2;
            let mut end = start;
            let mut depth = 0usize;
            while end < bytes.len() {
                match bytes[end] {
                    b'\n' => break,
                    b'(' => depth += 1,
                    b')' if depth == 0 => break,
                    b')' => depth -= 1,
                    _ => {}
                }
                end += 1;
            }
            let raw = &body[start..end];
            let target = raw
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(['<', '>']);
            if !target.is_empty() {
                links.push((line, target.to_string()));
            }
            i = end;
            continue;
        }
        i += 1;
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
        let links = extract_links(&strip_code_fences(body));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1, "/yes/");
    }
}
