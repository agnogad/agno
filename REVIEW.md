# Rust review: agno (crate root, `src/**`, v0.3.2, edition 2024, MSRV 1.85)

agno is a content-first static site generator: it walks `content/`, renders Markdown with
pulldown-cmark and syntect, renders MiniJinja templates, and writes pages, section and
taxonomy listings, feeds, a sitemap and a search index, with a fingerprint cache so a
rebuild only re-renders what changed. `Cargo.toml` is a published crate with a `lib.rs`,
so I reviewed it as a **library**, which is stricter than the code's own intent: the public
surface looks like a test hook rather than a supported API, and finding 6 argues that
should be made explicit one way or the other. The code is in good shape — there is no
`unsafe`, no hand-written `Ord`/`Hash`, and only two `expect`s outside tests — and the
single most important thing to fix is that the build can silently omit `static/` files
when the filesystem returns an error.

Tools: `cargo fmt --check` clean, `cargo test` 46 unit + 4 fuzz + 79 integration passing,
`cargo clippy --all-targets --all-features -- -W clippy::pedantic` reports 93 warnings
(mostly `#[must_use]` and missing `# Errors`, grouped in finding 12).

## Must fix

### 1. A filesystem error turns into "this file does not exist" (`src/site.rs:1584`, `:1556`, `:1158`)

**Rule:** Errors that lose information (rust-review, Error handling); contrast
`src/content.rs:182`, which propagates the same `WalkDir` error with `entry?`.

**Problem:** `static_files` collects with `filter_map(|entry| entry.ok())`, so an
unreadable directory or a file that disappears mid-walk is dropped from the list. The build
then exits 0 with those assets missing from `dist/` — a silently wrong site rather than a
failed build. The same pattern at `:1158` leaves an unreadable template out of the cache
header (stale pages), and at `:1556` leaves a file behind after `agno clean`.

**Fix:**
```rust
// static_files: the function already returns Result
fn static_files(cfg: &Config) -> Result<Vec<PathBuf>> {
    let src = cfg.static_dir();
    if !src.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(&src) {
        let entry = entry?;                       // was: filter_map(|e| e.ok())
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}
```
For `remove_all`, collect the failures and return the first one after the tree is walked,
so `agno clean` can report "left 2 files in dist/" instead of nothing.

## Should fix

### 2. `chunks(0)` is one refactor away from a panic (`src/site.rs:612`, `:639`)

**Rule:** A panic is a contract, and a contract the callee cannot see is a latent bug
(M-PANIC-ON-BUG, Microsoft Rust Guidelines).

**Problem:** `page_size` is `paginate_by.unwrap_or(0)` and is passed straight to
`all.chunks(page_size)`. It is safe only because `section_page_count` returns 1 for a zero
size and the loop at `:616` then `continue`s. The guard is 25 lines away from the panic, so
anyone touching `paginate_by` (allowing 0 to mean "all on one page", or a frontmatter value
that parses to 0) turns this into `chunks(0)` → `assert!(chunk_size != 0)` → abort.

**Fix:** let one `Option` decide both the page count and the chunk size, so the `continue`
at `:616` and the `chunks` call cannot disagree:
```rust
// Some(size) only when pagination actually splits the section.
let page_size = section.paginate_by.filter(|size| *size > 0 && all.len() > *size);
let Some(page_size) = page_size else {
    // ... the single-listing branch that is at :617 today ...
    continue;
};
let total = all.len().div_ceil(page_size);
for (index, chunk) in all.chunks(page_size).enumerate() {
```

### 3. Line numbers come from pointer arithmetic (`src/content.rs:660`)

**Rule:** Prefer `from`/`into` over `as`, which silently permits lossy conversions
(Effective Rust, Item 5).

**Problem:** `body.as_ptr() as usize - src.as_ptr() as usize` recovers the offset of a
subslice. It is correct today because `split_frontmatter` returns a slice of `src`, but if
that ever changes to an owned `String` the subtraction underflows: a panic in debug, and in
release the `offset > src.len()` guard quietly reports line 1. `check` prints these line
numbers in every "broken link" error.

**Fix:** the body is a *suffix* of the source, which is checkable without pointers:
```rust
fn body_start_line(src: &str, body: &str) -> usize {
    let Some(offset) = src.len().checked_sub(body.len()).filter(|_| src.ends_with(body)) else {
        return 1;
    };
    1 + src[..offset].bytes().filter(|byte| *byte == b'\n').count()
}
```

### 4. Bare `bool` parameters in the public API (`src/check.rs:31`, `src/content.rs:169`, `src/scaffold.rs:61`, `src/serve.rs:14`)

**Rule:** An enum instead of a `bool` parameter, a newtype only when it will always be
boolean (Effective Rust, Item 1); clippy::fn_params_excessive_bools.

**Problem:** `check(&root, false)` and `content::load(&cfg, true)` say nothing at the call
site. Three of these four are the same question — "include drafts?" — asked in three
signatures, so one type removes them all; `create(path, force)` is the fourth.

