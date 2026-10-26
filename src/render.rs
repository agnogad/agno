use crate::content::slugify;
use crate::markdown;
use crate::theme;
use anyhow::{Context, Result};
use minijinja::{AutoEscape, Environment, Error, ErrorKind, Output, Template, Value, path_loader};
use std::fmt::Write as _;
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
        // MiniJinja's HTML escaper also turns every `/` into `&#x2f;`. Browsers
        // decode it, so links still work, but it triples the size of every path
        // a template prints and mangles JSON-LD or a canonical URL. Slashes are
        // not special in text or in a quoted attribute, so only the characters
        // that are get escaped.
        env.set_formatter(|out, state, value| {
            if *state.auto_escape() == AutoEscape::Html && !value.is_safe() {
                write_html_escaped(out, value)
            } else {
                minijinja::escape_formatter(out, state, value)
            }
        });
        env.add_filter("slugify", |value: String| slugify(&value));
        env.add_filter("starts_with", |value: String, prefix: String| {
            value.starts_with(&prefix)
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

/// Escape a value for HTML text or a quoted attribute, leaving `/` alone.
fn write_html_escaped(out: &mut Output, value: &Value) -> Result<(), Error> {
    let write_error = |_| Error::new(ErrorKind::WriteFailure, "failed to write output");
    // A string can be escaped straight into the output; anything else is
    // formatted first, which costs an allocation but is rare.
    let mut owned;
    let text = match value.as_str() {
        Some(text) => text,
        None => {
            owned = String::new();
            write!(&mut owned, "{value}").map_err(write_error)?;
            &owned
        }
    };
    if !text
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'&' | b'<' | b'>' | b'"' | b'\''))
    {
        return out.write_str(text).map_err(write_error);
    }
    let mut escaped = String::with_capacity(text.len() + 16);
    markdown::push_escaped(&mut escaped, text);
    out.write_str(&escaped).map_err(write_error)
}

#[cfg(test)]
mod tests {
    use super::Templates;
    use minijinja::context;
    use std::fs;

    fn render(template: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "agno-render-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("t.html"), template).unwrap();
        let templates = Templates::load(&dir).unwrap();
        let rendered = templates
            .render(
                "t.html",
                context! { url => "/blog/hello-world/", title => "a & b <c>", body => "<p>raw</p>" },
            )
            .unwrap();
        let _ = fs::remove_dir_all(&dir);
        rendered
    }

    #[test]
    fn a_path_keeps_its_slashes() {
        assert_eq!(render("{{ url }}"), "/blog/hello-world/");
    }

    #[test]
    fn query_ampersands_and_quotes_are_still_escaped() {
        assert_eq!(render("{{ title }}"), "a &amp; b &lt;c&gt;");
    }

    #[test]
    fn safe_values_are_not_escaped() {
        assert_eq!(render("{{ body|safe }}"), "<p>raw</p>");
    }

    #[test]
    fn a_missing_value_still_renders_empty() {
        assert_eq!(render("[{{ nope }}]"), "[]");
    }
}
