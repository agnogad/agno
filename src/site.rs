use crate::config::{Config, MenuItem};
use crate::content::{self, Content, Page};
use crate::markdown::escape_html;
use crate::render::Templates;
use crate::theme;
use anyhow::{Context, Result};
use minijinja::context;
use minijinja::value::Serde;
use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value as Json;
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

pub struct Stats {
    pub pages: usize,
    pub ms_total: u128,
    pub ms_load: u128,
    pub ms_pages: u128,
    pub ms_rest: u128,
    pub output: PathBuf,
}

#[derive(Serialize)]
struct SiteView {
    title: String,
    description: String,
    base_url: String,
    language: String,
    menu: Vec<MenuItem>,
    year: i32,
}

#[derive(Serialize)]
struct PageView<'a> {
    title: &'a str,
    url: &'a str,
    date: &'a str,
    datetime: &'a str,
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
    datetime: &'a str,
    summary: &'a str,
}

#[derive(Serialize)]
struct ListView<'a> {
    title: &'a str,
    url: &'a str,
    description: &'a str,
    content: &'a str,
    pages: Vec<CardView<'a>>,
}

pub fn build(root: &Path, opts: &BuildOptions) -> Result<Stats> {
    let started = Instant::now();
    let cfg = Config::load(root)?;

    let phase = Instant::now();
    let content = content::load(&cfg, opts.drafts)?;
    let ms_load = phase.elapsed().as_millis();

    let templates = Templates::load(&cfg.templates())?;
    let site = site_view(&cfg);
    let output = cfg.output();
    if opts.clean && output.exists() {
        remove_all(&output).with_context(|| format!("cleaning {}", output.display()))?;
    }
    fs::create_dir_all(&output).with_context(|| format!("creating {}", output.display()))?;

    let phase = Instant::now();
    content
        .pages
        .par_iter()
        .map(|page| render_page(&cfg, &templates, &site, page))
        .collect::<Result<Vec<_>>>()?;
    let ms_pages = phase.elapsed().as_millis();

    let phase = Instant::now();

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
                pages: content
                    .pages
                    .iter()
                    .filter(|page| page.section.as_deref() == Some(section.key.as_str()))
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
                pages: content
                    .pages
                    .iter()
                    .filter(|page| page.tags.iter().any(|name| name == &tag.key))
                    .map(card)
                    .collect(),
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
    lists.par_iter().try_for_each(|(list, candidates)| {
        render_list(&cfg, &templates, &site, list, candidates)
    })?;

    write_text(&output, "rss.xml", &rss(&cfg, &content.pages))?;
    write_text(&output, "sitemap.xml", &sitemap(&cfg, &content))?;
    write_text(
        &output,
        "robots.txt",
        &format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
            cfg.base_url
        ),
    )?;

    copy_static(&cfg)?;
    if !output.join("style.css").exists() {
        fs::write(output.join("style.css"), theme::STYLE).context("writing default style.css")?;
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

fn render_page(cfg: &Config, templates: &Templates, site: &SiteView, page: &Page) -> Result<()> {
    let name = page
        .template
        .as_deref()
        .filter(|name| templates.has(name))
        .map(str::to_string)
        .or_else(|| templates.first(&["page.html"]))
        .with_context(|| format!("no template for page {}", page.url))?;
    let view = page_view(page);
    let cards: Vec<CardView> = Vec::new();
    let ctx = context! {
        site => Serde(site),
        page => Serde(&view),
        pages => Serde(&cards),
    };
    let html = templates.render(&name, ctx)?;
    write_output(cfg, &page.url, &html)
}

fn render_list(
    cfg: &Config,
    templates: &Templates,
    site: &SiteView,
    list: &ListView<'_>,
    candidates: &[&str],
) -> Result<()> {
    let name = templates
        .first(candidates)
        .with_context(|| format!("no template found among {candidates:?}"))?;
    let url = list.url;
    let ctx = context! {
        site => Serde(site),
        page => Serde(list),
        pages => Serde(&list.pages),
    };
    let html = templates.render(&name, ctx)?;
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
    let mut segments: Vec<&str> = url
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect();
    match segments.last() {
        Some(last) if last.contains('.') => {}
        _ => segments.push("index.html"),
    }
    cfg.output().join(segments.join("/"))
}

fn write_text(output: &Path, name: &str, body: &str) -> Result<()> {
    let path = output.join(name);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn site_view(cfg: &Config) -> SiteView {
    SiteView {
        title: cfg.title.clone(),
        description: cfg.description.clone(),
        base_url: cfg.base_url.clone(),
        language: cfg.language.clone(),
        menu: cfg.menu.clone(),
        year: OffsetDateTime::now_utc().year(),
    }
}

fn page_view(page: &Page) -> PageView<'_> {
    PageView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        datetime: &page.datetime,
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
        datetime: &page.datetime,
        summary: page.plain_description(),
    }
}

fn rss(cfg: &Config, pages: &[Page]) -> String {
    let mut items = String::new();
    for page in pages.iter().take(cfg.rss_items) {
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
    let entries: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.par_iter().try_for_each(|entry| {
        if entry.is_dir() {
            remove_all(entry)
        } else {
            fs::remove_file(entry)
        }
    })?;
    fs::remove_dir(path)
}

fn copy_static(cfg: &Config) -> Result<()> {
    let src = cfg.static_dir();
    if !src.is_dir() {
        return Ok(());
    }
    let dest = cfg.output();
    let files: Vec<PathBuf> = WalkDir::new(&src)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();

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
