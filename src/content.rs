use crate::config::Config;
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use time::format_description::well_known::{Rfc2822, Rfc3339};
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Page {
    pub source: PathBuf,
    pub url: String,
    pub slug: String,
    pub title: String,
    pub date: Option<OffsetDateTime>,
    pub updated: Option<OffsetDateTime>,
    pub date_text: String,
    pub date_display: String,
    pub datetime: String,
    pub updated_text: String,
    pub updated_display: String,
    pub reading_minutes: u32,
    pub words: u32,
    pub draft: bool,
    /// Sort position inside a section ordered with `sort_by = "weight"`.
    pub weight: i64,
    pub description: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    /// Extra URLs that should redirect to this page, already normalised.
    pub aliases: Vec<String>,
    pub template: Option<String>,
    pub extra: Value,
    /// Markdown body, kept until the page is rendered so unchanged pages can
    /// skip parsing and highlighting entirely.
    pub body: String,
    /// Source line the body starts on; `body` is stored with frontmatter and
    /// leading blank lines removed, so link reports need this offset.
    pub body_line: usize,
    /// Fingerprint of the page source; see [`crate::cache`].
    pub content_key: u64,
    pub section: Option<String>,
}

/// How a section orders the pages it lists. `date` keeps the historical
/// behaviour (newest first); `weight` is the documentation ordering, ascending,
/// with pages that share a weight falling back to date and then title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Date,
    Weight,
    Title,
}

impl SortBy {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "date" => Some(Self::Date),
            "weight" => Some(Self::Weight),
            "title" => Some(Self::Title),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub key: String,
    pub title: String,
    pub url: String,
    pub description: String,
    /// Markdown body of the section's `_index.md`, kept unrendered so an
    /// unchanged listing does not pay for Markdown parsing and highlighting on
    /// every build. It is rendered once, on demand, when the listing is
    /// actually written; `agno check` reads it directly.
    pub body: String,
    /// Source line the body starts on.
    pub body_line: usize,
    /// Source file behind this listing, empty for synthesized sections.
    pub source: PathBuf,
    /// Split this section's pages into `/section/page/N/` chunks of this size.
    pub paginate_by: Option<usize>,
    /// How this section orders its pages; `date` unless `_index.md` says
    /// otherwise.
    pub sort_by: SortBy,
    /// Template that renders this section instead of `section.html`.
    pub template: Option<String>,
    /// Remaining frontmatter, exposed to list templates as `page.extra`.
    pub extra: Value,
}

pub struct Content {
    pub home: Page,
    /// Ordering of the home page's list, from `content/_index.md`.
    pub home_sort_by: SortBy,
    pub pages: Vec<Page>,
    pub sections: Vec<Listing>,
    pub tags: Vec<Listing>,
    pub categories: Vec<Listing>,
    /// Files that live next to a page bundle's `index.md` and are copied to
    /// the bundle's output directory.
    pub assets: Vec<Asset>,
}

