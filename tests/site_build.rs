use agno::site::{BuildOptions, build};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("agno-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("content/blog")).unwrap();
    fs::write(
        root.join("agno.toml"),
        "title = \"Test Site\"\nbase_url = \"https://example.com\"\nauthor = \"Tester\"\nsearch_index = true\n",
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
    assert!(!rss.contains("About body"));

    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert!(sitemap.contains("https://example.com/about/"));

    let index = text(&root.join("dist/search-index.json"));
    assert!(index.contains("\"title\":\"Hello\""));
    assert!(!index.contains("\"title\":\"About\""));

    assert!(root.join("dist/style.css").exists());
    assert!(!root.join("dist/blog/draft/index.html").exists());
}

#[test]
fn user_templates_override_embedded_defaults() {
    let root = fixture("override");
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/index.html"),
        "custom {{ page.title }} {{ pages|length }}",
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

    let home = text(&root.join("dist/index.html"));
    assert!(home.starts_with("custom Home 2"));
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

fn clean_build(root: &Path) -> agno::site::Stats {
    build(
        root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap()
}

#[test]
fn a_page_bundle_does_not_create_its_own_section() {
    let root = fixture("bundle-no-self-section");
    fs::remove_file(root.join("content/about.md")).unwrap();
    fs::create_dir_all(root.join("content/about")).unwrap();
    fs::write(
        root.join("content/about/index.md"),
        "+++\ntitle = \"About\"\n+++\n\nAbout bundle body.\n",
    )
    .unwrap();

    clean_build(&root);

    let page = text(&root.join("dist/about/index.html"));
    assert!(page.contains("About bundle body."));
}

#[test]
fn duplicate_output_urls_are_rejected() {
    let root = fixture("duplicate-url");
    fs::write(
        root.join("content/blog/one.md"),
        "+++\ntitle = \"One\"\nurl = \"/same/\"\n+++\n\nOne.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/two.md"),
        "+++\ntitle = \"Two\"\nurl = \"/same/\"\n+++\n\nTwo.\n",
    )
    .unwrap();

    let err = build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("duplicate output URL /same/"), "{err}");
}

#[test]
fn file_and_directory_output_conflicts_are_rejected() {
    let root = fixture("output-conflict");
    fs::write(
        root.join("content/blog/one.md"),
        "+++\ntitle = \"One\"\nurl = \"/blog/thing.html\"\n+++\n\nOne.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/two.md"),
        "+++\ntitle = \"Two\"\nurl = \"/blog/thing.html/\"\n+++\n\nTwo.\n",
    )
    .unwrap();

    let err = build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("output path conflict"), "{err}");
}

#[test]
fn tag_slugs_are_merged() {
    let root = fixture("tag-slugs");
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"Rust\", \"rust\"]\n+++\n\nPost.\n",
    )
    .unwrap();

    clean_build(&root);

    assert!(root.join("dist/tags/rust/index.html").exists());
    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert_eq!(sitemap.matches("/tags/rust/").count(), 1);
}

#[test]
fn slugs_and_dotted_custom_urls_get_their_own_directories() {
    let root = fixture("url-model");
    fs::create_dir_all(root.join("content/docs")).unwrap();
    fs::write(
        root.join("content/blog/2024-05-01-Hello World.md"),
        "+++\ntitle = \"Spaced\"\n+++\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/docs/release.md"),
        "+++\ntitle = \"Release\"\nurl = \"/docs/release.notes/\"\n+++\n\nBody.\n",
    )
    .unwrap();

    clean_build(&root);

    assert!(root.join("dist/blog/hello-world/index.html").exists());
    assert!(root.join("dist/docs/release.notes/index.html").exists());
}

