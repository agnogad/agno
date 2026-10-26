use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

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
    /// Collapse HTML whitespace and strip comments on write.
    pub minify: bool,
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
            minify: false,
            root: PathBuf::new(),
        }
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        // Without this, `agno build -r typo` creates the directory and an empty
        // site inside it instead of reporting the mistake.
        if !root.is_dir() {
            bail!("site root {} does not exist", root.display());
        }
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

    /// Reject output paths that could delete the site or another input directory.
    pub fn validate(&self) -> Result<()> {
        // Canonical links, the sitemap and the feed all concatenate this with
        // a page path, so a bare host name silently produces relative URLs in
        // every one of them.
        if !self.base_url.is_empty()
            && !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://"))
        {
            bail!(
                "base_url must be an absolute http(s) URL, found \"{}\"",
                self.base_url
            );
        }

        let root = normalize(&self.root);
        let output = normalize(&self.output());

        if output == root {
            bail!("output_dir must not be the site root");
        }
        if root.as_os_str().is_empty() {
            if output.is_absolute() || output.starts_with("..") {
                bail!("output_dir must stay inside the site root");
            }
        } else if !output.starts_with(&root) {
            bail!("output_dir must stay inside the site root");
        }

        for (name, dir) in [
            ("content_dir", &self.content_dir),
            ("templates_dir", &self.templates_dir),
            ("static_dir", &self.static_dir),
        ] {
            let dir = normalize(&self.path(dir));
            if output.starts_with(&dir) || dir.starts_with(&output) {
                bail!("output_dir must not overlap {name}");
            }
        }

        let output_path = self.output();
        if let Ok(meta) = std::fs::symlink_metadata(&output_path)
            && meta.file_type().is_symlink()
        {
            bail!("output_dir must not be a symlink");
        }
        if let (Ok(root), Ok(parent)) = (
            self.root.canonicalize(),
            output_path.parent().unwrap_or(&output_path).canonicalize(),
        ) && !parent.starts_with(&root)
        {
            bail!("output_dir must stay inside the site root");
        }

        Ok(())
    }

    pub fn absolute(&self, url: &str) -> String {
        format!("{}{}", self.base_url, url)
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &str, output: &str) -> Config {
        Config {
            root: root.into(),
            output_dir: output.into(),
            ..Config::default()
        }
    }

    #[test]
    fn output_dir_must_stay_inside_the_site() {
        assert!(config(".", "dist").validate().is_ok());
        assert!(config(".", ".").validate().is_err());
        assert!(config(".", "../outside").validate().is_err());
        assert!(config(".", "content").validate().is_err());

        // An absolute path is outside the site by definition. `temp_dir` is
        // absolute on every platform; a literal `/tmp/...` is not, because
        // Windows needs a drive prefix before it counts as absolute.
        let outside = std::env::temp_dir().join("agno-outside");
        assert!(
            config(".", outside.to_str().unwrap()).validate().is_err(),
            "{}",
            outside.display()
        );
    }
}