impl Content {
    /// The taxonomies that get an overview page (`/tags/`, `/categories/`),
    /// in a stable order: kind, title, and the terms themselves.
    pub fn taxonomies(&self) -> Vec<(&'static str, &'static str, &[Listing])> {
        let mut out: Vec<(&'static str, &'static str, &[Listing])> = Vec::new();
        if !self.tags.is_empty() {
            out.push(("tags", "Tags", &self.tags));
        }
        if !self.categories.is_empty() {
            out.push(("categories", "Categories", &self.categories));
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub source: PathBuf,
    /// URL the asset is served from, e.g. `/blog/photo/photo.jpg`.
    pub url: String,
}

impl Page {
    pub fn rss_date(&self) -> String {
        self.date
            .and_then(|d| d.format(&Rfc2822).ok())
            .unwrap_or_default()
    }

    /// Description used in feeds, cards and meta tags. `summary` is the
    /// effective summary: either the frontmatter value or the text agno
    /// extracted from the rendered first paragraph.
    pub fn plain_description<'a>(&'a self, summary: &'a str) -> &'a str {
        if self.description.is_empty() {
            summary
        } else {
            &self.description
        }
    }
}

impl Listing {
    pub fn empty(key: &str, title: &str, url: &str) -> Self {
        Self {
            key: key.to_string(),
            title: title.to_string(),
            url: url.to_string(),
            description: String::new(),
            body: String::new(),
            body_line: 0,
            source: PathBuf::new(),
            paginate_by: None,
            sort_by: SortBy::Date,
            template: None,
            extra: Value::Object(Default::default()),
        }
    }
}

pub fn load(cfg: &Config, include_drafts: bool) -> Result<Content> {
    let root = cfg.content();
    let mut page_paths: Vec<PathBuf> = Vec::new();
    let mut index_paths: HashMap<String, PathBuf> = HashMap::new();
    // Non-Markdown files are only emitted when they sit inside a page bundle,
    // but they have to be collected during the same walk that finds `index.md`.
    let mut asset_paths: Vec<(String, PathBuf)> = Vec::new();

    if root.is_dir() {
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path().to_path_buf();
            let rel_dir = path
                .parent()
                .and_then(|p| p.strip_prefix(&root).ok())
                .map(rel_string)
                .unwrap_or_default();
            if is_markdown(&name) {
                if is_index(&name) {
                    index_paths.insert(rel_dir, path);
                } else if !name.starts_with('_') {
                    page_paths.push(path);
                }
            } else {
                asset_paths.push((rel_dir, path));
            }
        }
    }

    let loaded: Result<Vec<Option<Page>>> = page_paths
        .par_iter()
        .map(|path| load_page(&root, path, include_drafts, &cfg.language))
        .collect();
    let mut pages: Vec<Page> = loaded?.into_iter().flatten().collect();
    // Sort keys are computed once per page instead of once per comparison. The
    // URL breaks ties so two pages with the same date and title keep the same
    // order on every machine, whatever order the directory walk returned.
    pages.sort_by_cached_key(|page| {
        (
            std::cmp::Reverse(page.date),
            page.title.to_lowercase(),
            page.url.clone(),
        )
    });

    let (home, home_sort_by) = load_home(cfg, &index_paths)?;

    // Every directory that directly contains content is a section, at any
    // depth. Pages report their parent directory as `section`, and an
    // `_index.md` can add a section landing page without child pages.
    let mut section_keys: BTreeSet<String> =
        pages.iter().filter_map(|p| p.section.clone()).collect();
    for key in index_paths.keys() {
        if !key.is_empty() {
            section_keys.insert(key.clone());
        }
    }
    let sections = section_keys
        .iter()
        .map(|key| {
            let url = section_url(key);
            load_listing(&index_paths, key, &url)
        })
        .collect::<Result<Vec<_>>>()?;

    // Tags and categories are grouped by their URL slug so "Rust" and "rust"
    // share one page. Both are the same kind of listing, just a different
    // frontmatter field and URL prefix.
    let tags = taxonomy_terms(&pages, |page| &page.tags, "tags");
    let categories = taxonomy_terms(&pages, |page| &page.categories, "categories");

    let assets = bundle_assets(&root, pages.iter(), asset_paths);

    Ok(Content {
        home,
        home_sort_by,
        pages,
        sections,
        tags,
        categories,
        assets,
    })
}

/// One listing per distinct term, ordered by slug so output is deterministic.
fn taxonomy_terms(pages: &[Page], pick: impl Fn(&Page) -> &[String], kind: &str) -> Vec<Listing> {
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    for page in pages {
        for term in pick(page) {
            let slug = slugify(term);
            if !slug.is_empty() {
                titles.entry(slug).or_insert_with(|| term.clone());
            }
        }
    }
    titles
        .into_iter()
        .map(|(slug, title)| Listing::empty(&slug, &title, &format!("/{kind}/{slug}/")))
        .collect()
}

/// Map each page bundle directory to the URL of its page, then attach every
/// non-Markdown file below that directory to the same output path.
fn bundle_assets<'a>(
    root: &Path,
    pages: impl Iterator<Item = &'a Page>,
    asset_paths: Vec<(String, PathBuf)>,
) -> Vec<Asset> {
    let bundle_urls: HashMap<String, String> = pages
        .filter_map(|page| {
            let name = page.source.file_name()?.to_str()?;
            if !is_bundle_index(name) {
                return None;
            }
            let dir = page
                .source
                .parent()?
                .strip_prefix(root)
                .ok()
                .map(rel_string)?;
            Some((dir, page.url.clone()))
        })
        .collect();
    if bundle_urls.is_empty() {
        return Vec::new();
    }

    let mut assets = Vec::new();
    for (rel_dir, source) in asset_paths {
        let Some((bundle_dir, url)) = nearest_bundle(&rel_dir, &bundle_urls) else {
            continue;
        };
        let rel = source
            .strip_prefix(root)
            .map(rel_string)
            .unwrap_or_default();
        let rel = rel
            .strip_prefix(bundle_dir)
            .unwrap_or(&rel)
            .trim_start_matches('/');
        if rel.is_empty() {
            continue;
        }
        assets.push(Asset {
            source,
            url: format!("{}/{rel}", url.trim_end_matches('/')),
        });
    }
    assets.sort_by(|a, b| a.url.cmp(&b.url));
    assets
}

