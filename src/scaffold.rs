use crate::theme;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use time::OffsetDateTime;
use time::macros::format_description;

const CONFIG: &str = r#"title = "My Agno Site"
description = "A site built with agno"
base_url = "http://localhost:1111"
language = "en"

[[menu]]
title = "Blog"
url = "/blog/"

[[menu]]
title = "About"
url = "/about/"
"#;

const HOME: &str = r#"+++
title = "My Agno Site"
+++

Welcome. This site is built with **agno** — a small, fast static site generator.
"#;

const ABOUT: &str = r#"+++
title = "About"
description = "About this site"
+++

This is the about page. Edit `content/about.md` to change it.
"#;

const POST: &str = r#"+++
title = "Hello, agno"
date = __DATE__
description = "The first post on a site built with agno."
tags = ["hello", "agno"]
+++

This post lives in `content/blog/hello-world.md`. Edit it, save, and run `agno dev` to see the change.

## Markdown works

- Tables, footnotes and task lists
- Syntax highlighting
- Frontmatter in TOML or YAML

```rust
fn main() {
    println!("fast");
}
```

That is it. No Node, no `npm install`, no config required.
"#;

pub fn create(path: &Path, force: bool) -> Result<()> {
    if path.exists() {
        let is_empty = fs::read_dir(path)?.next().is_none();
        if !is_empty && !force {
            bail!(
                "{} already exists and is not empty (use --force to overwrite)",
                path.display()
            );
        }
    }

    for (name, body) in theme::TEMPLATES {
        write(path.join("templates").join(name), body)?;
    }
    write(path.join("static/style.css"), theme::STYLE)?;
    write(path.join("agno.toml"), CONFIG)?;
    write(path.join(".gitignore"), "dist/\n.agno-cache/\n")?;
    write(path.join("content/_index.md"), HOME)?;
    write(path.join("content/about.md"), ABOUT)?;

    let today = OffsetDateTime::now_utc()
        .date()
        .format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_else(|_| "2026-01-01".to_string());
    write(
        path.join("content/blog/hello-world.md"),
        &POST.replace("__DATE__", &today),
    )?;

    Ok(())
}

fn write(path: impl AsRef<Path>, contents: &str) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
