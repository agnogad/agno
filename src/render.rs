use crate::content::slugify;
use crate::theme;
use anyhow::{Context, Result};
use minijinja::{AutoEscape, Environment, Template, Value, path_loader};
use std::path::Path;

pub struct Templates {
    env: Environment<'static>,
}

impl Templates {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut env = Environment::new();
        // Templates are loaded once per build, so minijinja's per-render
        // metadata check for edited sources buys nothing and costs syscalls.
        env.set_auto_reload(false);
        env.set_auto_escape_callback(|_| AutoEscape::Html);
        env.add_filter("slugify", |value: String| slugify(&value));
        env.add_filter("starts_with", |value: String, prefix: String| {
            value.starts_with(&prefix)
        });
        env.add_filter("url", |value: String| {
            Value::from_safe_string(escape_url(&value))
        });
        for (name, source) in theme::TEMPLATES {
            if !dir.join(name).is_file() {
                env.add_template_owned(*name, *source)
                    .with_context(|| format!("loading embedded template {name}"))?;
            }
        }
        if dir.is_dir() {
            env.set_loader(path_loader(dir));
        }
        Ok(Self { env })
    }

    pub fn has(&self, name: &str) -> bool {
        self.env.get_template(name).is_ok()
    }

    pub fn first(&self, names: &[&str]) -> Option<String> {
        names
            .iter()
            .find(|name| self.has(name))
            .map(|name| name.to_string())
    }

    /// Resolve a template once so callers can render it many times without
    /// paying for a name lookup (and its lock) on every page.
    pub fn get(&self, name: &str) -> Result<Template<'_, '_>> {
        self.env
            .get_template(name)
            .with_context(|| format!("template {name} not found"))
    }

    pub fn render(&self, name: &str, context: Value) -> Result<String> {
        Ok(self.get(name)?.render(context)?)
    }
}

/// Escape a URL for use in an HTML attribute, leaving `/` alone.
///
/// The `Html` autoescape turns every `/` into `&#x2f;`, which browsers decode
/// but which triples the size of a path like `/blog/hello/`. A URL only needs
/// the characters that could end the attribute or start markup escaped, so the
/// `url` filter does that and marks the result safe.
fn escape_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len() + 8);
    for ch in url.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape_url;

    #[test]
    fn a_path_keeps_its_slashes() {
        assert_eq!(escape_url("/blog/hello-world/"), "/blog/hello-world/");
        assert_eq!(
            escape_url("https://example.com/a/b"),
            "https://example.com/a/b"
        );
    }

    #[test]
    fn query_ampersands_and_quotes_are_still_escaped() {
        assert_eq!(escape_url("/s?q=a&b=1"), "/s?q=a&amp;b=1");
        assert_eq!(escape_url("/a\"><script>"), "/a&quot;&gt;&lt;script&gt;");
        assert_eq!(escape_url("/it's"), "/it&#x27;s");
    }
}