fn nearest_bundle<'a>(
    rel_dir: &str,
    bundle_urls: &'a HashMap<String, String>,
) -> Option<(&'a str, &'a str)> {
    let mut dir = rel_dir;
    loop {
        if let Some((key, url)) = bundle_urls.get_key_value(dir) {
            return Some((key.as_str(), url.as_str()));
        }
        match dir.rsplit_once('/') {
            Some((head, _)) => dir = head,
            None => return None,
        }
    }
}

fn load_home(cfg: &Config, index_paths: &HashMap<String, PathBuf>) -> Result<(Page, SortBy)> {
    let Some(path) = index_paths.get("") else {
        let mut page = empty_page(cfg.title.clone(), "/".into());
        page.description = cfg.description.clone();
        return Ok((page, SortBy::Date));
    };
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;
    let sort_by = parse_sort_by(&fm, path)?;
    let mut page = empty_page(
        field_str(&fm, "title").unwrap_or_else(|| cfg.title.clone()),
        "/".into(),
    );
    page.source = path.clone();
    page.description = field_str(&fm, "description").unwrap_or_else(|| cfg.description.clone());
    page.summary = field_str(&fm, "summary").unwrap_or_default();
    page.template = field_str(&fm, "template");
    // The home page has no filename date, but its frontmatter dates are real:
    // they feed `<lastmod>` in the sitemap and `page.date` in the template.
    let date = field_str(&fm, "date").and_then(|s| parse_date(&s));
    let updated = field_str(&fm, "updated").and_then(|s| parse_date(&s));
    let (date_text, datetime, date_display, updated_text, updated_display) =
        date_fields(date, updated, &cfg.language);
    page.date = date;
    page.updated = updated;
    page.date_text = date_text;
    page.datetime = datetime;
    page.date_display = date_display;
    page.updated_text = updated_text;
    page.updated_display = updated_display;
    page.extra = fm;
    page.body_line = body_start_line(&src, body);
    page.body = body.to_string();
    Ok((page, sort_by))
}

fn parse_sort_by(fm: &Value, path: &Path) -> Result<SortBy> {
    match field_str(fm, "sort_by") {
        Some(value) => SortBy::parse(&value).with_context(|| {
            format!(
                "invalid sort_by \"{value}\" in {}: expected \"date\", \"weight\" or \"title\"",
                path.display()
            )
        }),
        None => Ok(SortBy::Date),
    }
}

fn load_listing(index_paths: &HashMap<String, PathBuf>, key: &str, url: &str) -> Result<Listing> {
    let Some(path) = index_paths.get(key) else {
        return Ok(Listing::empty(key, &section_title(key), url));
    };
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;
    let sort_by = parse_sort_by(&fm, path)?;
    Ok(Listing {
        key: key.to_string(),
        title: field_str(&fm, "title").unwrap_or_else(|| section_title(key)),
        url: url.to_string(),
        description: field_str(&fm, "description").unwrap_or_default(),
        body_line: body_start_line(&src, body),
        body: body.to_string(),
        source: path.clone(),
        paginate_by: field_usize(&fm, "paginate_by").filter(|size| *size > 0),
        sort_by,
        template: field_str(&fm, "template"),
        extra: fm,
    })
}

