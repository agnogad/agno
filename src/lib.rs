//! A fast, content-first static site generator.
//!
//! The crate is split into the pipeline the binary drives: [`config`] reads
//! `agno.toml`, [`content`] loads pages and sections, [`markdown`] renders
//! bodies, [`render`] loads templates, [`site`] writes the output, and
//! [`check`] validates links before a deploy.
//!
//! # API stability
//!
//! This is the library behind the `agno` binary, and the modules are public so
//! that the integration tests in `tests/` can drive the real pipeline instead
//! of a copy of it. They are **not** a stable API yet: items are added, renamed
//! and removed in minor releases, most of them have no rustdoc, and errors are
//! `anyhow::Error`, so a caller cannot match on a failure kind. If you need to
//! build on agno, pin the exact version and expect churn until 1.0.
//!
//! The supported interface is the command line: `agno new`, `build`, `dev`,
//! `check` and `clean`.

pub mod cache;
pub mod check;
pub mod config;
pub mod content;
pub mod markdown;
pub mod minify;
pub mod render;
pub mod scaffold;
pub mod serve;
pub mod site;
pub mod theme;
