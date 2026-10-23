use crate::config::Config;
use crate::markdown;
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
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
    pub reading_minutes: u32,
    pub draft: bool,
    pub description: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub template: Option<String>,
    pub extra: Value,
    pub html: String,
    pub section: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub key: String,
    pub title: String,
    pub url: String,
    pub description: String,
    pub content: String,
}

pub struct Content {
    pub home: Page,
    pub pages: Vec<Page>,
    pub sections: Vec<Listing>,
    pub tags: Vec<Listing>,
}

impl Page {
    pub fn rss_date(&self) -> String {
        self.date
            .and_then(|d| d.format(&Rfc2822).ok())
            .unwrap_or_default()
    }

    pub fn plain_description(&self) -> &str {
        if self.description.is_empty() {
            &self.summary
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
            content: String::new(),
        }
    }
}

pub fn load(cfg: &Config, include_drafts: bool) -> Result<Content> {
    let root = cfg.content();
    let mut page_paths: Vec<PathBuf> = Vec::new();
    let mut index_paths: HashMap<String, PathBuf> = HashMap::new();

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
            if !is_markdown(&name) {
                continue;
            }
            let path = entry.path().to_path_buf();
            let rel_dir = path
                .parent()
                .and_then(|p| p.strip_prefix(&root).ok())
                .map(rel_string)
                .unwrap_or_default();
            if is_index(&name) {
                index_paths.insert(rel_dir, path);
            } else if !name.starts_with('_') {
                page_paths.push(path);
            }
        }
    }

    let loaded: Result<Vec<Option<Page>>> = page_paths
        .par_iter()
        .map(|path| load_page(&root, path, include_drafts, cfg.highlight, &cfg.language))
        .collect();
    let mut pages: Vec<Page> = loaded?.into_iter().flatten().collect();
    pages.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    let home = load_home(cfg, &index_paths)?;

    let section_keys: BTreeSet<String> = pages.iter().filter_map(|p| p.section.clone()).collect();
    let sections = section_keys
        .iter()
        .map(|key| load_listing(cfg, &index_paths, key, &format!("/{key}/")))
        .collect::<Result<Vec<_>>>()?;

    let mut tag_names: BTreeSet<String> = BTreeSet::new();
    for page in &pages {
        for tag in &page.tags {
            tag_names.insert(tag.clone());
        }
    }
    let tags = tag_names
        .into_iter()
        .map(|tag| Listing {
            key: tag.clone(),
            title: tag.clone(),
            url: format!("/tags/{}/", slugify(&tag)),
            description: String::new(),
            content: String::new(),
        })
        .collect();

    Ok(Content {
        home,
        pages,
        sections,
        tags,
    })
}

fn load_home(cfg: &Config, index_paths: &HashMap<String, PathBuf>) -> Result<Page> {
    let Some(path) = index_paths.get("") else {
        let mut page = empty_page(cfg.title.clone(), "/".into());
        page.description = cfg.description.clone();
        return Ok(page);
    };
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)?;
    let (html, summary) = markdown::render(body, cfg.highlight);
    let mut page = empty_page(
        field_str(&fm, "title").unwrap_or_else(|| cfg.title.clone()),
        "/".into(),
    );
    page.source = path.clone();
    page.description = field_str(&fm, "description").unwrap_or_else(|| cfg.description.clone());
    page.summary = field_str(&fm, "summary").unwrap_or(summary);
    page.extra = fm;
    page.html = html;
    Ok(page)
}

fn load_listing(
    cfg: &Config,
    index_paths: &HashMap<String, PathBuf>,
    key: &str,
    url: &str,
) -> Result<Listing> {
    let Some(path) = index_paths.get(key) else {
        return Ok(Listing::empty(key, &humanize(key), url));
    };
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)?;
    let (html, _) = markdown::render(body, cfg.highlight);
    Ok(Listing {
        key: key.to_string(),
        title: field_str(&fm, "title").unwrap_or_else(|| humanize(key)),
        url: url.to_string(),
        description: field_str(&fm, "description").unwrap_or_default(),
        content: html,
    })
}

