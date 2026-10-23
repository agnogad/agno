use crate::cache;
use crate::config::{Config, MenuItem};
use crate::content::{self, Content, Listing, Page};
use crate::markdown::{self, TocEntry, escape_html, push_escaped};
use crate::render::Templates;
use crate::theme;
use anyhow::{Context, Result, bail};
use minijinja::context;
use minijinja::value::Serde;
use minijinja::{Template, Value};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use walkdir::WalkDir;

pub struct BuildOptions {
    pub drafts: bool,
    pub clean: bool,
    /// Minify HTML output (the config file can also turn this on).
    pub minify: bool,
}

#[derive(Debug)]
pub struct Stats {
    pub pages: usize,
    pub ms_total: u128,
    pub ms_load: u128,
    pub ms_pages: u128,
    pub ms_rest: u128,
    pub output: PathBuf,
}

#[derive(Serialize)]
struct SiteView<'a> {
    title: String,
    description: String,
    base_url: String,
    language: String,
    author: String,
    menu: Vec<MenuItem>,
    year: i32,
    post_count: usize,
    tags: Vec<TagView<'a>>,
}

#[derive(Serialize)]
struct TagView<'a> {
    name: &'a str,
    url: &'a str,
    count: usize,
    latest_title: &'a str,
    latest_url: &'a str,
}

#[derive(Serialize)]
struct PageView<'a> {
    title: &'a str,
    url: &'a str,
    date: &'a str,
    date_display: &'a str,
    datetime: &'a str,
    reading_minutes: u32,
    words: u32,
    description: &'a str,
    content: &'a str,
    tags: &'a [String],
    categories: &'a [String],
    slug: &'a str,
    section: Option<&'a str>,
    extra: &'a Json,
    toc: &'a [TocEntry],
    toc_html: &'a str,
}

#[derive(Serialize, Clone)]
struct CardView<'a> {
    title: &'a str,
    url: &'a str,
    date: &'a str,
    date_display: &'a str,
    datetime: &'a str,
    reading_minutes: u32,
    summary: &'a str,
    tags: &'a [String],
    section: Option<&'a str>,
}

#[derive(Serialize)]
struct SearchEntry<'a> {
    title: &'a str,
    url: &'a str,
    date: &'a str,
    description: &'a str,
    summary: &'a str,
    tags: &'a [String],
}

#[derive(Serialize)]
struct ListView<'a> {
    title: &'a str,
    url: String,
    description: &'a str,
    content: &'a str,
    pages: Vec<CardView<'a>>,
    paginator: Option<PagerView>,
}

#[derive(Serialize, Clone)]
struct PagerView {
    current: usize,
    total: usize,
    page_size: usize,
    item_count: usize,
    prev_url: Option<String>,
    next_url: Option<String>,
    first_url: String,
    last_url: String,
    links: Vec<PagerLink>,
}

#[derive(Serialize, Clone)]
struct PagerLink {
    number: usize,
    url: String,
    current: bool,
}

/// Page 1 lives at the section URL; later pages at `/section/page/N/`.
fn page_url(section_url: &str, number: usize) -> String {
    if number <= 1 {
        return section_url.to_string();
    }
    format!("{}/page/{number}/", section_url.trim_end_matches('/'))
}

struct PageIndex<'a> {
    sections: HashMap<&'a str, Vec<&'a Page>>,
    tags: HashMap<String, Vec<&'a Page>>,
}

impl<'a> PageIndex<'a> {
    fn new(pages: &'a [Page]) -> Self {
        let mut sections: HashMap<&str, Vec<&Page>> = HashMap::new();
        let mut tags: HashMap<String, Vec<&Page>> = HashMap::new();
        for page in pages {
            if let Some(section) = page.section.as_deref() {
                sections.entry(section).or_default().push(page);
            }
            let mut seen = HashSet::new();
            for tag in &page.tags {
                let slug = content::slugify(tag);
                if !slug.is_empty() && seen.insert(slug.clone()) {
                    tags.entry(slug).or_default().push(page);
                }
            }
        }
        Self { sections, tags }
    }

