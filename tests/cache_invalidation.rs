//! Incremental-cache behaviour that a no-op or body-edit benchmark cannot see:
//! adding content, changing volatile `site.*` stats, and a corrupt manifest.

use agno::site::{BuildOptions, build};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("agno-cache-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("content/blog")).unwrap();
    fs::write(
        root.join("agno.toml"),
        "title = \"Test Site\"\nbase_url = \"https://example.com\"\nauthor = \"Tester\"\n",
    )
    .unwrap();
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\n+++\n\nWelcome home.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/about.md"),
        "+++\ntitle = \"About\"\n+++\n\nAbout body.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"Rust\"]\n+++\n\nFirst post.\n",
    )
    .unwrap();
    root
}

fn build_with(root: &Path, clean: bool) {
    build(
        root,
        &BuildOptions {
            drafts: false,
            clean,
            minify: false,
        },
    )
    .unwrap();
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn add_post(root: &Path) {
    fs::write(
        root.join("content/blog/added.md"),
        "+++\ntitle = \"Added\"\ndate = 2023-01-01\ntags = [\"Rust\"]\n+++\n\nNew post.\n",
    )
    .unwrap();
}

fn manifest_of(root: &Path) -> PathBuf {
    let cache = root.join(".agno-cache");
    let mut found: Vec<PathBuf> = fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("outputs-") && name.ends_with(".json"))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one build manifest");
    found.remove(0)
}

/// The core regression: `post_count` and the tag counts change, but the stock
/// theme never reads them, so an unrelated page must keep its cached HTML.
#[test]
fn adding_a_post_does_not_invalidate_unrelated_page_fingerprints() {
    let root = fixture("add-post");
    build_with(&root, true);

    let about = root.join("dist/about/index.html");
    let before = modified(&about);
    assert!(fs::read_to_string(&about).unwrap().contains("About body"));

    build_with(&root, false);
    assert_eq!(before, modified(&about), "a no-op rebuild rewrote a page");

    add_post(&root);
    build_with(&root, false);

    assert_eq!(
        before,
        modified(&about),
        "an unrelated page was re-rendered when a post was added"
    );
    assert!(root.join("dist/blog/added/index.html").is_file());
    // Lists are derived from the pages and must still pick the post up.
    assert!(
        fs::read_to_string(root.join("dist/index.html"))
            .unwrap()
            .contains("Added")
    );
}

/// When the resolved template following `extends` reaches a volatile field,
/// invalidation is restored — the optimisation must not hide stale output.
#[test]
fn a_template_reading_volatile_site_fields_is_revalidated() {
    let root = fixture("volatile-template");
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/base.html"),
        "<!doctype html><html><head><title>{% block title %}{{ page.title }}{% endblock %}</title></head>\
         <body><span class=\"count\">{{ site.post_count }}</span>{% block content %}{% endblock %}</body></html>",
    )
    .unwrap();
    build_with(&root, true);
    build_with(&root, false); // establish the cache with the new template

    let about = root.join("dist/about/index.html");
    let before = modified(&about);
    assert!(
        fs::read_to_string(&about)
            .unwrap()
            .contains("class=\"count\"")
    );

    add_post(&root);
    build_with(&root, false);

    assert_ne!(
        before,
        modified(&about),
        "a template reading site.post_count kept a stale cached page"
    );
}

/// A corrupt manifest must never silently skip the stale sweep.
#[test]
fn a_corrupt_manifest_does_not_leave_stale_files() {
    let root = fixture("corrupt-manifest");
    build_with(&root, true);
    assert!(root.join("dist/about/index.html").is_file());

    // A page disappears from the source, then the manifest that records it is
    // damaged before the next build.
    fs::remove_file(root.join("content/about.md")).unwrap();
    fs::write(manifest_of(&root), "not json").unwrap();

    build_with(&root, false);

    assert!(
        !root.join("dist/about/index.html").exists(),
        "a stale file survived a build with a corrupt manifest"
    );
    assert!(root.join("dist/blog/hello/index.html").is_file());
}

/// Unicode-aware folding is what a case-insensitive static host does.
#[test]
fn unicode_case_variants_are_rejected_as_duplicate_outputs() {
    let root = fixture("unicode-case");
    fs::write(
        root.join("content/blog/upper.md"),
        "+++\ntitle = \"Upper\"\nurl = \"/Ä/\"\n+++\n\nUpper.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/lower.md"),
        "+++\ntitle = \"Lower\"\nurl = \"/ä/\"\n+++\n\nLower.\n",
    )
    .unwrap();

    let err = build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("duplicate output URL"), "{err}");
}
