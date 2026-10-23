use crate::config::{Config, MenuItem};
use crate::content::{self, Content, Page};
use crate::markdown::escape_html;
use crate::render::Templates;
use crate::theme;
use anyhow::{Context, Result, bail};
use minijinja::context;
use minijinja::value::Serde;
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
    description: &'a str,
    content: &'a str,
    tags: &'a [String],
    categories: &'a [String],
    slug: &'a str,
    section: Option<&'a str>,
    extra: &'a Json,
}

#[derive(Serialize)]
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
    url: &'a str,
    description: &'a str,
    content: &'a str,
    pages: Vec<CardView<'a>>,
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
    let cfg = Config::load(root)?;
    cfg.validate()?;

    let phase = Instant::now();
    let content = content::load(&cfg, opts.drafts)?;
    validate_urls(&cfg, &content)?;
    let ms_load = phase.elapsed().as_millis();

    let templates = Templates::load(&cfg.templates())?;
    let index = PageIndex::new(&content.pages);
    let site = site_view(&cfg, &content, &index);
    let output = cfg.output();
    if opts.clean && output.exists() {
        remove_all(&output).with_context(|| format!("cleaning {}", output.display()))?;
    }
    fs::create_dir_all(&output).with_context(|| format!("creating {}", output.display()))?;

    let mut expected = HashSet::new();
    for page in &content.pages {
        expected.insert(output_rel(&page.url));
    }

    let mut lists: Vec<(ListView<'_>, &[&str])> = Vec::new();
    lists.push((
        ListView {
            title: &content.home.title,
            url: "/",
            description: &content.home.description,
            content: &content.home.html,
            pages: content.pages.iter().map(card).collect(),
        },
        &["index.html"],
    ));
    for section in &content.sections {
        lists.push((
            ListView {
                title: &section.title,
                url: &section.url,
                description: &section.description,
                content: &section.content,
                pages: index
                    .section(&section.key)
                    .iter()
                    .copied()
                    .map(card)
                    .collect(),
            },
            &["section.html"],
        ));
    }
    for tag in &content.tags {
        lists.push((
            ListView {
                title: &tag.title,
                url: &tag.url,
                description: "",
                content: "",
                pages: index.tag(&tag.key).iter().copied().map(card).collect(),
            },
            &["tag.html", "section.html"],
        ));
    }
    lists.push((
        ListView {
            title: "404",
            url: "/404.html",
            description: "",
            content: "",
            pages: Vec::new(),
        },
        &["404.html"],
    ));
    expected.extend(lists.iter().map(|(list, _)| output_rel(list.url)));
    expected.insert("rss.xml".into());
    expected.insert("sitemap.xml".into());
    expected.insert("robots.txt".into());
    expected.insert("style.css".into());
    if cfg.search_index {
        expected.insert("search-index.json".into());
    }
    let static_files = static_files(&cfg)?;
    let static_paths = static_rels(&cfg, &static_files)?;
    let has_static_style = static_paths.iter().any(|path| path == "style.css");
    expected.extend(static_paths);

    if !opts.clean {
        remove_stale(root, &output, &expected)?;
    }

    let phase = Instant::now();
    content
        .pages
        .par_iter()
        .map(|page| render_page(&cfg, &templates, &site, page))
        .collect::<Result<Vec<_>>>()?;
    let ms_pages = phase.elapsed().as_millis();

    let phase = Instant::now();
    lists.par_iter().try_for_each(|(list, candidates)| {
        render_list(&cfg, &templates, &site, list, candidates)
    })?;

    write_text(&output, "rss.xml", &rss(&cfg, &content.pages))?;
    write_text(&output, "sitemap.xml", &sitemap(&cfg, &content))?;
    if cfg.search_index {
        let index = search_index(&content.pages)?;
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

    Ok(Stats {
        pages: content.pages.len(),
        ms_total: started.elapsed().as_millis(),
        ms_load,
        ms_pages,
        ms_rest: phase.elapsed().as_millis(),
        output,
    })
}

fn render_page(
    cfg: &Config,
    templates: &Templates,
    site: &SiteView<'_>,
    page: &Page,
) -> Result<()> {
    let name = match page.template.as_deref() {
        Some(name) if templates.has(name) => name.to_string(),
        Some(name) => bail!("template {name} not found for page {}", page.url),
        None => templates
            .first(&["page.html"])
            .with_context(|| format!("no template for page {}", page.url))?,
    };
    let view = page_view(page);
    let cards: Vec<CardView> = Vec::new();
    let ctx = context! {
        site => Serde(site),
        page => Serde(&view),
        pages => Serde(&cards),
    };
    let html = templates
        .render(&name, ctx)
        .with_context(|| format!("rendering page {}", page.url))?;
    write_output(cfg, &page.url, &html)
}

fn render_list(
    cfg: &Config,
    templates: &Templates,
    site: &SiteView<'_>,
    list: &ListView<'_>,
    candidates: &[&str],
) -> Result<()> {
    let url = list.url;
    let name = templates
        .first(candidates)
        .with_context(|| format!("no template found for {url} among {candidates:?}"))?;
    let ctx = context! {
        site => Serde(site),
        page => Serde(list),
        pages => Serde(&list.pages),
    };
    let html = templates
        .render(&name, ctx)
        .with_context(|| format!("rendering list {url}"))?;
    write_output(cfg, url, &html)
}

fn write_output(cfg: &Config, url: &str, html: &str) -> Result<()> {
    let path = output_path(cfg, url);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, html).with_context(|| format!("writing {}", path.display()))
}

