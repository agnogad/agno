---
title: About
summary: About the bongo static site generator: who it is for, its design goals, and what it leaves out.
---

# About bongo

bongo is a static site generator written in TypeScript. It runs in Bun, Deno, and Node, and it is made for sites that need no server.

## Who it's for

bongo is a good fit for a blog, a docs site, a marketing page, or a portfolio. If you need sign-in, content for each user, or live data, you probably want something else, or a static site that calls an API.

## Design goals

- **Fast by default**: Rolldown and LightningCSS bundle in Rust, and a rebuild renders only the pages that a change reaches.
- **Small surface**: few conventions to learn, and no JavaScript in a page unless you add it.
- **TypeScript first**: the config is a `.ts` file, and the config, plugins, and templates are typed.
- **Plugins are config**: a loader turns any file extension into HTML. The Markdown plugin is only a loader.

## Non-goals

bongo leaves out:

- a component model or a UI framework,
- server-side rendering and edge functions,
- a built-in CMS or content API,
- hot module replacement: dev reloads the page after each rebuild.

## Architecture

The build runs in two walks. Walk 1 reads each content file, runs its loaders, and records the page. Then bongo makes the registries: `$pages`, `$section`, and `$tags`. Walk 2 renders `.hbs` content and layouts, which can all read those registries. Last, it bundles the scripts and stylesheets that the pages link, and puts their file names in the pages.

## Source

The source is on [GitHub](https://github.com/lukeed/bongo). Contributions are welcome.
