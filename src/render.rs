use crate::content::slugify;
use crate::theme;
use anyhow::{Context, Result};
use minijinja::{AutoEscape, Environment, Value, path_loader};
use std::path::Path;

pub struct Templates {
    env: Environment<'static>,
}

impl Templates {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|_| AutoEscape::Html);
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

    pub fn render(&self, name: &str, context: Value) -> Result<String> {
        let template = self
            .env
            .get_template(name)
            .with_context(|| format!("template {name} not found"))?;
        Ok(template.render(context)?)
    }
}