    fn section(&self, key: &str) -> &[&'a Page] {
        self.sections.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    fn tag(&self, key: &str) -> &[&'a Page] {
        self.tags.get(key).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    output: String,
    files: Vec<String>,
}

pub fn build(root: &Path, opts: &BuildOptions) -> Result<Stats> {
    let started = Instant::now();
    let mut cfg = Config::load(root)?;
    if opts.minify {
        cfg.minify = true;
    }
    cfg.validate()?;

    let phase = Instant::now();
    let content = content::load(&cfg, opts.drafts)?;
    validate_urls(&cfg, &content)?;
    let ms_load = phase.elapsed().as_millis();

    let templates = Templates::load(&cfg.templates())?;
    let index = PageIndex::new(&content.pages);
    let site = site_view(&cfg, &content, &index);
    // Serialize the site once: it is identical for every page, and rebuilding
    // it per page used to cost O(pages x tags) on taxonomy-heavy sites.
    let site_value: Value = Serde(&site).into();
    let header = header_hash(&cfg, &site)?;
    let fingerprints = cache::Fingerprints::load(root, header);
    let output = cfg.output();
    if opts.clean && output.exists() {
        remove_all(&output).with_context(|| format!("cleaning {}", output.display()))?;
    }
    fs::create_dir_all(&output).with_context(|| format!("creating {}", output.display()))?;

    // Stale output is removed before anything is written, so a URL that
    // changed shape (file to directory, or back) can be recreated cleanly.
    let static_files = static_files(&cfg)?;
    let static_paths = static_rels(&cfg, &static_files)?;
    let has_static_style = static_paths.iter().any(|path| path == "style.css");
    let mut expected = expected_outputs(&cfg, &content, &static_paths)?;
    if !opts.clean {
        remove_stale(root, &output, &expected)?;
    }

    // ---- pages ----
    let phase = Instant::now();
    let default_name = templates
        .first(&["page.html"])
        .context("no template for pages: page.html is missing")?;
    let default_page = templates.get(&default_name)?;
    let fresh: Vec<Option<String>> = content
        .pages
        .par_iter()
        .map(|page| {
            render_page(
                &cfg,
                &templates,
                &site_value,
                &fingerprints,
                &default_name,
                &default_page,
                page,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let ms_pages = phase.elapsed().as_millis();

    content
        .pages
        .par_iter()
        .try_for_each(|page| write_aliases(&cfg, page))?;

    // A page's summary comes from its own Markdown, so it is only known once
    // the page has rendered (or read back from the fingerprint cache).
    let summaries: HashMap<&str, &str> = content
        .pages
        .iter()
        .zip(&fresh)
        .map(|(page, fresh)| {
            let summary = fresh
                .as_deref()
                .or_else(|| fingerprints.summary(&page.url, page.content_key))
                .unwrap_or(&page.summary);
            (page.url.as_str(), summary)
        })
        .collect();

    let lists = build_lists(&content, &index, &summaries)?;
    expected.extend(lists.iter().map(|(list, _)| output_rel(&list.url)));

    let phase = Instant::now();
    let mut list_names: Vec<String> = Vec::with_capacity(lists.len());
    for (list, candidates) in &lists {
        list_names.push(
            templates.first(candidates).with_context(|| {
                format!("no template found for {} among {candidates:?}", list.url)
            })?,
        );
    }
    let list_keys: Vec<u64> = lists
        .iter()
        .zip(&list_names)
        .map(|((list, _), name)| list_key(header, name, list))
        .collect();
    let list_rendered: Vec<bool> = lists
        .par_iter()
        .zip(list_names.par_iter())
        .zip(list_keys.par_iter())
        .map(|((list, name), key)| -> Result<bool> {
            let (list, _) = list;
            if fingerprints.summary(&list.url, *key).is_some()
                && output_path(&cfg, &list.url).is_file()
            {
                return Ok(false);
            }
            render_list(&cfg, &templates, &site_value, list, name)?;
            Ok(true)
        })
        .collect::<Result<Vec<_>>>()?;

    write_text(&output, "rss.xml", &rss(&cfg, &content.pages, &summaries))?;
    write_text(&output, "sitemap.xml", &sitemap(&cfg, &content))?;
    if cfg.search_index {
        let index = search_index(&content.pages, &summaries)?;
        write_text(&output, "search-index.json", &index)?;
    }
    write_text(
        &output,
        "robots.txt",
        &format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
            cfg.base_url
        ),
    )?;

    copy_static(&cfg, &static_files)?;
    if !has_static_style {
        fs::write(output.join("style.css"), theme::STYLE).context("writing default style.css")?;
    }

    write_manifest(root, &output, &expected)?;

    // Only rewrite the fingerprint file when the page or list set changed.
    let expected_entries = content.pages.len() + lists.len();
    if fresh.iter().any(Option::is_some)
        || list_rendered.iter().any(|rendered| *rendered)
        || fingerprints.len() != expected_entries
    {
        let pages = content.pages.iter().zip(fresh.iter()).map(|(page, fresh)| {
            let summary = fresh
                .as_deref()
                .or_else(|| fingerprints.summary(&page.url, page.content_key))
                .unwrap_or(&page.summary);
            (page.url.as_str(), page.content_key, summary)
        });
        let lists = lists
            .iter()
            .zip(&list_keys)
            .map(|((list, _), key)| (list.url.as_str(), *key, ""));
        fingerprints.save(root, pages.chain(lists))?;
    }

    Ok(Stats {
        pages: content.pages.len(),
        ms_total: started.elapsed().as_millis(),
        ms_load,
        ms_pages,
        ms_rest: phase.elapsed().as_millis(),
        output,
    })
}

/// Every path the build is expected to write, computed from metadata only so
/// stale output can be removed before any page is rendered.
fn expected_outputs(
    cfg: &Config,
    content: &Content,
    static_paths: &[String],
) -> Result<HashSet<String>> {
    let mut expected = HashSet::new();
    for page in &content.pages {
        expected.insert(output_rel(&page.url));
        for alias in &page.aliases {
            expected.insert(output_rel(alias));
        }
    }
    expected.insert(output_rel("/"));
    expected.insert(output_rel("/404.html"));

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for page in &content.pages {
        if let Some(section) = page.section.as_deref() {
            *counts.entry(section).or_default() += 1;
        }
    }
    for section in &content.sections {
        expected.insert(output_rel(&section.url));
        let count = counts.get(section.key.as_str()).copied().unwrap_or(0);
        for number in 2..=section_page_count(section, count) {
            let url = page_url(&section.url, number);
            if !expected.insert(output_rel(&url)) {
                bail!("paginated URL {url} collides with another page or section");
            }
        }
    }
    for tag in &content.tags {
        expected.insert(output_rel(&tag.url));
    }

    expected.insert("rss.xml".into());
    expected.insert("sitemap.xml".into());
    expected.insert("robots.txt".into());
    expected.insert("style.css".into());
    if cfg.search_index {
        expected.insert("search-index.json".into());
    }
    expected.extend(static_paths.iter().cloned());
    Ok(expected)
}

fn section_page_count(section: &Listing, count: usize) -> usize {
    match section.paginate_by {
        Some(size) if size > 0 && count > size => count.div_ceil(size),
        _ => 1,
    }
}

fn build_lists<'a>(
    content: &'a Content,
    index: &PageIndex<'a>,
    summaries: &HashMap<&'a str, &'a str>,
) -> Result<Vec<(ListView<'a>, &'static [&'static str])>> {
    let mut lists: Vec<(ListView<'a>, &'static [&'static str])> = Vec::new();
    lists.push((
        ListView {
            title: &content.home.title,
            url: "/".to_string(),
            description: &content.home.description,
            content: &content.home.html,
            pages: content
                .pages
                .iter()
                .map(|page| card(page, summaries))
                .collect(),
            paginator: None,
        },
        &["index.html"],
    ));
    for section in &content.sections {
        let all: Vec<CardView> = index
            .section(&section.key)
            .iter()
            .copied()
            .map(|page| card(page, summaries))
            .collect();
        let page_size = section.paginate_by.unwrap_or(0);
        let total = section_page_count(section, all.len());
        if total <= 1 {
            lists.push((
                ListView {
                    title: &section.title,
                    url: section.url.clone(),
                    description: &section.description,
                    content: &section.content,
                    pages: all,
                    paginator: None,
                },
                &["section.html"],
            ));
            continue;
        }
        let links: Vec<PagerLink> = (1..=total)
            .map(|number| PagerLink {
                number,
                url: page_url(&section.url, number),
                current: false,
            })
            .collect();
        for (index, chunk) in all.chunks(page_size).enumerate() {
            let current = index + 1;
            let url = page_url(&section.url, current);
            let mut links = links.clone();
            for link in &mut links {
                link.current = link.number == current;
            }
            lists.push((
                ListView {
                    title: &section.title,
                    url,
                    description: &section.description,
                    content: &section.content,
                    pages: chunk.to_vec(),
                    paginator: Some(PagerView {
                        current,
                        total,
                        page_size,
                        item_count: all.len(),
                        prev_url: (current > 1).then(|| page_url(&section.url, current - 1)),
                        next_url: (current < total).then(|| page_url(&section.url, current + 1)),
                        first_url: section.url.clone(),
                        last_url: page_url(&section.url, total),
                        links,
                    }),
                },
                &["section.html"],
            ));
        }
    }
    for tag in &content.tags {
        lists.push((
            ListView {
                title: &tag.title,
                url: tag.url.clone(),
                description: "",
                content: "",
                pages: index
                    .tag(&tag.key)
                    .iter()
                    .copied()
                    .map(|page| card(page, summaries))
                    .collect(),
                paginator: None,
            },
            &["tag.html", "section.html"],
        ));
    }
    lists.push((
        ListView {
            title: "404",
            url: "/404.html".to_string(),
            description: "",
            content: "",
            pages: Vec::new(),
            paginator: None,
        },
        &["404.html"],
    ));
    Ok(lists)
}

fn write_aliases(cfg: &Config, page: &Page) -> Result<()> {
    for alias in &page.aliases {
        let body = redirect_page(&cfg.absolute(&page.url), &page.title);
        write_output(cfg, alias, &body)
            .with_context(|| format!("writing alias {alias} for {}", page.url))?;
    }
    Ok(())
}

fn redirect_page(target: &str, title: &str) -> String {
    let target = escape_html(target);
    let title = escape_html(title);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<title>Redirecting to {title}</title>\n<link rel=\"canonical\" href=\"{target}\">\n\
<meta http-equiv=\"refresh\" content=\"0; url={target}\">\n<meta name=\"robots\" content=\"noindex\">\n\
</head>\n<body>\n<p>Redirecting to <a href=\"{target}\">{title}</a>.</p>\n</body>\n</html>\n"
    )
}

#[allow(clippy::too_many_arguments)]
fn render_page(
    cfg: &Config,
    templates: &Templates,
    site: &Value,
    fingerprints: &cache::Fingerprints,
    default_name: &str,
    default_template: &Template<'_, '_>,
    page: &Page,
) -> Result<Option<String>> {
    // The rendered file in `dist/` doubles as the cache: when the page source,
    // the templates and the site-wide context are unchanged, skip Markdown,
    // highlighting, templating and the write entirely.
    if fingerprints.summary(&page.url, page.content_key).is_some()
        && output_path(cfg, &page.url).is_file()
    {
        return Ok(None);
    }

    let rendered = markdown::render(&page.body, cfg.highlight);
    let summary = if page.summary.is_empty() {
        rendered.summary
    } else {
        page.summary.clone()
    };
    let view = page_view(
        page,
        &rendered.html,
        &summary,
        &rendered.toc,
        &rendered.toc_html,
    );
    let cards: Vec<CardView> = Vec::new();
    let ctx = context! {
        site => site.clone(),
        page => Serde(&view),
        pages => Serde(&cards),
    };
    let rendered = match page.template.as_deref() {
        Some(name) if name != default_name => templates
            .get(name)
            .with_context(|| format!("template {name} not found for page {}", page.url))?
            .render(ctx)
            .with_context(|| format!("rendering page {}", page.url))?,
        _ => default_template
            .render(ctx)
            .with_context(|| format!("rendering page {}", page.url))?,
    };
    write_output(cfg, &page.url, &rendered)?;
    Ok(Some(summary))
}

fn render_list(
    cfg: &Config,
    templates: &Templates,
    site: &Value,
    list: &ListView<'_>,
    name: &str,
) -> Result<()> {
    let url = &list.url;
    let ctx = context! {
        site => site.clone(),
        page => Serde(list),
        pages => Serde(&list.pages),
    };
    let html = templates
        .render(name, ctx)
        .with_context(|| format!("rendering list {url}"))?;
    write_output(cfg, url, &html)
}

/// Hash everything a list page is rendered from. Lists are re-rendered only
/// when one of their cards, their own metadata, or the site context changes.
fn list_key(header: u64, template: &str, list: &ListView<'_>) -> u64 {
    let mut hash = cache::fingerprint(header, template.as_bytes());
    hash = cache::fingerprint(hash, list.url.as_bytes());
    hash = cache::fingerprint(hash, list.title.as_bytes());
    hash = cache::fingerprint(hash, list.description.as_bytes());
    hash = cache::fingerprint(hash, list.content.as_bytes());
    for card in &list.pages {
        hash = cache::fingerprint(hash, card.url.as_bytes());
        hash = cache::fingerprint(hash, card.title.as_bytes());
        hash = cache::fingerprint(hash, card.date.as_bytes());
        hash = cache::fingerprint(hash, card.date_display.as_bytes());
        hash = cache::fingerprint(hash, card.datetime.as_bytes());
        hash = cache::fingerprint(hash, card.summary.as_bytes());
        hash = cache::fingerprint(hash, card.section.unwrap_or_default().as_bytes());
        hash = cache::fingerprint(hash, &card.reading_minutes.to_le_bytes());
        for tag in card.tags {
            hash = cache::fingerprint(hash, tag.as_bytes());
        }
    }
    if let Some(pager) = &list.paginator {
        for number in [
            pager.current,
            pager.total,
            pager.page_size,
            pager.item_count,
        ] {
            hash = cache::fingerprint(hash, &number.to_le_bytes());
        }
        for url in [
            pager.prev_url.as_deref(),
            pager.next_url.as_deref(),
            Some(pager.first_url.as_str()),
            Some(pager.last_url.as_str()),
        ] {
            hash = cache::fingerprint(hash, url.unwrap_or_default().as_bytes());
        }
    }
    hash
}

fn write_output(cfg: &Config, url: &str, html: &str) -> Result<()> {
    let path = output_path(cfg, url);
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let body = if cfg.minify {
        crate::minify::minify_html(html)
    } else {
        html.to_string()
    };
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

thread_local! {
    /// Directories this worker already created. Avoids one `mkdir` syscall per
    /// page while still keeping writes independent across threads.
    static KNOWN_DIRS: std::cell::RefCell<HashSet<PathBuf>> =
        std::cell::RefCell::new(HashSet::new());
}

fn ensure_dir(dir: &Path) -> Result<()> {
    let known = KNOWN_DIRS.with(|dirs| dirs.borrow().contains(dir));
    if known {
        return Ok(());
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    KNOWN_DIRS.with(|dirs| {
        dirs.borrow_mut().insert(dir.to_path_buf());
    });
    Ok(())
}

fn output_path(cfg: &Config, url: &str) -> PathBuf {
    cfg.output().join(output_rel(url))
}

pub fn output_rel(url: &str) -> String {
    let mut segments: Vec<&str> = url
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect();
    if url.ends_with('/') || segments.is_empty() {
        segments.push("index.html");
    }
    segments.join("/")
}

fn ancestors(path: &str) -> Vec<&str> {
    path.char_indices()
        .filter(|(_, ch)| *ch == '/')
        .map(|(index, _)| &path[..index])
        .collect()
}

pub fn validate_outputs(cfg: &Config, content: &Content) -> Result<()> {
    validate_urls(cfg, content)
}

fn validate_urls(cfg: &Config, content: &Content) -> Result<()> {
    let mut files: HashMap<String, String> = HashMap::new();
    let mut dirs: HashSet<String> = HashSet::new();
    let mut add = |url: &str, what: &str| -> Result<()> {
        let key = output_rel(url).to_ascii_lowercase();
        if let Some(existing) = files.insert(key.clone(), what.to_string()) {
            bail!("duplicate output URL {url}: {existing} and {what}");
        }
        if dirs.contains(&key) {
            bail!("output path conflict at {url}: another output is below it");
        }
        for parent in ancestors(&key) {
            if let Some(existing) = files.get(parent) {
                bail!("output path conflict at {url}: {existing} is a file at {parent}");
            }
            dirs.insert(parent.to_string());
        }
        Ok(())
    };

    add("/", "home")?;
    add("/404.html", "404 page")?;
    add("/style.css", "default stylesheet")?;
    add("/rss.xml", "RSS feed")?;
    add("/sitemap.xml", "sitemap")?;
    add("/robots.txt", "robots.txt")?;
    if cfg.search_index {
        add("/search-index.json", "search index")?;
    }
    for page in &content.pages {
        add(&page.url, &format!("page {}", page.source.display()))?;
        for alias in &page.aliases {
            add(alias, &format!("alias of {}", page.url))?;
        }
    }
    for section in &content.sections {
        add(&section.url, &format!("section {}", section.key))?;
    }
    for tag in &content.tags {
        add(&tag.url, &format!("tag {}", tag.title))?;
    }
    Ok(())
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(".agno-cache").join("outputs.json")
}

/// Everything a page render depends on except the page source itself.
fn header_hash(cfg: &Config, site: &SiteView<'_>) -> Result<u64> {
    let site_json = serde_json::to_string(site).context("serializing site context")?;
    Ok(cache::combine(&[
        cache::fingerprint(0, env!("CARGO_PKG_VERSION").as_bytes()),
        cache::fingerprint(0, if cfg.highlight { b"on" } else { b"off" }),
        cache::fingerprint(0, if cfg.minify { b"min" } else { b"raw" }),
        cache::fingerprint(0, cfg.language.as_bytes()),
        cache::fingerprint(0, site_json.as_bytes()),
        templates_fingerprint(&cfg.templates()),
    ]))
}

fn templates_fingerprint(dir: &Path) -> u64 {
    let mut hash = cache::fingerprint(0, b"agno-templates");
    for (name, source) in theme::TEMPLATES {
        hash = cache::fingerprint(hash, name.as_bytes());
        hash = cache::fingerprint(hash, source.as_bytes());
    }
    if !dir.is_dir() {
        return hash;
    }
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    for file in files {
        hash = cache::fingerprint(hash, file.to_string_lossy().as_bytes());
        if let Ok(bytes) = fs::read(&file) {
            hash = cache::fingerprint(hash, &bytes);
        }
    }
    hash
}

fn read_manifest(root: &Path) -> Option<Manifest> {
    let src = fs::read_to_string(manifest_path(root)).ok()?;
    serde_json::from_str(&src).ok()
}

fn write_manifest(root: &Path, output: &Path, expected: &HashSet<String>) -> Result<()> {
    let path = manifest_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut files: Vec<String> = expected.iter().cloned().collect();
    files.sort();
    let manifest = Manifest {
        output: output.to_string_lossy().to_string(),
        files,
    };
    let body = serde_json::to_string(&manifest).context("serializing build manifest")?;
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn remove_stale(root: &Path, output: &Path, expected: &HashSet<String>) -> Result<()> {
    let Some(manifest) = read_manifest(root) else {
        return Ok(());
    };
    if manifest.output != output.to_string_lossy().as_ref() {
        return Ok(());
    }
    for rel in manifest.files {
        if expected.contains(&rel)
            || rel.is_empty()
            || rel.starts_with('/')
            || rel.split('/').any(|part| part == "." || part == "..")
        {
            continue;
        }
        let path = output.join(&rel);
        match fs::remove_file(&path) {
            Ok(()) => {
                let mut parent = path.parent();
                while let Some(dir) = parent {
                    if dir == output || fs::remove_dir(dir).is_err() {
                        break;
                    }
                    parent = dir.parent();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("removing stale {}", path.display()));
            }
        }
    }
    Ok(())
}

fn write_text(output: &Path, name: &str, body: &str) -> Result<()> {
    let path = output.join(name);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn site_view<'a>(cfg: &Config, content: &'a Content, index: &PageIndex<'a>) -> SiteView<'a> {
    let post_count = content
        .pages
        .iter()
        .filter(|page| page.date.is_some())
        .count();
    let tags = content
        .tags
        .iter()
        .map(|tag| {
            let pages = index.tag(&tag.key);
            let latest = pages.first().copied();
            TagView {
                name: &tag.title,
                url: &tag.url,
                count: pages.len(),
                latest_title: latest.map(|page| page.title.as_str()).unwrap_or_default(),
                latest_url: latest.map(|page| page.url.as_str()).unwrap_or_default(),
            }
        })
        .collect();
    SiteView {
        title: cfg.title.clone(),
        description: cfg.description.clone(),
        base_url: cfg.base_url.clone(),
        language: cfg.language.clone(),
        author: cfg.author.clone(),
        menu: cfg.menu.clone(),
        year: OffsetDateTime::now_utc().year(),
        post_count,
        tags,
    }
}

fn page_view<'a>(
    page: &'a Page,
    html: &'a str,
    summary: &'a str,
    toc: &'a [TocEntry],
    toc_html: &'a str,
) -> PageView<'a> {
    PageView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        date_display: &page.date_display,
        datetime: &page.datetime,
        reading_minutes: page.reading_minutes,
        words: page.words,
        description: if page.description.is_empty() {
            summary
        } else {
            &page.description
        },
        content: html,
        tags: &page.tags,
        categories: &page.categories,
        slug: &page.slug,
        section: page.section.as_deref(),
        extra: &page.extra,
        toc,
        toc_html,
    }
}

fn describe<'a>(page: &'a Page, summary: &'a str) -> &'a str {
    if page.description.is_empty() {
        summary
    } else {
        &page.description
    }
}

fn card<'a>(page: &'a Page, summaries: &HashMap<&'a str, &'a str>) -> CardView<'a> {
    let summary = summaries.get(page.url.as_str()).copied().unwrap_or("");
    CardView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        date_display: &page.date_display,
        datetime: &page.datetime,
        reading_minutes: page.reading_minutes,
        summary: describe(page, summary),
        tags: &page.tags,
        section: page.section.as_deref(),
    }
}

fn rss(cfg: &Config, pages: &[Page], summaries: &HashMap<&str, &str>) -> String {
    let mut items = String::new();
    for page in pages
        .iter()
        .filter(|page| page.date.is_some())
        .take(cfg.rss_items)
    {
        let link = cfg.absolute(&page.url);
        let pub_date = page.rss_date();
        let summary = summaries.get(page.url.as_str()).copied().unwrap_or("");
        items.push_str(&format!(
            "<item><title>{}</title><link>{}</link><guid isPermaLink=\"true\">{}</guid>{}<description>{}</description></item>",
            escape_html(&page.title),
            escape_html(&link),
            escape_html(&link),
            if pub_date.is_empty() {
                String::new()
            } else {
                format!("<pubDate>{pub_date}</pubDate>")
            },
            escape_html(describe(page, summary)),
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\"><channel><title>{}</title><link>{}/</link><description>{}</description><language>{}</language>{items}</channel></rss>\n",
        escape_html(&cfg.title),
        escape_html(&cfg.base_url),
        escape_html(&cfg.description),
        escape_html(&cfg.language),
    )
}

fn search_index(pages: &[Page], summaries: &HashMap<&str, &str>) -> Result<String> {
    let entries: Vec<SearchEntry<'_>> = pages
        .iter()
        .filter(|page| page.date.is_some())
        .map(|page| {
            let summary = summaries.get(page.url.as_str()).copied().unwrap_or("");
            SearchEntry {
                title: &page.title,
                url: &page.url,
                date: &page.datetime,
                description: describe(page, summary),
                summary,
                tags: &page.tags,
            }
        })
        .collect();
    serde_json::to_string(&entries).context("serializing search-index.json")
}

fn sitemap(cfg: &Config, content: &Content) -> String {
    let count = content.pages.len() + content.sections.len() + content.tags.len() + 1;
    let mut urls = String::with_capacity(count * 96);
    push_url(&mut urls, cfg, "/", content.home.date);
    for page in &content.pages {
        push_url(&mut urls, cfg, &page.url, page.updated.or(page.date));
    }
    for section in &content.sections {
        push_url(&mut urls, cfg, &section.url, None);
    }
    for tag in &content.tags {
        push_url(&mut urls, cfg, &tag.url, None);
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{urls}</urlset>\n"
    )
}

fn push_url(out: &mut String, cfg: &Config, url: &str, date: Option<OffsetDateTime>) {
    out.push_str("<url><loc>");
    push_escaped(out, &cfg.base_url);
    push_escaped(out, url);
    out.push_str("</loc>");
    if let Some(lastmod) = date.and_then(|value| value.format(&Rfc3339).ok()) {
        out.push_str("<lastmod>");
        out.push_str(&lastmod);
        out.push_str("</lastmod>");
    }
    out.push_str("</url>");
}

pub fn remove_all(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return fs::remove_file(path);
    }
    if !meta.is_dir() {
        return fs::remove_file(path);
    }
    let entries: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.par_iter().try_for_each(|entry| {
        let meta = fs::symlink_metadata(entry)?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            remove_all(entry)
        } else {
            fs::remove_file(entry)
        }
    })?;
    fs::remove_dir(path)
}

fn static_files(cfg: &Config) -> Result<Vec<PathBuf>> {
    let src = cfg.static_dir();
    if !src.is_dir() {
        return Ok(Vec::new());
    }
    Ok(WalkDir::new(&src)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect())
}

fn static_rels(cfg: &Config, files: &[PathBuf]) -> Result<Vec<String>> {
    let src = cfg.static_dir();
    files
        .iter()
        .map(|file| Ok(rel_string(file.strip_prefix(&src)?)))
        .collect()
}

fn copy_static(cfg: &Config, files: &[PathBuf]) -> Result<()> {
    let src = cfg.static_dir();
    let dest = cfg.output();
    files.par_iter().try_for_each(|file| {
        let rel = file.strip_prefix(&src)?;
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(file, &target)?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

fn rel_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
