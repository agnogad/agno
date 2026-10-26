use crate::cache;
use crate::config::{Config, MenuItem};
use crate::content::{self, Content, Listing, Page, SortBy};
use crate::markdown::{self, TocEntry, escape_html};
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
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
    categories: Vec<TagView<'a>>,
    sections: Vec<SectionView<'a>>,
}

#[derive(Serialize)]
struct SectionView<'a> {
    title: &'a str,
    url: &'a str,
    count: usize,
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
    updated: &'a str,
    updated_display: &'a str,
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
    /// Previous and next page in the section's own order; `None` at the ends.
    prev: Option<NeighborView<'a>>,
    next: Option<NeighborView<'a>>,
}

#[derive(Serialize, Clone)]
struct NeighborView<'a> {
    title: &'a str,
    url: &'a str,
    date_display: &'a str,
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
    url: &'a str,
    description: &'a str,
    content: &'a str,
    extra: &'a Json,
    pages: Vec<CardView<'a>>,
    paginator: Option<&'a PagerView>,
    /// Terms of a taxonomy overview page (`/tags/`, `/categories/`); empty on
    /// every other list.
    terms: Vec<TermView<'a>>,
}

#[derive(Serialize, Clone)]
struct TermView<'a> {
    name: &'a str,
    url: &'a str,
    count: usize,
}

/// A list before its cards are materialised. Keeping page references here lets
/// an unchanged list be fingerprinted without allocating a `CardView` per page.
struct ListSpec<'a> {
    title: &'a str,
    url: String,
    description: &'a str,
    /// Unrendered Markdown of the section's `_index.md`.
    body: &'a str,
    /// Filled in only when the list is actually written. Paginated pages of
    /// one section share it, so the body is rendered once per section.
    content: Arc<OnceLock<String>>,
    extra: Json,
    pages: Vec<&'a Page>,
    paginator: Option<PagerView>,
    terms: Vec<TermView<'a>>,
    /// `template` from the section's or home page's `_index.md`.
    template: Option<&'a str>,
    candidates: &'static [&'static str],
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
    /// Previous and next page of each page, in its section's own order.
    links: HashMap<&'a str, (Option<&'a Page>, Option<&'a Page>)>,
    tags: HashMap<String, Vec<&'a Page>>,
    categories: HashMap<String, Vec<&'a Page>>,
}

impl<'a> PageIndex<'a> {
    fn new(pages: &'a [Page], sections: &'a [Listing]) -> Self {
        let mut grouped: HashMap<&str, Vec<&Page>> = HashMap::new();
        for page in pages {
            if let Some(section) = page.section.as_deref() {
                grouped.entry(section).or_default().push(page);
            }
        }
        let order: HashMap<&str, SortBy> = sections
            .iter()
            .map(|section| (section.key.as_str(), section.sort_by))
            .collect();
        let mut links: HashMap<&str, (Option<&Page>, Option<&Page>)> = HashMap::new();
        for (key, group) in &mut grouped {
            sort_pages(group, order.get(*key).copied().unwrap_or_default());
            for (position, page) in group.iter().enumerate() {
                let prev = position.checked_sub(1).map(|i| group[i]);
                let next = group.get(position + 1).copied();
                links.insert(page.url.as_str(), (prev, next));
            }
        }
        Self {
            sections: grouped,
            links,
            tags: term_index(pages, |page| &page.tags),
            categories: term_index(pages, |page| &page.categories),
        }
    }

