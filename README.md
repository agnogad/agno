# agno

[![CI](https://github.com/agnogad/agno/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/agnogad/agno/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

**A fast, content-first static site generator written in Rust.**

agno turns Markdown and Jinja-style templates into a static site from a single
binary — no Node.js, no bundler, and no plugin runtime. It is built for sites
where the content is the product: blogs, documentation, notes, and small
marketing pages.

## Why agno?

- **Fast by default.** Cold builds are parallel, from Markdown parsing and
  syntax highlighting to template rendering and output writing.
- **One static binary.** The release build is about 4 MB and has no runtime
  dependencies. No `node_modules`, no language runtime, no service to install.
- **Small, understandable surface.** A focused feature set instead of a plugin
  ecosystem, a bundler, and a JavaScript toolchain.
- **Batteries included.** Sections, tags, page bundles, drafts, RSS, sitemap,
  search index, `404.html`, syntax highlighting, and a live-reloading dev server.
- **Comfortable authoring.** TOML or YAML frontmatter, optional templates, and
  a working site from `agno new` in seconds.

Benchmarked against Hugo and Astro on the same corpus, agno builds **3.9–12.8×
faster than Hugo** and **16.7–185× faster than Astro** on cold builds. See
[Benchmarks](#benchmarks) for numbers, methodology, and caveats.

## Installation

### Requirements

- Rust **1.85+** (edition 2024)
- A C toolchain at build time (`cc`, MSVC, or Xcode command-line tools). The
  syntax highlighter bundles Oniguruma; the resulting binary is still fully
  static at runtime.

### Install from GitHub

```sh
cargo install --git https://github.com/agnogad/agno
```

### Build from source

```sh
git clone https://github.com/agnogad/agno.git
cd agno
cargo install --path .
```

Prebuilt binaries for Linux, macOS, and Windows are planned for releases.
agno is not yet published on crates.io.

Verify the install:

```sh
agno --version
```

## Quick start

```sh
agno new blog
cd blog
agno dev
```

Open <http://localhost:1111>. Edit anything under `content/`, `templates/`, or
`static/` and the browser reloads after the rebuild.

Build for production:

```sh
agno build
# Built 2 pages in 9 ms → ./dist
```

The generated site is in `dist/` and can be deployed to any static host.

## Commands

| Command | Description |
| --- | --- |
| `agno new <dir>` | Scaffold a new site with the default theme, content, and config |
| `agno build` | Build the site into `dist/` |
| `agno build --drafts` | Include pages marked `draft = true` |
| `agno build --stats` | Print content, page, and feed timings |
| `agno dev` | Build, serve on port 1111, and rebuild on file changes |
| `agno dev --port 3000` | Use a different port |
| `agno clean` | Remove `dist/` and the local cache |

`build`, `dev`, and `clean` accept `--root <dir>` to operate on another site
directory. `new` accepts `--force` to write into a non-empty directory.

## Benchmarks

Cold-build medians on an Intel i5-13420H (12 threads), CachyOS, Linux 7.2.
The output directory is removed before every timed run and process startup is
included. `code` adds one syntax-highlighted code block per post; `heavy` adds
three code blocks, a table, and a footnote.

| Scenario | agno | Hugo | Astro |
| --- | ---: | ---: | ---: |
| 100 posts, plain | **12.9 ms** | 97.8 ms | 2,385 ms |
| 1,000 posts, plain | **65.3 ms** | 320.9 ms | 3,596 ms |
| 1,000 posts, code | **92.1 ms** | 396.3 ms | 3,530 ms |
| 1,000 posts, heavy | **181.4 ms** | 2,071.3 ms | 5,715 ms |
| 3,000 posts, heavy | **554.7 ms** | 7,120.9 ms | 9,273 ms |
| 10,000 posts, code | **1,004 ms** | 4,067 ms | — |

| Other situations | agno | Hugo | Astro |
| --- | ---: | ---: | ---: |
| Single-edit rebuild, 1,000 posts | **180 ms** | 545 ms | 3,724 ms |
| 2-page site, cold build | **17 ms** | 95 ms | 2,825 ms |
| CLI startup | **1.8 ms** | 58.1 ms | 133.2 ms |
| Peak RSS, 1,000 posts/code | **21 MB** | 146 MB | 624 MB |
| Peak RSS, 3,000 posts/heavy | **49 MB** | 540 MB | 701 MB |
| Install footprint | **4.0 MB binary** | 62.6 MB binary | 121 MB `node_modules` |

At 10,000 posts agno sustains about **9,955 pages/s**, compared with 2,459
pages/s for Hugo. The full matrix, raw samples, and reproduction steps are in
[`bench/REPORT.md`](bench/REPORT.md) and [`bench/README.md`](bench/README.md).

> **Fairness note:** the Astro fixture renders Markdown pages only and does not
> include syntax highlighting, taxonomies, RSS, or a sitemap. Its timings are
> optimistic. agno and Hugo both perform real syntax highlighting (syntect /
> Oniguruma and Chroma, respectively). Numbers vary with hardware, filesystem,
> and corpus; run the suite on your own machine before making decisions.

## Project layout

```text
agno.toml          optional config
content/           Markdown pages and sections
  _index.md        homepage metadata and content
  about.md         /about/
  blog/
    _index.md      optional section metadata
    hello.md       /blog/hello/
templates/         optional MiniJinja templates
static/            copied verbatim to dist/
dist/              build output
```

A directory under `content/` becomes a section. A file named `index.md` turns
its directory into a page bundle:

```text
content/blog/hello/index.md  →  /blog/hello/
```

Filenames can start with a date. `2024-05-01-hello.md` sets the date and derives
the slug automatically.

## Content

Frontmatter can be TOML (`+++`) or YAML (`---`):

```markdown
+++
title = "Hello, agno"
date = 2024-05-01
updated = 2024-05-02
description = "A short summary used in lists and meta tags."
tags = ["rust", "static-sites"]
categories = ["engineering"]
draft = false
template = "post.html"   # optional override
slug = "hello-agno"      # optional
url = "/hello/"          # optional
+++

Body in **Markdown**.
```

| Field | Purpose |
| --- | --- |
| `title` | Page title; defaults to a humanized filename |
| `date`, `updated` | Publication and modification dates; used for sorting, feeds, and sitemap |
| `description` | Meta description and list summary |
| `summary` | Explicit summary, overriding the first paragraph |
| `draft` | Excluded from builds unless `--drafts` is passed |
| `tags`, `categories` | Taxonomy values; tags get their own pages |
| `template` | Template name for this page |
| `slug` | Override the generated slug |
| `url` | Override the generated URL |

Any other frontmatter field is available to templates as
`page.extra.<name>`.

## Templates

Templates use [MiniJinja](https://docs.rs/minijinja) and live in `templates/`.
Files in `templates/` override the embedded defaults; missing templates fall
back to them, so a site still builds without a `templates/` directory.

| Variable | Contents |
| --- | --- |
| `site.title`, `site.description`, `site.base_url`, `site.language`, `site.author`, `site.menu`, `site.year`, `site.post_count` | Site metadata |
| `site.tags[].name`, `.url`, `.count`, `.latest_title`, `.latest_url` | Tags with post counts and the newest tagged post |
| `page.title`, `page.url`, `page.date`, `page.date_display`, `page.datetime`, `page.reading_minutes`, `page.description`, `page.content`, `page.tags`, `page.categories`, `page.slug`, `page.section`, `page.extra` | Current page or list page |
| `pages[].title`, `.url`, `.date`, `.date_display`, `.datetime`, `.reading_minutes`, `.summary`, `.tags`, `.section` | Page cards in list templates |

The default templates are `base.html`, `page.html`, `section.html`,
`index.html`, and `404.html`. Add `tag.html` to use a dedicated tag layout.
Filters include `slugify`, `starts_with`, `safe`, and the MiniJinja built-ins.

```html
{% extends "base.html" %}
{% block content %}
  <h1>{{ page.title }}</h1>
  <div class="content">{{ page.content|safe }}</div>
{% endblock %}
```

## Configuration

`agno.toml` is optional; every key has a default.

```toml
title = "My Site"
description = ""
base_url = "https://example.com"
language = "en"
author = "Your Name"

content_dir = "content"
templates_dir = "templates"
static_dir = "static"
output_dir = "dist"

highlight = true
rss_items = 20
search_index = false

[[menu]]
title = "Blog"
url = "/blog/"

[[menu]]
title = "About"
url = "/about/"
```

Set `search_index = true` to emit `search-index.json` for client-side search.
`page.date_display` follows `language` for Turkish and English month names.

## Architecture

agno keeps the pipeline intentionally small:

- `src/content.rs` discovers Markdown files, parses frontmatter, and derives
  URLs, dates, and sections.
- `src/markdown.rs` renders CommonMark/GFM with `pulldown-cmark` and highlights
  fenced code with `syntect`.
- `src/render.rs` renders MiniJinja templates with the embedded theme as a
  fallback.
- `src/site.rs` runs the parallel build, writes pages and feeds, and cleans the
  output directory.
- `src/serve.rs` serves `dist/` with `tiny_http` and rebuilds on filesystem
  changes.
- `src/scaffold.rs` creates a working starter site.

Builds use Rayon for parallel parsing, rendering, output writing, and cleanup.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The benchmark matrix compares agno with Hugo and Astro:

```sh
python3 bench/suite.py
```

See [`bench/README.md`](bench/README.md) for prerequisites, fixtures, scenarios,
and environment variables.

## Design scope

agno is deliberately a **content-first** static site generator. It does not aim
to replace application frameworks:

- no JavaScript islands or component framework
- no bundler or asset pipeline
- no plugin system
- no image optimization pipeline
- no i18n framework or theme marketplace

Static `<script>` tags, plain CSS, and the `static/` directory are the extension
points. Keeping that surface small is what keeps the binary portable and the
build fast.

## Contributing

Issues and pull requests are welcome. Please keep changes focused, run the
checks above, and add a test when behavior changes.

## License

[MIT](LICENSE)