#[test]
fn section_urls_are_slugified_and_sections_can_be_empty() {
    let root = fixture("section-slug");
    fs::create_dir_all(root.join("content/My Notes")).unwrap();
    fs::write(
        root.join("content/My Notes/_index.md"),
        "+++\ntitle = \"My Notes\"\n+++\n\nNotes intro.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/My Notes/entry.md"),
        "+++\ntitle = \"Entry\"\n+++\n\nEntry body.\n",
    )
    .unwrap();

    clean_build(&root);

    assert!(root.join("dist/my-notes/index.html").exists());
    assert!(root.join("dist/my-notes/entry/index.html").exists());
    assert!(text(&root.join("dist/my-notes/index.html")).contains("Entry"));
}

#[test]
fn crlf_frontmatter_is_parsed() {
    let root = fixture("crlf");
    fs::write(
        root.join("content/blog/crlf.md"),
        "+++\r\ntitle = \"CRLF\"\r\ndate = 2024-07-01\r\n+++\r\n\r\nCRLF body.\r\n",
    )
    .unwrap();

    clean_build(&root);

    let page = text(&root.join("dist/blog/crlf/index.html"));
    assert!(page.contains("CRLF"));
    assert!(page.contains("CRLF body."));
}

#[test]
fn incremental_build_removes_stale_generated_and_static_files() {
    let root = fixture("stale");
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/extra.txt"), "extra").unwrap();

    clean_build(&root);
    assert!(root.join("dist/blog/hello/index.html").exists());
    assert!(root.join("dist/extra.txt").exists());

    fs::remove_file(root.join("content/blog/hello.md")).unwrap();
    fs::remove_file(root.join("static/extra.txt")).unwrap();
    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: false,
        },
    )
    .unwrap();

    assert!(!root.join("dist/blog/hello/index.html").exists());
    assert!(!root.join("dist/extra.txt").exists());
    assert!(root.join("dist/about/index.html").exists());
}

#[test]
fn removing_a_static_stylesheet_restores_the_default() {
    let root = fixture("stale-style");
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/style.css"), "custom-style").unwrap();
    clean_build(&root);
    assert!(text(&root.join("dist/style.css")).contains("custom-style"));

    fs::remove_file(root.join("static/style.css")).unwrap();
    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: false,
        },
    )
    .unwrap();

    assert!(!text(&root.join("dist/style.css")).contains("custom-style"));
}

#[test]
fn incremental_build_handles_a_file_to_directory_url_change() {
    let root = fixture("stale-shape");
    let page = root.join("content/blog/thing.md");
    fs::write(
        &page,
        "+++\ntitle = \"Thing\"\nurl = \"/blog/thing.html\"\n+++\n\nBody.\n",
    )
    .unwrap();
    clean_build(&root);
    assert!(root.join("dist/blog/thing.html").is_file());

    fs::write(
        &page,
        "+++\ntitle = \"Thing\"\nurl = \"/blog/thing.html/\"\n+++\n\nBody.\n",
    )
    .unwrap();
    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: false,
        },
    )
    .unwrap();

    assert!(root.join("dist/blog/thing.html/index.html").is_file());
}

#[test]
fn output_dir_must_not_escape_or_overlap_inputs() {
    for value in [".", "content", "../outside"] {
        let root = fixture(&format!("unsafe-{}", value.replace(['.', '/'], "_")));
        fs::write(
            root.join("agno.toml"),
            format!(
                "title = \"Test Site\"\nbase_url = \"https://example.com\"\noutput_dir = \"{value}\"\n"
            ),
        )
        .unwrap();

        let err = build(
            &root,
            &BuildOptions {
                drafts: false,
                clean: true,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("output_dir"), "{value}: {err}");
        assert!(
            root.join("content/about.md").exists(),
            "{value} deleted content"
        );
    }
}

#[test]
fn a_missing_page_template_is_an_error() {
    let root = fixture("missing-template");
    fs::write(
        root.join("content/blog/bad.md"),
        "+++\ntitle = \"Bad\"\ntemplate = \"nope.html\"\n+++\n\nBody.\n",
    )
    .unwrap();

    let err = build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("template nope.html not found"), "{err}");
}