**Fix:**
```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Drafts {
    #[default]
    Skip,
    Include,
}

pub fn check(root: &Path, drafts: Drafts) -> Result<Report>;
pub fn load(cfg: &Config, drafts: Drafts) -> Result<Content>;
pub fn dev(root: &Path, port: u16, drafts: Drafts) -> Result<()>;
```

### 5. `serve::dev` takes `PathBuf` and immediately borrows it (`src/serve.rs:14`)

**Rule:** C-CALLER-CONTROL (Rust API Guidelines): take by value only when you keep it.

**Problem:** the first thing `dev` does is `root.canonicalize()`, which only needs
`&Path`, so callers must give up ownership of a path they still hold. clippy's
`needless_pass_by_value` flags the same thing.

**Fix:**
```rust
pub fn dev(root: &Path, port: u16, drafts: Drafts) -> Result<()> {
    let root = root.canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
```

### 6. The public API is accidental (`src/lib.rs:1-11`, `src/cache.rs:38`, `src/site.rs:1547`)

**Rule:** Keep `anyhow` out of a library's public API, and make the error type implement
`std::error::Error` (Effective Rust, Item 4; C-GOOD-ERR); public items are documented
(rust-review, Documentation).

**Problem:** every module is `pub mod`, so the surface is 81 items — 69 of them without a
line of rustdoc — and it exposes `anyhow::Error` through `site::build`, `check::check` and
`Config::load`, so a downstream caller cannot distinguish "no config" from "bad config".
Two items are worse than useless: `cache::Entry` is public but no public function returns
it, and `site::remove_all` is an unrestricted recursive delete reachable from any crate
that depends on agno. Meanwhile the only in-repo consumer of this surface is
`tests/site_build.rs`.

**Fix:** pick one. Either make it a real API — `pub(crate)` for everything the tests do not
need, a `thiserror` enum for build failures, `#![warn(missing_docs)]`, and
`remove_all` replaced by a `clean(root) -> Result<PathBuf>` that validates the config
first. Or keep it as a test hook and say so in `lib.rs`, with `#[doc(hidden)]` on the
internals, so nobody builds on it by accident.

### 7. `BuildOptions`, `Report` and `Stats` miss the common traits (`src/site.rs:24`, `:32`, `src/check.rs:18`)

**Rule:** C-COMMON-TRAITS (Rust API Guidelines): eagerly implement `Debug`, `Clone`,
`Default`, `PartialEq` on public types.

**Problem:** `BuildOptions` has no `Default`, so every caller writes all three fields and a
future field breaks them; `Report` has `Default` but no `Debug`, so it cannot be printed in
a failing test; `Stats` stores four `u128` millisecond counts, which is not the type for a
duration and forces callers to format the unit themselves.

**Fix:** (breaking, which is fine before 1.0)
```rust
#[derive(Debug, Clone, Default)]
pub struct BuildOptions { pub drafts: bool, pub clean: bool, pub minify: bool }

#[derive(Debug, Default)]
pub struct Report { /* ... */ }

#[derive(Debug)]
pub struct Stats {
    pub pages: usize,
    pub total: std::time::Duration,
    pub load: std::time::Duration,
    pub pages_time: std::time::Duration,
    pub rest: std::time::Duration,
    pub output: PathBuf,
}
```

## Consider

### 8. The cache writer's comment promises more than `as` delivers (`src/cache.rs:88`, `:95`, `:98`)

**Rule:** Prefer `from`/`into` over `as`, which silently permits lossy conversions
(Effective Rust, Item 5); clippy::cast_possible_truncation.

**Problem:** the comment says lengths are "u32 and never clamped", but `len() as u32`
*wraps* above 4 GiB. A single frontmatter summary that large is absurd, and the failure
mode is benign (the decoder rejects the file and the next build is a full rebuild), but the
comment states an invariant the code does not hold.

**Fix:**
```rust
let summary_len = u32::try_from(summary.len())
    .with_context(|| format!("summary for {url} is too large to cache"))?;
```

### 9. `url.ends_with(".html")` is case-sensitive (`src/content.rs:958`)

**Rule:** Treat file extensions case-insensitively (the crate already does this in
`is_named`, `src/content.rs:647`).

**Problem:** `url = "/print.HTML"` becomes `/print.HTML/` — a directory URL for what the
author meant as a file, so the output path no longer matches the link they wrote.

**Fix:**
```rust
let is_file = url
    .rsplit_once('.')
    .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("html"));
if url.ends_with('/') || is_file {
```

### 10. `agno new --force` overwrites `content/` (`src/scaffold.rs:62-79`)