fn load_page(
    root: &Path,
    path: &Path,
    include_drafts: bool,
    language: &str,
) -> Result<Option<Page>> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;

    let draft = field_bool(&fm, "draft").unwrap_or(false);
    if draft && !include_drafts {
        return Ok(None);
    }

    let rel = path
        .strip_prefix(root)
        .map(rel_string)
        .unwrap_or_else(|_| rel_string(path));
    let rel_no_ext = match rel.rsplit_once('.') {
        Some((head, _)) => head.to_string(),
        None => rel.clone(),
    };
    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let is_bundle_index = file_stem.eq_ignore_ascii_case("index");
    let parent = rel_no_ext
        .rsplit_once('/')
        .map(|(head, _)| head.to_string())
        .unwrap_or_default();

    let (filename_date, stem_clean) = split_date_prefix(&file_stem);

    let date = field_str(&fm, "date")
        .and_then(|s| parse_date(&s))
        .or_else(|| filename_date.and_then(parse_date));
    let updated = field_str(&fm, "updated").and_then(|s| parse_date(&s));
    let (date_text, datetime, date_display, updated_text, updated_display) =
        date_fields(date, updated, language);
    let word_count = body.split_whitespace().count();
    let reading_minutes = if word_count == 0 {
        0
    } else {
        word_count.div_ceil(200) as u32
    };

    let default_slug = if is_bundle_index {
        parent
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("index")
            .to_string()
    } else {
        stem_clean.to_string()
    };

    let slug = field_str(&fm, "slug")
        .map(|s| slugify(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let slug = slugify(&default_slug);
            if slug.is_empty() { "page".into() } else { slug }
        });

    let default_url = if is_bundle_index {
        let mut segments: Vec<String> = parent
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(url_segment)
            .collect();
        if segments.is_empty() {
            "/".to_string()
        } else {
            segments.pop();
            segments.push(slug.clone());
            format!("/{}/", segments.join("/"))
        }
    } else {
        let parent = rel_no_ext
            .rsplit_once('/')
            .map(|(head, _)| head)
            .unwrap_or("");
        let mut segments: Vec<String> = parent
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(url_segment)
            .collect();
        segments.push(slug.clone());
        format!("/{}/", segments.join("/"))
    };
    let url = match field_str(&fm, "url") {
        Some(url) => {
            normalize_url(&url).with_context(|| format!("invalid url in {}", path.display()))?
        }
        None => default_url,
    };
    let title = field_str(&fm, "title").unwrap_or_else(|| humanize(stem_clean));

    let summary = field_str(&fm, "summary").unwrap_or_default();
    let description = field_str(&fm, "description").unwrap_or_default();

    // A bundle's page lives at `/dir/`, so its section is the bundle's parent
    // directory; a regular page belongs to the directory it sits in.
    let section_source = if is_bundle_index {
        parent.rsplit_once('/').map(|(head, _)| head).unwrap_or("")
    } else {
        rel_no_ext
            .rsplit_once('/')
            .map(|(head, _)| head)
            .unwrap_or("")
    };
    let section = (!section_source.is_empty()).then(|| section_source.to_string());

    // An alias that points at the page itself, or repeats an earlier one, is a
    // no-op rather than a conflict: it would otherwise fail the build with a
    // duplicate-output error for something the author cannot see.
    let mut deduped: Vec<String> = Vec::new();
    for alias in parse_aliases(&fm, path)? {
        if alias != url && !deduped.contains(&alias) {
            deduped.push(alias);
        }
    }

    Ok(Some(Page {
        source: path.to_path_buf(),
        url,
        slug,
        title,
        date,
        updated,
        date_text,
        date_display,
        datetime,
        updated_text,
        updated_display,
        reading_minutes,
        words: word_count as u32,
        draft,
        weight: field_i64(&fm, "weight").unwrap_or(0),
        description,
        summary,
        tags: term_list(&fm, "tags"),
        categories: term_list(&fm, "categories"),
        aliases: deduped,
        template: field_str(&fm, "template"),
        extra: fm,
        body_line: body_start_line(&src, body),
        body: body.to_string(),
        content_key: crate::cache::fingerprint(0, src.as_bytes()),
        section,
    }))
}