fn output_path(cfg: &Config, url: &str) -> PathBuf {
    cfg.output().join(output_rel(url))
}

fn output_rel(url: &str) -> String {
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

fn page_view(page: &Page) -> PageView<'_> {
    PageView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        date_display: &page.date_display,
        datetime: &page.datetime,
        reading_minutes: page.reading_minutes,
        description: page.plain_description(),
        content: &page.html,
        tags: &page.tags,
        categories: &page.categories,
        slug: &page.slug,
        section: page.section.as_deref(),
        extra: &page.extra,
    }
}

fn card(page: &Page) -> CardView<'_> {
    CardView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        date_display: &page.date_display,
        datetime: &page.datetime,
        reading_minutes: page.reading_minutes,
        summary: page.plain_description(),
        tags: &page.tags,
        section: page.section.as_deref(),
    }
}

fn rss(cfg: &Config, pages: &[Page]) -> String {
    let mut items = String::new();
    for page in pages
        .iter()
        .filter(|page| page.date.is_some())
        .take(cfg.rss_items)
    {
        let link = cfg.absolute(&page.url);
        let pub_date = page.rss_date();
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
            escape_html(page.plain_description()),
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

fn search_index(pages: &[Page]) -> Result<String> {
    let entries: Vec<SearchEntry<'_>> = pages
        .iter()
        .filter(|page| page.date.is_some())
        .map(|page| SearchEntry {
            title: &page.title,
            url: &page.url,
            date: &page.datetime,
            description: page.plain_description(),
            summary: &page.summary,
            tags: &page.tags,
        })
        .collect();
    serde_json::to_string(&entries).context("serializing search-index.json")
}

fn sitemap(cfg: &Config, content: &Content) -> String {
    let mut entries: Vec<(String, Option<OffsetDateTime>)> =
        vec![("/".to_string(), content.home.date)];
    entries.extend(
        content
            .pages
            .iter()
            .map(|page| (page.url.clone(), page.updated.or(page.date))),
    );
    entries.extend(content.sections.iter().map(|s| (s.url.clone(), None)));
    entries.extend(content.tags.iter().map(|t| (t.url.clone(), None)));

    let mut urls = String::new();
    for (url, date) in entries {
        urls.push_str("<url><loc>");
        urls.push_str(&escape_html(&cfg.absolute(&url)));
        urls.push_str("</loc>");
        if let Some(lastmod) = date.and_then(|d| d.format(&Rfc3339).ok()) {
            urls.push_str(&format!("<lastmod>{lastmod}</lastmod>"));
        }
        urls.push_str("</url>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{urls}</urlset>\n"
    )
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