**Rule:** Respect intent (rust-review, "respect intent" — the flag's name describes
templates, not the author's prose).

**Problem:** `--force` is documented as "overwrite", but it also rewrites
`content/about.md` and `content/blog/hello-world.md`. Re-scaffolding an existing site to
pick up new templates therefore destroys the two pages a user is most likely to have
edited, with no per-file prompt.

**Fix:** limit `force` to the files agno owns (`templates/`, `static/style.css`,
`.gitignore`) and skip any `content/` file that already exists, or refuse and list the
files it would replace.

### 11. `build()` is 183 lines (`src/site.rs:308`)

**Rule:** Keep functions to one job (Google Rust Style Guide, "small functions";
clippy::too_many_lines).

**Problem:** load, header hash, page render, list render, feeds, static copy, manifest and
fingerprint save all live in one body, which is why the fingerprint conditions at `:499`
need a comment to be readable. The phases already have names — they are just inline.

**Fix:** extract `render_pages(...)`, `render_lists(...)` and `write_feeds(...)`; the
`Stats` timings already mark the boundaries.

### 12. 99 pedantic warnings, 49 auto-fixable

**Rule:** clippy::must_use_candidate, clippy::missing_errors_doc, clippy::map_unwrap_or.

**Problem:** 28 `#[must_use]` candidates (`slugify`, `parse_date`, `minify_html`,
`Fingerprints::len`, …), 14 public `Result` functions without a `# Errors` section, 10
`map(..).unwrap_or(..)` that read better as `map_or`/`is_some_and`, 8 redundant closures.
None is a bug; together they are the difference between "clippy clean" and "clippy quiet".

**Fix:** `cargo clippy --fix --all-targets -- -W clippy::pedantic` for the 49 mechanical
ones, then write the `# Errors` sections by hand.

## What's already good

- **No `unsafe` anywhere**, no `transmute`, no hand-written `Ord`/`Hash`/`Eq`, and exactly
  two `expect`s outside tests — both on genuinely local invariants (`minify.rs:103` slices
  at a byte the ASCII scanner just matched; `serve.rs:234` builds a header from constants).
- **Deterministic output by construction.** Every sort carries a total tie-breaker
  (`content.rs:213` ends with `page.url`), terms and sections live in `BTreeMap`/`BTreeSet`,
  and the `HashMap`s in `site.rs` are only ever looked up, never iterated into a template —
  so two machines produce byte-identical sites.
- **Errors carry their context.** `with_context(|| format!("reading {}", path.display()))`
  is the house style, and the frontmatter errors name the file *and* the offending value
  (`invalid date "yesterday" in ./content/x.md`, `duplicate output URL /old/: alias of /b/
  and alias of /a/`).
- **The tests earn their keep.** 123 of them, including a fuzz file for parser panics and
  minifier idempotency, and `check` shares `markdown::options()` and `site::output_rel`
  with the build so the two cannot disagree about what a link resolves to.

---

The BeautifulRust `lints/` config would have caught findings 4, 6, 7 and 8 before they
landed: paste its `[lints.rust]` / `[lints.clippy]` tables into `Cargo.toml` and copy
`lints/clippy.toml` next to it. `missing_debug_implementations` and `missing_docs` cover 7
and half of 6, `unreachable_pub` covers the rest of 6, `max-fn-params-bools = 0` covers 4,
and `cast_possible_truncation` covers 8.

---

## Resolution

Applied on `fix/review-findings` (not merged):

| # | Finding | Status | Test |
|---|---|---|---|
| 1 | `filter_map(..ok())` drops filesystem errors (`static_files`, `templates_fingerprint`, `remove_all`) | **fixed** — all three propagate; `clean` reports what it could not delete | `an_unreadable_static_directory_fails_the_build`, `clean_reports_a_directory_it_cannot_empty` |
| 2 | `chunks(page_size)` panics when `paginate_by = 0` | **fixed** — one `split_size` helper owns the invariant, so no call site can see `0` | `paginate_by_zero_is_treated_as_no_pagination`, `paginate_by_equal_to_the_page_count_does_not_split` |
| 3 | `body_start_line` derives a line number from pointer subtraction | **fixed** — the loader records the line while splitting | `check_reports_source_lines_and_reference_links` |
| 4 | `bool` parameters (`drafts`, `force`) | **fixed** — `content::Drafts` and `scaffold::Overwrite` | call sites updated |
| 5 | `dev` takes `PathBuf` | **fixed** — `&Path` | dev smoke test |
| 6 | Accidental public API | **fixed** — `lib.rs` documents that the modules are test seams, not a stable API; `cache::Entry` made private | — |
| 7 | Missing `Default`/`Debug`; `Stats` stores `u128` milliseconds | **fixed** — derives added, `Stats` holds `Duration` | `--stats` smoke |
| 8 | `as u32` in the cache wraps | **fixed** — `field_len` refuses a length over 4 GiB instead of truncating | unit tests in `cache.rs` |
| 9 | `.html` check is case-sensitive | **fixed** — `eq_ignore_ascii_case` on the tail | `a_url_ending_in_an_html_extension_keeps_its_shape` |
| 10 | `new --force` overwrites `content/` | **deferred** — `--force` means "overwrite", and narrowing it silently would break the flag's only use | — |
| 11 | `build()` is 183 lines | **deferred** — the phases are already named blocks; splitting it now would churn the diff for a review-only gain | — |
| 12 | 93 pedantic warnings | **deferred** — all `must_use`/`# Errors` style, no defects; a mechanical `clippy --fix` pass is a separate commit | — |

The default `cargo clippy --all-targets --all-features -- -D warnings` gate is clean, and
`cargo fmt --check` is clean.
