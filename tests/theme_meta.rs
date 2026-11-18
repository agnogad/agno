use agno::site::{BuildOptions, build};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("agno-theme-meta-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("content/blog")).unwrap();
    fs::write(
        root.join("agno.toml"),
        "title = \"Test Site\"\nbase_url = \"https://example.com\"\nauthor = \"Tester\"\n",
    )
    .unwrap();
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\n+++\n\nHome.\n",
    )
    .unwrap();
    root
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn clean_build(root: &Path) {
    build(
        root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: false,
        },
    )
    .unwrap();
}

#[test]
fn default_theme_emits_social_meta_with_escaped_values() {
    let root = fixture("escaped");
    // TOML literal strings keep the quotes and markup intact so the template's
    // autoescaping is what we actually exercise here.
    fs::write(
        root.join("content/blog/hello.md"),
        r#"+++
title = 'Hello "World"'
date = 2024-05-01
description = 'A "quoted" & <special> desc'
image = 'https://example.com/img/a&b.png'
+++

Body.
"#,
    )
    .unwrap();

    clean_build(&root);

    let html = text(&root.join("dist/blog/hello/index.html"));
    assert!(
        html.contains(r#"<meta property="og:title" content="Hello &quot;World&quot;">"#),
        "og:title: {html}"
    );
    assert!(
        html.contains(r#"<meta property="og:description" content="A &quot;quoted&quot; &amp; &lt;special&gt; desc">"#),
        "og:description: {html}"
    );
    assert!(
        html.contains(r#"<meta property="og:url" content="https://example.com/blog/hello/">"#),
        "og:url: {html}"
    );
    assert!(!html.contains("&#x2f;"), "slashes were escaped: {html}");
    assert!(
        html.contains(r#"<meta property="og:type" content="article">"#),
        "og:type: {html}"
    );
    assert!(
        html.contains(r#"<meta property="og:site_name" content="Test Site">"#),
        "og:site_name: {html}"
    );
    assert!(
        html.contains(
            r#"<meta property="og:image" content="https://example.com/img/a&amp;b.png">"#
        ),
        "og:image: {html}"
    );
    assert!(
        html.contains(r#"<meta name="twitter:card" content="summary_large_image">"#),
        "twitter:card: {html}"
    );
}

#[test]
fn social_meta_falls_back_without_a_date_or_image() {
    let root = fixture("no-image");
    fs::write(
        root.join("content/blog/plain.md"),
        "+++\ntitle = \"Plain\"\n+++\n\nBody.\n",
    )
    .unwrap();

    clean_build(&root);

    let html = text(&root.join("dist/blog/plain/index.html"));
    assert!(
        html.contains(r#"<meta property="og:type" content="website">"#),
        "og:type: {html}"
    );
    assert!(!html.contains("og:image"), "unexpected og:image: {html}");
    assert!(
        html.contains(r#"<meta name="twitter:card" content="summary">"#),
        "twitter:card: {html}"
    );
}

#[test]
fn a_relative_og_image_is_resolved_against_the_base_url() {
    let root = fixture("relative-image");
    fs::write(
        root.join("content/blog/cover.md"),
        "+++\ntitle = \"Cover\"\nimage = '/img/cover.png'\n+++\n\nBody.\n",
    )
    .unwrap();

    clean_build(&root);

    let html = text(&root.join("dist/blog/cover/index.html"));
    assert!(
        html.contains(r#"<meta property="og:image" content="https://example.com/img/cover.png">"#),
        "og:image must be absolute for social crawlers: {html}"
    );
}