/// The five rendered date strings a page exposes, derived once from its
/// `date` and `updated` values.
fn date_fields(
    date: Option<OffsetDateTime>,
    updated: Option<OffsetDateTime>,
    language: &str,
) -> (String, String, String, String, String) {
    (
        date.map(ymd).unwrap_or_default(),
        date.and_then(|d| d.format(&Rfc3339).ok())
            .unwrap_or_default(),
        date.map(|d| format_date_display(d, language))
            .unwrap_or_default(),
        updated.map(ymd).unwrap_or_default(),
        updated
            .map(|d| format_date_display(d, language))
            .unwrap_or_default(),
    )
}

fn empty_page(title: String, url: String) -> Page {
    Page {
        source: PathBuf::new(),
        url,
        slug: String::new(),
        title,
        date: None,
        updated: None,
        date_text: String::new(),
        date_display: String::new(),
        datetime: String::new(),
        updated_text: String::new(),
        updated_display: String::new(),
        reading_minutes: 0,
        words: 0,
        draft: false,
        weight: 0,
        description: String::new(),
        summary: String::new(),
        tags: Vec::new(),
        categories: Vec::new(),
        aliases: Vec::new(),
        template: None,
        extra: Value::Object(Default::default()),
        body_line: 0,
        body: String::new(),
        content_key: 0,
        section: None,
    }
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_string_lossy().starts_with('.') && entry.depth() > 0
}

/// True when the file ends in `.md` or `.markdown`, in any case: `POST.MD` is
/// an ordinary name on a case-insensitive filesystem, and treating it as an
/// asset made the page disappear without a word.
fn is_markdown(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"),
        None => false,
    }
}

fn is_index(name: &str) -> bool {
    is_named(name, "_index")
}

fn is_bundle_index(name: &str) -> bool {
    is_named(name, "index")
}

/// `_index.md` for a given stem, with a Markdown extension in any case.
fn is_named(name: &str, stem: &str) -> bool {
    match name.rsplit_once('.') {
        Some((head, ext)) => {
            head.eq_ignore_ascii_case(stem)
                && (ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
        }
        None => false,
    }
}

/// The source line that `body` starts on. `split_frontmatter` returns a
/// subslice of the file, so the prefix length gives the line offset.
fn body_start_line(src: &str, body: &str) -> usize {
    let offset = body.as_ptr() as usize - src.as_ptr() as usize;
    if offset > src.len() {
        return 1;
    }
    1 + src[..offset].bytes().filter(|byte| *byte == b'\n').count()
}

pub fn rel_string(path: &Path) -> String {
    let mut out = String::with_capacity(path.as_os_str().len() + 8);
    for component in path.components() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

pub fn split_frontmatter(src: &str) -> Result<(Value, &str)> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);

    for (delim, is_toml) in [("+++", true), ("---", false)] {
        if let Some(rest) = src.strip_prefix(delim) {
            let rest = rest
                .strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest);
            let needle = if is_toml { "\n+++" } else { "\n---" };
            // The usual case: the closing delimiter is on a later line. An
            // empty block (`---` immediately followed by `---`) puts it on the
            // first one, which `find` cannot see because there is no leading
            // newline to match.
            let pos = rest.find(needle).or_else(|| {
                let after = rest.strip_prefix(delim)?;
                (after.is_empty() || after.starts_with(['\r', '\n'])).then_some(0)
            });
            if let Some(pos) = pos {
                // The delimiter search lands on the LF; a CRLF file leaves a
                // trailing CR on the frontmatter, which TOML rejects.
                let front = rest[..pos].trim_end_matches('\r');
                // A file that ends right after the closing delimiter has no
                // body at all, hence the fallback.
                let body = rest.get(pos + delim.len() + 1..).unwrap_or("");
                let value: Value = if is_toml {
                    let parsed: toml::Value =
                        toml::from_str(front).with_context(|| "invalid TOML frontmatter")?;
                    toml_to_json(parsed)
                } else {
                    serde_yaml_ng::from_str(front).with_context(|| "invalid YAML frontmatter")?
                };
                // A file that merely starts with `---` and has another `---`
                // further down parses as a YAML scalar or sequence. Treating
                // that as empty frontmatter silently dropped the text between
                // the two rules, so it is an error instead. An empty block
                // (`---\n---`) is still fine: there is nothing to lose.
                let value = match value {
                    Value::Object(_) => value,
                    Value::Null => Value::Object(Default::default()),
                    other => bail!(
                        "frontmatter must be a mapping of keys to values, found {}",
                        json_kind(&other)
                    ),
                };
                return Ok((value, body.trim_start_matches(['\r', '\n'])));
            }
        }
    }
    Ok((Value::Object(Default::default()), src))
}