    fn section(&self, key: &str) -> &[&'a Page] {
        self.sections.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The page before and after `url` in its section, if any.
    fn links(&self, url: &str) -> (Option<&'a Page>, Option<&'a Page>) {
        self.links.get(url).copied().unwrap_or((None, None))
    }

    fn tag(&self, key: &str) -> &[&'a Page] {
        self.tags.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    fn category(&self, key: &str) -> &[&'a Page] {
        self.categories.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Pages carrying one term, whichever taxonomy `kind` names.
    fn terms(&self, kind: &str, key: &str) -> &[&'a Page] {
        if kind == "tags" {
            self.tag(key)
        } else {
            self.category(key)
        }
    }
}

/// Group pages by the slug of each term in one taxonomy. Pages carry a handful
/// of terms, so a linear scan beats building a set per page just to drop
/// duplicates that share a slug.
fn term_index(
    pages: &[Page],
    pick: for<'p> fn(&'p Page) -> &'p [String],
) -> HashMap<String, Vec<&Page>> {
    let mut index: HashMap<String, Vec<&Page>> = HashMap::new();
    for page in pages {
        let mut seen: Vec<String> = Vec::new();
        for term in pick(page) {
            let slug = content::slugify(term);
            if !slug.is_empty() && !seen.contains(&slug) {
                seen.push(slug.clone());
                index.entry(slug).or_default().push(page);
            }
        }
    }
    index
}

/// Order one section's pages. `date` leaves the global newest-first order
/// alone; the other two re-sort, and a stable sort keeps the date order as the
/// tie-breaker for equal weights. The URL is the last key so a section renders
/// the same way on every machine.
fn sort_pages(pages: &mut [&Page], sort_by: SortBy) {
    match sort_by {
        SortBy::Date => {}
        SortBy::Weight => pages.sort_by_cached_key(|page| {
            (
                page.weight,
                std::cmp::Reverse(page.date),
                page.title.to_lowercase(),
                page.url.clone(),
            )
        }),
        SortBy::Title => {
            pages.sort_by_cached_key(|page| (page.title.to_lowercase(), page.url.clone()))
        }
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
    // Every worker forgets which directories exist, because the stale-output
    // sweep below may delete some of them.
    BUILD_GENERATION.fetch_add(1, Ordering::Relaxed);

    let phase = Instant::now();
    let content = content::load(&cfg, opts.drafts)?;
    let ms_load = phase.elapsed().as_millis();

    let static_files = static_files(&cfg)?;
    let static_paths = static_rels(&cfg, &static_files)?;
    // Content outputs and static files share one namespace: a page at
    // `/about/` and `static/about/index.html` would otherwise race, with the
    // static copy silently replacing the page.
    validate_urls(&cfg, &content, &static_paths)?;
    let has_static_style = static_paths.iter().any(|path| path == "style.css");

    let templates = Templates::load(&cfg.templates())?;
    let index = PageIndex::new(&content.pages, &content.sections);
    // A page's neighbours are part of what it renders, so they join its cache
    // key: adding or reordering a page re-renders the two pages around it.
    let links: Vec<(Option<&Page>, Option<&Page>)> = content
        .pages
        .iter()
        .map(|page| index.links(&page.url))
        .collect();
    let page_keys: Vec<u64> = content
        .pages
        .iter()
        .zip(&links)
        .map(|(page, (prev, next))| page_key(page, *prev, *next))
        .collect();
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
    // The previous manifest doubles as the "did anything change?" check below.
    let previous = if opts.clean {
        None
    } else {
        read_manifest(root, &output)
    };
    let mut expected = expected_outputs(&cfg, &content, &static_paths)?;
    if let Some(previous) = &previous {
        remove_stale(&output, previous, &expected)?;
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
        .zip(links.par_iter())
        .zip(page_keys.par_iter())
        .map(|((page, (prev, next)), key)| {
            render_page(
                &cfg,
                &templates,
                &site_value,
                &fingerprints,
                &default_name,
                &default_page,
                page,
                *prev,
                *next,
                *key,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let ms_pages = phase.elapsed().as_millis();

    content
        .pages
        .par_iter()
        .zip(&fresh)
        .try_for_each(|(page, fresh)| write_aliases(&cfg, page, fresh.is_some()))?;

    // A page's summary comes from its own Markdown, so it is only known once
    // the page has rendered (or read back from the fingerprint cache).
    let summaries: HashMap<&str, &str> = content
        .pages
        .iter()
        .zip(&fresh)
        .zip(&page_keys)
        .map(|((page, fresh), key)| {
            let summary = fresh
                .as_deref()
                .or_else(|| fingerprints.summary(&page.url, *key))
                .unwrap_or(&page.summary);
            (page.url.as_str(), summary)
        })
        .collect();

    // A card's fingerprint is identical in every list it appears in, so it is
    // computed once here instead of once per list.
    let card_keys: HashMap<&str, u64> = content
        .pages
        .iter()
        .map(|page| {
            let summary = summaries.get(page.url.as_str()).copied().unwrap_or("");
            (page.url.as_str(), card_key(page, summary))
        })
        .collect();

    let lists = build_lists(&content, &index);
    expected.extend(lists.iter().map(|list| output_rel(&list.url)));

    let phase = Instant::now();
    let list_names: Vec<String> = lists
        .iter()
        .map(|list| match list.template {
            // An explicit `template` wins and is reported by `render_list` if
            // it does not exist, the same way a page's override is.
            Some(name) => Ok(name.to_string()),
            None => templates.first(list.candidates).with_context(|| {
                format!(
                    "no template found for {} among {:?}",
                    list.url, list.candidates
                )
            }),
        })
        .collect::<Result<Vec<_>>>()?;
    let list_keys: Vec<u64> = lists
        .iter()
        .zip(&list_names)
        .map(|(list, name)| list_key(header, name, list, &card_keys))
        .collect();
    let list_rendered: Vec<bool> = lists
        .par_iter()
        .zip(list_names.par_iter())
        .zip(list_keys.par_iter())
        .map(|((list, name), key)| -> Result<bool> {
            if fingerprints.summary(&list.url, *key).is_some()
                && output_path(&cfg, &list.url).is_file()
            {
                return Ok(false);
            }
            render_list(&cfg, &templates, &site_value, list, &summaries, name)?;
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
    copy_assets(&cfg, &content.assets)?;
    if !has_static_style {
        write_text(&output, "style.css", theme::STYLE)?;
    }

    write_manifest(root, &output, &expected, previous.as_ref())?;

    // Only rewrite the fingerprint file when the page or list set changed.
    let expected_entries = content.pages.len() + lists.len();
    if fresh.iter().any(Option::is_some)
        || list_rendered.iter().any(|rendered| *rendered)
        || fingerprints.len() != expected_entries
    {
        let pages = content
            .pages
            .iter()
            .zip(fresh.iter())
            .zip(page_keys.iter())
            .map(|((page, fresh), key)| {
                let summary = fresh
                    .as_deref()
                    .or_else(|| fingerprints.summary(&page.url, *key))
                    .unwrap_or(&page.summary);
                (page.url.as_str(), *key, summary)
            });
        let lists = lists
            .iter()
            .zip(&list_keys)
            .map(|(list, key)| (list.url.as_str(), *key, ""));
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
pub(crate) fn expected_outputs(
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
    for (kind, _, terms) in content.taxonomies() {
        expected.insert(output_rel(&format!("/{kind}/")));
        for term in terms {
            expected.insert(output_rel(&term.url));
        }
    }

    expected.insert("rss.xml".into());
    expected.insert("sitemap.xml".into());
    expected.insert("robots.txt".into());
    expected.insert("style.css".into());
    if cfg.search_index {
        expected.insert("search-index.json".into());
    }
    expected.extend(static_paths.iter().cloned());
    for asset in &content.assets {
        expected.insert(output_rel(&asset.url));
    }
    Ok(expected)
}

fn section_page_count(section: &Listing, count: usize) -> usize {
    match section.paginate_by {
        Some(size) if size > 0 && count > size => count.div_ceil(size),
        _ => 1,
    }
}

fn build_lists<'a>(content: &'a Content, index: &PageIndex<'a>) -> Vec<ListSpec<'a>> {
    let mut lists: Vec<ListSpec<'a>> = Vec::new();
    let mut home_pages: Vec<&Page> = content.pages.iter().collect();
    sort_pages(&mut home_pages, content.home_sort_by);
    lists.push(ListSpec {
        title: &content.home.title,
        url: "/".to_string(),
        description: &content.home.description,
        body: &content.home.body,
        content: Arc::new(OnceLock::new()),
        extra: content.home.extra.clone(),
        pages: home_pages,
        paginator: None,
        terms: Vec::new(),
        template: content.home.template.as_deref(),
        candidates: &["index.html"],
    });
    for section in &content.sections {
        let all = index.section(&section.key);
        let page_size = section.paginate_by.unwrap_or(0);
        let total = section_page_count(section, all.len());
        // One cell per section: every paginated page renders the same body.
        let content_cell = Arc::new(OnceLock::new());
        if total <= 1 {
            lists.push(ListSpec {
                title: &section.title,
                url: section.url.clone(),
                description: &section.description,
                body: &section.body,
                content: content_cell,
                extra: section.extra.clone(),
                pages: all.to_vec(),
                paginator: None,
                terms: Vec::new(),
                template: section.template.as_deref(),
                candidates: &["section.html"],
            });
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
            lists.push(ListSpec {
                title: &section.title,
                url,
                description: &section.description,
                body: &section.body,
                content: Arc::clone(&content_cell),
                extra: section.extra.clone(),
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
                terms: Vec::new(),
                template: section.template.as_deref(),
                candidates: &["section.html"],
            });
        }
    }
    // A taxonomy gets an overview page (`/tags/`) listing its terms, the same
    // way Hugo and Zola do, followed by one listing per term.
    for (kind, title, terms) in content.taxonomies() {
        lists.push(ListSpec {
            title,
            url: format!("/{kind}/"),
            description: "",
            body: "",
            content: Arc::new(OnceLock::new()),
            extra: Json::Object(Default::default()),
            pages: Vec::new(),
            paginator: None,
            terms: terms
                .iter()
                .map(|term| TermView {
                    name: &term.title,
                    url: &term.url,
                    count: index.terms(kind, &term.key).len(),
                })
                .collect(),
            template: None,
            candidates: &["taxonomy.html"],
        });
    }
    for tag in &content.tags {
        lists.push(ListSpec {
            title: &tag.title,
            url: tag.url.clone(),
            description: "",
            body: "",
            content: Arc::new(OnceLock::new()),
            extra: Json::Object(Default::default()),
            pages: index.tag(&tag.key).to_vec(),
            paginator: None,
            terms: Vec::new(),
            template: None,
            candidates: &["tag.html", "section.html"],
        });
    }
    for category in &content.categories {
        lists.push(ListSpec {
            title: &category.title,
            url: category.url.clone(),
            description: "",
            body: "",
            content: Arc::new(OnceLock::new()),
            extra: Json::Object(Default::default()),
            pages: index.category(&category.key).to_vec(),
            paginator: None,
            terms: Vec::new(),
            template: None,
            candidates: &["category.html", "tag.html", "section.html"],
        });
    }
    lists.push(ListSpec {
        title: "404",
        url: "/404.html".to_string(),
        description: "",
        body: "",
        content: Arc::new(OnceLock::new()),
        extra: Json::Object(Default::default()),
        pages: Vec::new(),
        paginator: None,
        terms: Vec::new(),
        template: None,
        candidates: &["404.html"],
    });
    lists
}

fn write_aliases(cfg: &Config, page: &Page, changed: bool) -> Result<()> {
    for alias in &page.aliases {
        // An unchanged page keeps its redirect files: rewriting them on every
        // build only churns mtimes and slows no-op rebuilds down.
        if !changed && output_path(cfg, alias).is_file() {
            continue;
        }
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
    prev: Option<&Page>,
    next: Option<&Page>,
    key: u64,
) -> Result<Option<String>> {
    // The rendered file in `dist/` doubles as the cache: when the page source,
    // the templates, the site-wide context and the page's neighbours are
    // unchanged, skip Markdown, highlighting, templating and the write entirely.
    if fingerprints.summary(&page.url, key).is_some() && output_path(cfg, &page.url).is_file() {
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
        prev,
        next,
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
    list: &ListSpec<'_>,
    summaries: &HashMap<&str, &str>,
    name: &str,
) -> Result<()> {
    let url = &list.url;
    // Section bodies are rendered here, not while loading content: an
    // unchanged listing is skipped entirely, so its Markdown never has to be
    // parsed or highlighted on a rebuild.
    let content = if list.body.is_empty() {
        ""
    } else {
        list.content
            .get_or_init(|| markdown::render(list.body, cfg.highlight).html)
            .as_str()
    };
    let cards: Vec<CardView<'_>> = list
        .pages
        .iter()
        .map(|page| card(page, summaries))
        .collect();
    let view = ListView {
        title: list.title,
        url: list.url.as_str(),
        description: list.description,
        content,
        extra: &list.extra,
        pages: cards,
        paginator: list.paginator.as_ref(),
        terms: list.terms.clone(),
    };
    let ctx = context! {
        site => site.clone(),
        page => Serde(&view),
        pages => Serde(&view.pages),
    };
    let html = templates
        .render(name, ctx)
        .with_context(|| format!("rendering list {url}"))?;
    write_output(cfg, url, &html)
}

/// Hash everything a list page is rendered from. Lists are re-rendered only
/// when one of their cards, their own metadata, or the site context changes.
fn list_key(
    header: u64,
    template: &str,
    list: &ListSpec<'_>,
    card_keys: &HashMap<&str, u64>,
) -> u64 {
    let mut hash = cache::fingerprint(header, template.as_bytes());
    hash = cache::fingerprint(hash, list.url.as_bytes());
    hash = cache::fingerprint(hash, list.title.as_bytes());
    hash = cache::fingerprint(hash, list.description.as_bytes());
    hash = cache::fingerprint(hash, list.body.as_bytes());
    // Section extras are template data that no other fingerprint covers.
    if let Ok(extra) = serde_json::to_string(&list.extra) {
        hash = cache::fingerprint(hash, extra.as_bytes());
    }
    for term in &list.terms {
        hash = cache::fingerprint(hash, term.name.as_bytes());
        hash = cache::fingerprint(hash, term.url.as_bytes());
        hash = cache::fingerprint(hash, &(term.count as u64).to_le_bytes());
    }
    for page in &list.pages {
        // Each card carries a fingerprint of the fields it renders, computed
        // once per page, so a list hashes two words per card instead of every
        // field of every card.
        hash = cache::fingerprint(hash, page.url.as_bytes());
        let key = card_keys.get(page.url.as_str()).copied().unwrap_or(0);
        hash = cache::fingerprint(hash, &key.to_le_bytes());
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
    if cfg.minify {
        let body = crate::minify::minify_html(html);
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
    } else {
        // Writing the rendered string straight through avoids copying every
        // page's HTML into a second buffer just to hand it to `fs::write`.
        fs::write(&path, html.as_bytes()).with_context(|| format!("writing {}", path.display()))
    }
}

thread_local! {
    /// Directories this worker already created, tagged with the build that
    /// created them. Avoids one `mkdir` syscall per page while still keeping
    /// writes independent across threads. The tag matters: stale cleanup can
    /// delete a directory between two builds in the same process (`agno dev`),
    /// and a cache that outlived its build would skip recreating it.
    static KNOWN_DIRS: std::cell::RefCell<(u64, HashSet<PathBuf>)> =
        std::cell::RefCell::new((0, HashSet::new()));
}

/// Bumped once per build so every worker drops its directory cache.
static BUILD_GENERATION: AtomicU64 = AtomicU64::new(0);

fn ensure_dir(dir: &Path) -> Result<()> {
    let generation = BUILD_GENERATION.load(Ordering::Relaxed);
    let known = KNOWN_DIRS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.0 != generation {
            cell.0 = generation;
            cell.1.clear();
        }
        cell.1.contains(dir)
    });
    if known {
        return Ok(());
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    KNOWN_DIRS.with(|cell| {
        cell.borrow_mut().1.insert(dir.to_path_buf());
    });
    Ok(())
}

fn output_path(cfg: &Config, url: &str) -> PathBuf {
    cfg.output().join(output_rel(url))
}

/// Decode `%XX` escapes in a URL path. Returns `None` for malformed input.
pub(crate) fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn output_rel(url: &str) -> String {
    // Built straight into one buffer: this runs a handful of times per page per
    // build, and a `Vec` of segments plus a `join` doubled the allocations.
    let mut out = String::with_capacity(url.len() + 11);
    let mut empty = true;
    for segment in url
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
    {
        if !empty {
            out.push('/');
        }
        out.push_str(segment);
        empty = false;
    }
    if url.ends_with('/') || empty {
        if !empty {
            out.push('/');
        }
        out.push_str("index.html");
    }
    out
}

pub fn validate_outputs(cfg: &Config, content: &Content, static_paths: &[String]) -> Result<()> {
    validate_urls(cfg, content, static_paths)
}

#[derive(Default)]
struct Outputs {
    files: HashMap<String, String>,
    dirs: HashSet<String>,
}

impl Outputs {
    fn add(&mut self, url: &str, what: &str) -> Result<()> {
        self.add_key(&output_rel(url), url, what)
    }

    /// URL matching is case-insensitive, which is what a static host does.
    fn add_key(&mut self, key: &str, display: &str, what: &str) -> Result<()> {
        let key = key.to_ascii_lowercase();
        if let Some(existing) = self.files.insert(key.clone(), what.to_string()) {
            bail!("duplicate output URL {display}: {existing} and {what}");
        }
        if self.dirs.contains(&key) {
            bail!("output path conflict at {display}: another output is below it");
        }
        // Walk the parent directories without collecting them into a `Vec`:
        // this runs once per output path.
        for (index, _) in key.char_indices().filter(|(_, ch)| *ch == '/') {
            let parent = &key[..index];
            if let Some(existing) = self.files.get(parent) {
                bail!("output path conflict at {display}: {existing} is a file at {parent}");
            }
            self.dirs.insert(parent.to_string());
        }
        Ok(())
    }
}

fn validate_urls(cfg: &Config, content: &Content, static_paths: &[String]) -> Result<()> {
    let mut out = Outputs::default();
    let overridden: HashSet<&str> = static_paths.iter().map(String::as_str).collect();

    // A static file sitting on a generated root path intentionally replaces
    // that output: `static/style.css` is the documented theme override, and
    // the same trick is useful for robots.txt or a hand-written 404 page.
    let mut root = |url: &str, what: &str| -> Result<()> {
        if !overridden.contains(output_rel(url).as_str()) {
            out.add(url, what)?;
        }
        Ok(())
    };
    root("/", "home")?;
    root("/404.html", "404 page")?;
    root("/style.css", "default stylesheet")?;
    root("/rss.xml", "RSS feed")?;
    root("/sitemap.xml", "sitemap")?;
    root("/robots.txt", "robots.txt")?;
    if cfg.search_index {
        root("/search-index.json", "search index")?;
    }
    for page in &content.pages {
        out.add(&page.url, &format!("page {}", page.source.display()))?;
        for alias in &page.aliases {
            out.add(alias, &format!("alias of {}", page.url))?;
        }
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for page in &content.pages {
        if let Some(section) = page.section.as_deref() {
            *counts.entry(section).or_default() += 1;
        }
    }
    for section in &content.sections {
        out.add(&section.url, &format!("section {}", section.key))?;
        let count = counts.get(section.key.as_str()).copied().unwrap_or(0);
        for number in 2..=section_page_count(section, count) {
            out.add(
                &page_url(&section.url, number),
                &format!("page {number} of section {}", section.key),
            )?;
        }
    }
    for (kind, _, terms) in content.taxonomies() {
        out.add(&format!("/{kind}/"), &format!("{kind} index"))?;
        for term in terms {
            out.add(&term.url, &format!("{kind} term {}", term.title))?;
        }
    }
    for asset in &content.assets {
        out.add(&asset.url, &format!("asset {}", asset.source.display()))?;
    }
    // Static files come last so a collision reports the page or asset that
    // owns the path, rather than whichever happened to be added first.
    for rel in static_paths {
        out.add_key(rel, rel, &format!("static file {rel}"))?;
    }
    Ok(())
}

/// One manifest per output directory: switching `output_dir` and back must not
/// lose the record of what the earlier build wrote, or the stale sweep below
/// skips files that no longer have a source.
fn manifest_path(root: &Path, output: &Path) -> PathBuf {
    let tag = cache::fingerprint(0, output.to_string_lossy().as_bytes());
    root.join(".agno-cache")
        .join(format!("outputs-{tag:016x}.json"))
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

fn read_manifest(root: &Path, output: &Path) -> Option<Manifest> {
    let src = fs::read_to_string(manifest_path(root, output)).ok()?;
    serde_json::from_str(&src).ok()
}

fn write_manifest(
    root: &Path,
    output: &Path,
    expected: &HashSet<String>,
    previous: Option<&Manifest>,
) -> Result<()> {
    let mut files: Vec<String> = expected.iter().cloned().collect();
    files.sort();
    if let Some(previous) = previous
        && previous.output == output.to_string_lossy()
        && previous.files == files
    {
        return Ok(());
    }
    let path = manifest_path(root, output);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let manifest = Manifest {
        output: output.to_string_lossy().to_string(),
        files,
    };
    let body = serde_json::to_string(&manifest).context("serializing build manifest")?;
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn remove_stale(output: &Path, manifest: &Manifest, expected: &HashSet<String>) -> Result<()> {
    if manifest.output != output.to_string_lossy() {
        return Ok(());
    }
    for rel in &manifest.files {
        if expected.contains(rel)
            || rel.is_empty()
            || rel.starts_with('/')
            || rel.split('/').any(|part| part == "." || part == "..")
        {
            continue;
        }
        let path = output.join(rel);
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
    // A no-op rebuild regenerates identical feeds and manifests. Skipping the
    // write keeps their mtimes stable, which matters for rsync-style deploys.
    if let Ok(existing) = fs::read(&path)
        && existing == body.as_bytes()
    {
        return Ok(());
    }
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn site_view<'a>(cfg: &Config, content: &'a Content, index: &'a PageIndex<'a>) -> SiteView<'a> {
    let post_count = content
        .pages
        .iter()
        .filter(|page| page.date.is_some())
        .count();
    let tags = term_views(&content.tags, |key| index.tag(key));
    let categories = term_views(&content.categories, |key| index.category(key));
    let sections = content
        .sections
        .iter()
        // `site.sections` is the top-level navigation; nested sections still
        // render their own listings at their own URLs.
        .filter(|section| !section.key.contains('/'))
        .map(|section| SectionView {
            title: &section.title,
            url: &section.url,
            count: index.section(&section.key).len(),
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
        categories,
        sections,
    }
}

/// `site.tags` / `site.categories`: every term with its count and newest page.
fn term_views<'a>(
    terms: &'a [Listing],
    pages_of: impl Fn(&str) -> &'a [&'a Page],
) -> Vec<TagView<'a>> {
    terms
        .iter()
        .map(|term| {
            let pages = pages_of(&term.key);
            let latest = pages.first().copied();
            TagView {
                name: &term.title,
                url: &term.url,
                count: pages.len(),
                latest_title: latest.map(|page| page.title.as_str()).unwrap_or_default(),
                latest_url: latest.map(|page| page.url.as_str()).unwrap_or_default(),
            }
        })
        .collect()
}

fn page_view<'a>(
    page: &'a Page,
    html: &'a str,
    summary: &'a str,
    toc: &'a [TocEntry],
    toc_html: &'a str,
    prev: Option<&'a Page>,
    next: Option<&'a Page>,
) -> PageView<'a> {
    PageView {
        title: &page.title,
        url: &page.url,
        date: &page.date_text,
        date_display: &page.date_display,
        datetime: &page.datetime,
        updated: &page.updated_text,
        updated_display: &page.updated_display,
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
        prev: neighbor(prev),
        next: neighbor(next),
    }
}

fn neighbor(page: Option<&Page>) -> Option<NeighborView<'_>> {
    page.map(|page| NeighborView {
        title: &page.title,
        url: &page.url,
        date_display: &page.date_display,
    })
}

/// Cache key of a rendered page: its source fingerprint plus every field the
/// navigation exposes about the two pages it points at. A neighbour that is
/// added, removed, renamed or re-dated therefore re-renders this page.
fn page_key(page: &Page, prev: Option<&Page>, next: Option<&Page>) -> u64 {
    let mut hash = page.content_key;
    for neighbor in [prev, next] {
        let (url, title, date_display) = neighbor
            .map(|page| {
                (
                    page.url.as_str(),
                    page.title.as_str(),
                    page.date_display.as_str(),
                )
            })
            .unwrap_or(("", "", ""));
        hash = cache::fingerprint(hash, url.as_bytes());
        hash = cache::fingerprint(hash, title.as_bytes());
        hash = cache::fingerprint(hash, date_display.as_bytes());
    }
    hash
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

/// Hash of everything a card puts on the page. A list is re-rendered only when
/// one of its cards would actually look different, so editing a paragraph that
/// does not touch the summary leaves the home page and the tag pages alone.
fn card_key(page: &Page, summary: &str) -> u64 {
    let mut hash = cache::fingerprint(0, page.title.as_bytes());
    hash = cache::fingerprint(hash, page.url.as_bytes());
    hash = cache::fingerprint(hash, page.date_text.as_bytes());
    hash = cache::fingerprint(hash, page.date_display.as_bytes());
    hash = cache::fingerprint(hash, page.datetime.as_bytes());
    hash = cache::fingerprint(hash, describe(page, summary).as_bytes());
    hash = cache::fingerprint(hash, page.section.as_deref().unwrap_or_default().as_bytes());
    hash = cache::fingerprint(hash, &page.reading_minutes.to_le_bytes());
    for tag in &page.tags {
        hash = cache::fingerprint(hash, tag.as_bytes());
    }
    hash
}

fn rss(cfg: &Config, pages: &[Page], summaries: &HashMap<&str, &str>) -> String {
    let mut items = String::new();
    for page in pages
        .iter()
        .filter(|page| page.date.is_some())
        .take(cfg.rss_items)
    {
        let link = {
            let mut escaped = String::with_capacity(page.url.len() + cfg.base_url.len() + 8);
            push_url_escaped(&mut escaped, &cfg.absolute(&page.url));
            escaped
        };
        let pub_date = page.rss_date();
        let summary = summaries.get(page.url.as_str()).copied().unwrap_or("");
        items.push_str(&format!(
            "<item><title>{}</title><link>{link}</link><guid isPermaLink=\"true\">{link}</guid>{}<description>{}</description></item>",
            escape_html(&page.title),
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
    // Pages report their modification date when they have one; the home page
    // used to report its publication date, which disagreed with every other
    // entry in the same sitemap.
    push_url(
        &mut urls,
        cfg,
        "/",
        content.home.updated.or(content.home.date),
    );
    for page in &content.pages {
        push_url(&mut urls, cfg, &page.url, page.updated.or(page.date));
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for page in &content.pages {
        if let Some(section) = page.section.as_deref() {
            *counts.entry(section).or_default() += 1;
        }
    }
    for section in &content.sections {
        push_url(&mut urls, cfg, &section.url, None);
        // Paginated listings are real pages with their own canonical URLs.
        let count = counts.get(section.key.as_str()).copied().unwrap_or(0);
        for number in 2..=section_page_count(section, count) {
            push_url(&mut urls, cfg, &page_url(&section.url, number), None);
        }
    }
    for (kind, _, terms) in content.taxonomies() {
        push_url(&mut urls, cfg, &format!("/{kind}/"), None);
        for term in terms {
            push_url(&mut urls, cfg, &term.url, None);
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{urls}</urlset>\n"
    )
}

fn push_url(out: &mut String, cfg: &Config, url: &str, date: Option<OffsetDateTime>) {
    out.push_str("<url><loc>");
    push_url_escaped(out, &cfg.base_url);
    push_url_escaped(out, url);
    out.push_str("</loc>");
    if let Some(lastmod) = date.and_then(|value| value.format(&Rfc3339).ok()) {
        out.push_str("<lastmod>");
        out.push_str(&lastmod);
        out.push_str("</lastmod>");
    }
    out.push_str("</url>");
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Append a URL to an XML document: percent-encode the characters a URI cannot
/// carry literally, then XML-escape what is left.
///
/// A `url` frontmatter value may contain a space, and `<loc>https://host/my
/// page/</loc>` is not a valid sitemap entry even though it is well-formed XML.
fn push_url_escaped(out: &mut String, url: &str) {
    for ch in url.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch if ch.is_ascii_control()
                || matches!(ch, ' ' | '`' | '{' | '|' | '}' | '\\' | '^') =>
            {
                let mut buffer = [0u8; 4];
                for byte in ch.encode_utf8(&mut buffer).bytes() {
                    out.push('%');
                    out.push(HEX[usize::from(byte >> 4)] as char);
                    out.push(HEX[usize::from(byte & 0xf)] as char);
                }
            }
            ch => out.push(ch),
        }
    }
}

pub fn remove_all(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return fs::remove_file(path);
    }
    // `DirEntry::file_type` is served from readdir's `d_type` on Linux, so this
    // avoids one `statx` per entry: a 10,000-page site has 10,000 of them and
    // they dominate deleting the output tree.
    let entries: Vec<(PathBuf, bool)> = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let is_dir = match entry.file_type() {
                Ok(kind) => kind.is_dir(),
                Err(_) => fs::symlink_metadata(entry.path())
                    .map(|meta| meta.is_dir())
                    .unwrap_or(false),
            };
            (entry.path(), is_dir)
        })
        .collect();
    entries.par_iter().try_for_each(|(entry, is_dir)| {
        if *is_dir {
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

/// Relative output paths of every file in `static/`, for callers that need to
/// validate links or output collisions without copying anything.
pub(crate) fn static_paths(cfg: &Config) -> Result<Vec<String>> {
    static_rels(cfg, &static_files(cfg)?)
}

fn copy_static(cfg: &Config, files: &[PathBuf]) -> Result<()> {
    let src = cfg.static_dir();
    let dest = cfg.output();
    files.par_iter().try_for_each(|file| {
        let rel = file.strip_prefix(&src)?;
        let target = dest.join(rel);
        // `fs::copy` stamps the target with the current time, so a target that
        // is at least as new as its source with the same size is already a
        // faithful copy. Re-copying untouched assets was the bulk of a rebuild
        // on sites with images or fonts.
        if is_up_to_date(file, &target) {
            return Ok(());
        }
        if let Some(parent) = target.parent() {
            ensure_dir(parent)?;
        }
        fs::copy(file, &target)?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

fn copy_assets(cfg: &Config, assets: &[content::Asset]) -> Result<()> {
    let dest = cfg.output();
    assets.par_iter().try_for_each(|asset| {
        let target = dest.join(output_rel(&asset.url));
        if is_up_to_date(&asset.source, &target) {
            return Ok(());
        }
        if let Some(parent) = target.parent() {
            ensure_dir(parent)?;
        }
        fs::copy(&asset.source, &target)?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

fn is_up_to_date(source: &Path, target: &Path) -> bool {
    let (Ok(src), Ok(dst)) = (fs::metadata(source), fs::metadata(target)) else {
        return false;
    };
    if src.len() != dst.len() {
        return false;
    }
    match (src.modified(), dst.modified()) {
        (Ok(src), Ok(dst)) => dst >= src,
        _ => false,
    }
}

fn rel_string(path: &Path) -> String {
    content::rel_string(path)
}

#[cfg(test)]
mod tests {
    use super::{output_rel, remove_all};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn output_rel_maps_urls_to_files() {
        assert_eq!(output_rel("/"), "index.html");
        assert_eq!(output_rel("/blog/"), "blog/index.html");
        assert_eq!(output_rel("/blog/page/2/"), "blog/page/2/index.html");
        assert_eq!(output_rel("/blog/thing.html"), "blog/thing.html");
        assert_eq!(
            output_rel("/blog/thing.html/"),
            "blog/thing.html/index.html"
        );
        assert_eq!(output_rel("/a//b/"), "a/b/index.html");
        assert_eq!(output_rel("/a/../b/"), "a/b/index.html");
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("agno-rm-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn remove_all_deletes_a_nested_tree() {
        let root = scratch("nested");
        let dist = root.join("dist");
        fs::create_dir_all(dist.join("blog/post")).unwrap();
        fs::write(dist.join("blog/post/index.html"), "<p>hi</p>").unwrap();
        fs::write(dist.join("style.css"), "body{}").unwrap();

        remove_all(&dist).unwrap();

        assert!(!dist.exists());
        assert!(root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn remove_all_unlinks_symlinks_without_following_them() {
        let root = scratch("symlink");
        let outside = root.join("outside.txt");
        fs::write(&outside, "keep me").unwrap();
        let dist = root.join("dist");
        fs::create_dir_all(dist.join("nested")).unwrap();
        std::os::unix::fs::symlink(&outside, dist.join("nested/link.txt")).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), dist.join("dangling")).unwrap();

        remove_all(&dist).unwrap();

        assert!(!dist.exists());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me");
    }
}
