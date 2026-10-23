use agno::site::{BuildOptions, build};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("agno-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("content/blog")).unwrap();
    fs::write(
        root.join("agno.toml"),
        "title = \"Test Site\"\nbase_url = \"https://example.com\"\n",
    )
    .unwrap();
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\n+++\n\nWelcome **home**.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/about.md"),
        "+++\ntitle = \"About\"\ndescription = \"About us\"\n+++\n\nAbout body.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"Rust\", \"Çığır\"]\n+++\n\nFirst **post**.\n\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/draft.md"),
        "+++\ntitle = \"Draft\"\ndraft = true\n+++\n\nHidden.\n",
    )
    .unwrap();
    root
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[test]
fn builds_pages_lists_feeds_and_taxonomies() {
    let root = fixture("full");
    let stats = build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap();
    assert_eq!(stats.pages, 2);

    let post = text(&root.join("dist/blog/hello/index.html"));
    assert!(post.contains("<title>Hello · Test Site</title>"));
    assert!(post.contains("<strong>post</strong>"));
    assert!(post.contains("2024-05-01"));
    assert!(post.contains("/tags/rust/"));
    assert!(post.contains("/tags/cigir/"));

    assert!(text(&root.join("dist/index.html")).contains("Hello"));
    assert!(text(&root.join("dist/blog/index.html")).contains("Hello"));
    assert!(text(&root.join("dist/tags/rust/index.html")).contains("Hello"));
    assert!(text(&root.join("dist/404.html")).contains("404"));

    let rss = text(&root.join("dist/rss.xml"));
    assert!(rss.contains("<link>https://example.com/blog/hello/</link>"));
    assert!(!rss.contains("Draft"));

    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert!(sitemap.contains("https://example.com/about/"));

    assert!(root.join("dist/style.css").exists());
    assert!(!root.join("dist/blog/draft/index.html").exists());
}

#[test]
fn drafts_flag_includes_drafts() {
    let root = fixture("drafts");
    let stats = build(
        &root,
        &BuildOptions {
            drafts: true,
            clean: true,
        },
    )
    .unwrap();
    assert_eq!(stats.pages, 3);
    assert!(root.join("dist/blog/draft/index.html").exists());
}

#[test]
fn yaml_frontmatter_and_page_bundles_work() {
    let root = fixture("yaml");
    fs::create_dir_all(root.join("content/notes/hello")).unwrap();
    fs::write(
        root.join("content/notes/hello/index.md"),
        "---\ntitle: Bundle\ndate: 2024-06-02T10:30:00Z\n---\n\nBundle body.\n",
    )
    .unwrap();

    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap();

    let page = text(&root.join("dist/notes/hello/index.html"));
    assert!(page.contains("Bundle"));
    assert!(page.contains("2024-06-02"));
}