/// Human name for a frontmatter value's shape, used in error messages.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a table",
    }
}

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(i.into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(key, value)| (key, toml_to_json(value)))
                .collect(),
        ),
    }
}

fn field<'a>(fm: &'a Value, key: &str) -> Option<&'a Value> {
    fm.get(key)
}

fn field_str(fm: &Value, key: &str) -> Option<String> {
    match field(fm, key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn field_bool(fm: &Value, key: &str) -> Option<bool> {
    match field(fm, key)? {
        Value::Bool(b) => b.to_string().parse().ok(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn field_usize(fm: &Value, key: &str) -> Option<usize> {
    match field(fm, key)? {
        Value::Number(n) => n.as_u64().map(|n| n as usize),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn field_i64(fm: &Value, key: &str) -> Option<i64> {
    match field(fm, key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_aliases(fm: &Value, path: &Path) -> Result<Vec<String>> {
    field_string_list(fm, "aliases")
        .into_iter()
        .map(|alias| {
            normalize_url(&alias)
                .with_context(|| format!("invalid alias {alias:?} in {}", path.display()))
        })
        .collect()
}

fn field_string_list(fm: &Value, key: &str) -> Vec<String> {
    match field(fm, key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Tags or categories as written, minus terms that would share a term page.
///
/// "Rust" and "rust" slugify to the same URL, so keeping both would render two
/// links to one page; a term that slugifies to nothing has no page at all and
/// would render `/tags//`.
fn term_list(fm: &Value, key: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut slugs: Vec<String> = Vec::new();
    for term in field_string_list(fm, key) {
        let slug = slugify(&term);
        if slug.is_empty() || slugs.contains(&slug) {
            continue;
        }
        slugs.push(slug);
        terms.push(term);
    }
    terms
}

/// `YYYY-MM-DD`, the machine-readable date used by `<time datetime>` and feeds.
fn ymd(date: OffsetDateTime) -> String {
    date.format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

pub fn format_date_display(date: OffsetDateTime, language: &str) -> String {
    let month = date.month() as u8;
    if language.to_ascii_lowercase().starts_with("tr") {
        let names = [
            "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül",
            "Ekim", "Kasım", "Aralık",
        ];
        return format!(
            "{} {} {}",
            date.day(),
            names[(month - 1) as usize],
            date.year()
        );
    }
    let names = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!(
        "{} {}, {}",
        names[(month - 1) as usize],
        date.day(),
        date.year()
    )
}

pub fn parse_date(input: &str) -> Option<OffsetDateTime> {
    let input = input.trim();
    if let Ok(dt) = OffsetDateTime::parse(input, &Rfc3339) {
        return Some(dt);
    }
    if let Ok(date) = Date::parse(input, format_description!("[year]-[month]-[day]")) {
        return Some(date.midnight().assume_utc());
    }
    for fmt in [
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"),
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]"),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]"),
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]"),
    ] {
        if let Ok(dt) = PrimitiveDateTime::parse(input, fmt) {
            return Some(dt.assume_utc());
        }
    }
    None
}

fn split_date_prefix(stem: &str) -> (Option<&str>, &str) {
    let bytes = stem.as_bytes();
    if bytes.len() > 11
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        (Some(&stem[..10]), &stem[11..])
    } else {
        (None, stem)
    }
}

fn normalize_url(url: &str) -> Result<String> {
    let url = url.trim();
    if url.is_empty() {
        bail!("url cannot be empty");
    }
    if url.contains("://") || url.starts_with("//") {
        bail!("url must be a site-relative path");
    }
    if url.contains(['\\', '?', '#']) {
        bail!("url contains unsupported characters");
    }
    let url = if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{url}")
    };
    if url
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        bail!("url must not contain . or .. segments");
    }
    if url.ends_with('/') || url.ends_with(".html") {
        Ok(url)
    } else {
        Ok(format!("{url}/"))
    }
}

fn url_segment(input: &str) -> String {
    let slug = slugify(input);
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

/// `/docs/guide/` for a nested section key `docs/guide`.
fn section_url(key: &str) -> String {
    let mut url = String::from("/");
    for segment in key.split('/').filter(|segment| !segment.is_empty()) {
        url.push_str(&url_segment(segment));
        url.push('/');
    }
    url
}

fn section_title(key: &str) -> String {
    humanize(key.rsplit('/').next().unwrap_or(key))
}

pub fn humanize(input: &str) -> String {
    let spaced = input.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let mapped = match ch {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'I' | 'İ' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            'â' | 'Â' => 'a',
            'î' | 'Î' => 'i',
            'û' | 'Û' => 'u',
            _ => ch,
        };
        if mapped.is_alphanumeric() {
            out.extend(mapped.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_frontmatter_and_body() {
        let (fm, body) = split_frontmatter(
            "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"a\", \"b\"]\n+++\n\n# Body\n",
        )
        .unwrap();
        assert_eq!(field_str(&fm, "title").as_deref(), Some("Hello"));
        assert_eq!(
            parse_date(&field_str(&fm, "date").unwrap()).unwrap().year(),
            2024
        );
        assert_eq!(field_string_list(&fm, "tags"), vec!["a", "b"]);
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn parses_yaml_frontmatter() {
        let (fm, body) = split_frontmatter("---\ntitle: Hi\ndraft: true\n---\nBody").unwrap();
        assert_eq!(field_str(&fm, "title").as_deref(), Some("Hi"));
        assert_eq!(field_bool(&fm, "draft"), Some(true));
        assert_eq!(body, "Body");
    }

    #[test]
    fn no_frontmatter_is_content() {
        let (fm, body) = split_frontmatter("just text").unwrap();
        assert!(fm.as_object().unwrap().is_empty());
        assert_eq!(body, "just text");
    }

    #[test]
    fn an_empty_frontmatter_block_is_allowed() {
        let (fm, body) = split_frontmatter("---\n---\n\nBody").unwrap();
        assert!(fm.as_object().unwrap().is_empty());
        assert_eq!(body, "Body");
    }

    #[test]
    fn frontmatter_that_is_not_a_mapping_is_rejected() {
        // A document that starts with a rule and has another one further down
        // is not frontmatter; silently dropping the text between them was the
        // bug this guards.
        let err = split_frontmatter("---\nSome text\n---\nMore\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("must be a mapping"), "{err}");
        let err = split_frontmatter("---\n- a\n- b\n---\nMore\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("a list"), "{err}");
    }

    #[test]
    fn slugify_handles_turkish() {
        assert_eq!(slugify("Çığır Açan Şeyler"), "cigir-acan-seyler");
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
    }

    #[test]
    fn formats_display_dates_in_english_and_turkish() {
        let date = parse_date("2026-07-02T14:46:49+04:00").unwrap();
        assert_eq!(format_date_display(date, "en"), "July 2, 2026");
        assert_eq!(format_date_display(date, "tr"), "2 Temmuz 2026");
    }

    #[test]
    fn date_prefix_is_split() {
        assert_eq!(
            split_date_prefix("2024-05-01-post"),
            (Some("2024-05-01"), "post")
        );
        assert_eq!(split_date_prefix("post"), (None, "post"));
    }

    #[test]
    fn markdown_names_are_matched_without_regard_to_case() {
        assert!(is_markdown("post.md"));
        assert!(is_markdown("POST.MD"));
        assert!(is_markdown("Post.Markdown"));
        assert!(!is_markdown("post.txt"));
        assert!(!is_markdown("post"));

        assert!(is_index("_index.md"));
        assert!(is_index("_INDEX.MD"));
        assert!(!is_index("index.md"));

        assert!(is_bundle_index("index.md"));
        assert!(is_bundle_index("Index.MD"));
        assert!(!is_bundle_index("_index.md"));
        assert!(!is_bundle_index("indexes.md"));
    }
}
