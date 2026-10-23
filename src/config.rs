use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub title: String,
    pub description: String,
    pub base_url: String,
    pub language: String,
    pub author: String,
    pub menu: Vec<MenuItem>,
    pub content_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub static_dir: PathBuf,
    pub output_dir: PathBuf,
    pub highlight: bool,
    pub rss_items: usize,
    pub search_index: bool,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct MenuItem {
    pub title: String,
    pub url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "My Site".into(),
            description: String::new(),
            base_url: "http://localhost:1111".into(),
            language: "en".into(),
            author: String::new(),
            menu: Vec::new(),
            content_dir: "content".into(),
            templates_dir: "templates".into(),
            static_dir: "static".into(),
            output_dir: "dist".into(),
            highlight: true,
            rss_items: 20,
            search_index: false,
            root: PathBuf::new(),
        }
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("agno.toml");
        let mut cfg: Config = if path.is_file() {
            let src = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            toml::from_str(&src).with_context(|| format!("parsing {}", path.display()))?
        } else {
            Self::default()
        };
        if cfg.title.trim().is_empty() {
            cfg.title = "My Site".into();
        }
        cfg.base_url = cfg.base_url.trim_end_matches('/').to_string();
        cfg.root = root.to_path_buf();
        Ok(cfg)
    }

    pub fn path(&self, p: impl AsRef<Path>) -> PathBuf {
        self.root.join(p)
    }

    pub fn content(&self) -> PathBuf {
        self.path(&self.content_dir)
    }

    pub fn templates(&self) -> PathBuf {
        self.path(&self.templates_dir)
    }

    pub fn static_dir(&self) -> PathBuf {
        self.path(&self.static_dir)
    }

    pub fn output(&self) -> PathBuf {
        self.path(&self.output_dir)
    }

    pub fn absolute(&self, url: &str) -> String {
        format!("{}{}", self.base_url, url)
    }
}
