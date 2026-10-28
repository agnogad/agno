---
title: Home
summary: bongo is a fast, minimal static site generator built for the Bun era.
---

# bongo

**bongo** is a small, opinionated static site generator. It uses [tempura](https://github.com/lukeed/tempura) templates, which look like Handlebars, a two-pass render pipeline, and [Rolldown](https://rolldown.rs) and [LightningCSS](https://lightningcss.dev) to bundle assets.

The config is a plain `.ts` file. The folders are few and plain, and there is no framework lock-in.

## What it does

- Compiles `.hbs`, `.md`, and `.html` content into pages
- Bundles TypeScript and CSS with Rolldown and LightningCSS, with content-hashed file names
- Renders each page through the layouts of its folder, then the root HTML layout
- Gives `$site`, `$page`, `$pages`, `$section`, and `$tags` to layouts, partials, and `.hbs` content, to build indexes and archives

## Getting started

```bash
bun create bongo my-site
cd my-site
bun run build
```

## Explore

- [Blog](/blog/) — dev log posts on building bongo
- [About](/about/) — design goals and architecture overview
- [Contact](/contact/) — how to reach the project