fn load_page(
    root: &Path,
    path: &Path,
    include_drafts: bool,
    highlight: bool,
    language: &str,
) -> Result<Option<Page>> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (fm, body) = split_frontmatter(&src)?;

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
    let is_bundle_index = file_stem == "index";
    let parent = rel_no_ext
        .rsplit_once('/')
        .map(|(head, _)| head.to_string())
        .unwrap_or_default();

    let (filename_date, stem_clean) = split_date_prefix(&file_stem);

    let date = field_str(&fm, "date")
        .and_then(|s| parse_date(&s))
        .or_else(|| filename_date.and_then(parse_date));
    let updated = field_str(&fm, "updated").and_then(|s| parse_date(&s));
    let date_text = date
        .map(|d| {
            d.format(format_description!("[year]-[month]-[day]"))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let datetime = date
        .and_then(|d| d.format(&Rfc3339).ok())
        .unwrap_or_default();
    let date_display = date
        .map(|d| format_date_display(d, language))
        .unwrap_or_default();
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

    let default_url = if is_bundle_index {
        if parent.is_empty() {
            "/".to_string()
        } else {
            format!("/{parent}/")
        }
    } else {
        format!("/{rel_no_ext}/")
    };
    let url = field_str(&fm, "url")
        .map(|u| normalize_url(&u))
        .unwrap_or(default_url);
    let slug = field_str(&fm, "slug").unwrap_or_else(|| slugify(&default_slug));
    let title = field_str(&fm, "title").unwrap_or_else(|| humanize(stem_clean));

    let (html, auto_summary) = markdown::render(body, highlight);
    let summary = field_str(&fm, "summary").unwrap_or(auto_summary);
    let description = field_str(&fm, "description").unwrap_or_else(|| summary.clone());

    let section = if rel_no_ext.contains('/') {
        rel_no_ext.split('/').next().map(|s| s.to_string())
    } else {
        None
    };

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
        reading_minutes,
        draft,
        description,
        summary,
        tags: field_string_list(&fm, "tags"),
        categories: field_string_list(&fm, "categories"),
        template: field_str(&fm, "template"),
        extra: fm,
        html,
        section,
    }))
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
        reading_minutes: 0,
        draft: false,
        description: String::new(),
        summary: String::new(),
        tags: Vec::new(),
        categories: Vec::new(),
        template: None,
        extra: Value::Object(Default::default()),
        html: String::new(),
        section: None,
    }
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_string_lossy().starts_with('.') && entry.depth() > 0
}

fn is_markdown(name: &str) -> bool {
    name.ends_with(".md") || name.ends_with(".markdown")
}

fn is_index(name: &str) -> bool {
    name == "_index.md" || name == "_index.markdown"
}

fn rel_string(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn split_frontmatter(src: &str) -> Result<(Value, &str)> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);

    for (delim, is_toml) in [("+++", true), ("---", false)] {
        if let Some(rest) = src.strip_prefix(delim) {
            let rest = rest.strip_prefix('\n').unwrap_or(rest);
            if let Some(pos) = rest.find(&format!("\n{delim}")) {
                let front = &rest[..pos];
                let body = &rest[pos + delim.len() + 1..];
                let value: Value = if is_toml {
                    let parsed: toml::Value =
                        toml::from_str(front).with_context(|| "invalid TOML frontmatter")?;
                    toml_to_json(parsed)
                } else {
                    serde_yaml_ng::from_str(front).with_context(|| "invalid YAML frontmatter")?
                };
                let value = match value {
                    Value::Object(_) => value,
                    _ => Value::Object(Default::default()),
                };
                return Ok((value, body.trim_start_matches('\n')));
            }
        }
    }
    Ok((Value::Object(Default::default()), src))
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

fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let url = if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{url}")
    };
    if url.ends_with('/') || url.ends_with(".html") {
        url
    } else {
        format!("{url}/")
    }
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
}
