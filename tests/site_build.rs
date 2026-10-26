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
            minify: false,
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
            minify: false,
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
            minify: false,
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
            minify: false,
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
            minify: false,
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
            minify: false,
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
            minify: false,
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
fn categories_get_term_pages_and_an_overview() {
    let root = fixture("categories");
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"Rust\"]\ncategories = [\"Engineering\", \"Notes\"]\n+++\n\nPost.\n",
    )
    .unwrap();

    clean_build(&root);

    let term = text(&root.join("dist/categories/engineering/index.html"));
    assert!(term.contains("Hello"), "{term}");

    let overview = text(&root.join("dist/categories/index.html"));
    assert!(overview.contains("/categories/engineering/"), "{overview}");
    assert!(overview.contains("/categories/notes/"), "{overview}");

    let page = text(&root.join("dist/blog/hello/index.html"));
    assert!(page.contains("/categories/engineering/"), "{page}");

    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert_eq!(sitemap.matches("/categories/engineering/").count(), 1);
    assert!(sitemap.contains("<loc>https://example.com/categories/</loc>"));
}

#[test]
fn the_tags_index_lists_every_term_with_its_count() {
    let root = fixture("tags-index");

    clean_build(&root);

    let index = text(&root.join("dist/tags/index.html"));
    assert!(index.contains("/tags/rust/"), "{index}");
    assert!(index.contains("/tags/cigir/"), "{index}");
    assert!(index.contains("<span class=\"count\">1</span>"), "{index}");
}

#[test]
fn a_site_without_terms_gets_no_taxonomy_index() {
    let root = fixture("no-taxonomies");
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\n+++\n\nPost.\n",
    )
    .unwrap();

    clean_build(&root);

    assert!(!root.join("dist/tags/index.html").exists());
    assert!(!root.join("dist/categories/index.html").exists());
}

#[test]
fn custom_taxonomy_template_overrides_the_default() {
    let root = fixture("taxonomy-template");
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/taxonomy.html"),
        "terms:{% for term in page.terms %}{{ term.name }}={{ term.count }};{% endfor %}",
    )
    .unwrap();

    clean_build(&root);

    assert_eq!(
        text(&root.join("dist/tags/index.html")),
        "terms:Çığır=1;Rust=1;"
    );
}

#[test]
fn a_page_cannot_take_a_taxonomy_url() {
    let root = fixture("taxonomy-collision");
    fs::write(
        root.join("content/tags.md"),
        "+++\ntitle = \"Tags\"\nurl = \"/tags/\"\n+++\n\nBody.\n",
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

#[test]
fn editing_a_section_index_rerenders_its_listing() {
    let root = fixture("section-body");
    build_incremental(&root);

    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\n+++\n\nBrand new intro.\n",
    )
    .unwrap();
    build_incremental(&root);

    let listing = text(&root.join("dist/blog/index.html"));
    assert!(listing.contains("Brand new intro."), "{listing}");
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
            minify: false,
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
            minify: false,
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
            minify: false,
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
                minify: false,
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
            minify: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("template nope.html not found"), "{err}");
}

fn build_incremental(root: &Path) -> agno::site::Stats {
    build(
        root,
        &BuildOptions {
            drafts: false,
            clean: false,
            minify: false,
        },
    )
    .unwrap()
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn incremental_build_leaves_unchanged_pages_alone() {
    let root = fixture("incremental-skip");
    build_incremental(&root);

    let page = root.join("dist/blog/hello/index.html");
    let before = modified(&page);

    build_incremental(&root);
    assert_eq!(before, modified(&page), "unchanged page was rewritten");

    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\ntags = [\"Rust\", \"Çığır\"]\n+++\n\nSecond **version**.\n",
    )
    .unwrap();
    build_incremental(&root);

    let after = text(&page);
    assert!(after.contains("<strong>version</strong>"), "{after}");
    assert!(!after.contains("<strong>post</strong>"));
    assert_ne!(before, modified(&page), "changed page was not rewritten");
}

#[test]
fn incremental_build_rerenders_when_the_site_context_changes() {
    let root = fixture("incremental-site");
    build_incremental(&root);

    let page = root.join("dist/blog/hello/index.html");
    assert!(text(&page).contains("<title>Hello · Test Site</title>"));

    fs::write(
        root.join("agno.toml"),
        "title = \"Renamed Site\"\nbase_url = \"https://example.com\"\n",
    )
    .unwrap();
    build_incremental(&root);
    assert!(text(&page).contains("<title>Hello · Renamed Site</title>"));
}

#[test]
fn incremental_build_restores_a_deleted_output_file() {
    let root = fixture("incremental-deleted");
    build_incremental(&root);

    let page = root.join("dist/blog/hello/index.html");
    fs::remove_file(&page).unwrap();
    build_incremental(&root);
    assert!(page.is_file(), "deleted output was not restored");
    assert!(text(&page).contains("<strong>post</strong>"));
}

#[test]
fn incremental_build_picks_up_template_edits() {
    let root = fixture("incremental-template");
    build_incremental(&root);

    let page = root.join("dist/blog/hello/index.html");
    assert!(!text(&page).contains("data-revision"));

    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/page.html"),
        "{% extends \"base.html\" %}{% block content %}<div data-revision=\"2\">{{ page.content|safe }}</div>{% endblock %}",
    )
    .unwrap();
    build_incremental(&root);
    assert!(text(&page).contains("data-revision=\"2\""));
}

#[test]
fn headings_get_anchors_and_a_table_of_contents() {
    let root = fixture("toc");
    fs::write(
        root.join("content/blog/toc.md"),
        "+++\ntitle = \"Toc\"\ndate = 2024-01-01\n+++\n\nIntro.\n\n## One\n\nA.\n\n## Two\n\nB.\n\n### Deep\n\nC.\n",
    )
    .unwrap();
    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: false,
        },
    )
    .unwrap();

    let page = text(&root.join("dist/blog/toc/index.html"));
    assert!(page.contains("<h2 id=\"one\">"), "{page}");
    assert!(page.contains("<h3 id=\"deep\">"), "{page}");
    assert!(page.contains("<a href=\"#two\">Two</a>"), "{page}");
    assert!(page.contains("min read"), "{page}");
}

#[test]
fn sections_can_paginate_their_pages() {
    let root = fixture("pagination");
    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\npaginate_by = 2\n+++\n",
    )
    .unwrap();
    for i in 0..5 {
        fs::write(
            root.join(format!("content/blog/p{i}.md")),
            format!(
                "+++\ntitle = \"Post {i}\"\ndate = 2024-01-0{} \n+++\n\nBody {i}.\n",
                i + 1
            ),
        )
        .unwrap();
    }

    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: false,
        },
    )
    .unwrap();

    let first = text(&root.join("dist/blog/index.html"));
    assert_eq!(first.matches("class=\"post-list\"").count(), 1);
    assert!(first.contains("Post 4"), "newest post first: {first}");
    assert!(first.contains("1 / 3"), "{first}");
    assert!(first.contains("rel=\"next\""), "{first}");
    assert!(!first.contains("rel=\"prev\""), "{first}");

    let second = text(&root.join("dist/blog/page/2/index.html"));
    assert!(second.contains("2 / 3"));
    assert!(second.contains("rel=\"prev\""));
    assert!(second.contains("rel=\"next\""));

    let third = text(&root.join("dist/blog/page/3/index.html"));
    assert!(third.contains("3 / 3"));
    assert!(!third.contains("Older →"), "last page has no next link");
    assert!(!root.join("dist/blog/page/4").exists());

    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert!(sitemap.contains("/blog/page/2/"), "{sitemap}");
    assert!(sitemap.contains("/blog/page/3/"), "{sitemap}");
}

#[test]
fn templates_can_read_the_updated_date_and_sections() {
    let root = fixture("updated");
    fs::write(
        root.join("content/blog/hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2024-05-01\nupdated = 2024-06-02\n+++\n\nBody.\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/page.html"),
        "updated={{ page.updated }}|display={{ page.updated_display }}|\
         sections={% for s in site.sections %}{{ s.title }}:{{ s.count }} {% endfor %}",
    )
    .unwrap();

    clean_build(&root);

    let page = text(&root.join("dist/blog/hello/index.html"));
    assert!(page.contains("updated=2024-06-02"), "{page}");
    assert!(page.contains("display=June 2, 2024"), "{page}");
    assert!(page.contains("sections=Blog:1"), "{page}");
}

#[test]
fn list_pages_expose_frontmatter_extra() {
    let root = fixture("list-extra");
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\nhero = \"home.png\"\n+++\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\nhero = \"blog.png\"\n+++\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/index.html"),
        "hero={{ page.extra.hero }}",
    )
    .unwrap();
    fs::write(
        root.join("templates/section.html"),
        "hero={{ page.extra.hero }}",
    )
    .unwrap();

    clean_build(&root);
    assert_eq!(text(&root.join("dist/index.html")), "hero=home.png");
    let section = root.join("dist/blog/index.html");
    assert_eq!(text(&section), "hero=blog.png");
    // Tag pages also use section.html; their extra object is simply empty.
    assert_eq!(text(&root.join("dist/tags/rust/index.html")), "hero=");

    let before = modified(&section);
    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\nhero = \"blog-2.png\"\n+++\n",
    )
    .unwrap();
    build_incremental(&root);
    assert_eq!(text(&section), "hero=blog-2.png");
    assert_ne!(before, modified(&section), "extra change was not rendered");
}

#[test]
fn aliases_emit_redirect_pages() {
    let root = fixture("aliases");
    fs::write(
        root.join("content/blog/moved.md"),
        "+++\ntitle = \"Moved\"\ndate = 2024-02-02\naliases = [\"/old-post/\", \"legacy/moved\"]\n+++\n\nBody.\n",
    )
    .unwrap();

    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: false,
        },
    )
    .unwrap();

    let alias = text(&root.join("dist/old-post/index.html"));
    assert!(alias.contains("http-equiv=\"refresh\""), "{alias}");
    assert!(alias.contains("https://example.com/blog/moved/"), "{alias}");
    assert!(alias.contains("rel=\"canonical\""), "{alias}");
    assert!(root.join("dist/legacy/moved/index.html").is_file());

    let sitemap = text(&root.join("dist/sitemap.xml"));
    assert!(
        !sitemap.contains("old-post"),
        "aliases stay out of the sitemap"
    );
}

#[test]
fn self_aliases_and_repeated_aliases_are_dropped() {
    let root = fixture("alias-self");
    fs::write(
        root.join("content/blog/self.md"),
        "+++\ntitle = \"Self\"\ndate = 2024-01-01\naliases = [\"/blog/self/\", \"/old/\", \"/old/\"]\n+++\n\nBody.\n",
    )
    .unwrap();

    clean_build(&root);

    let page = text(&root.join("dist/blog/self/index.html"));
    assert!(
        !page.contains("http-equiv=\"refresh\""),
        "the page was replaced by a redirect to itself: {page}"
    );
    assert!(root.join("dist/old/index.html").is_file());
}

#[test]
fn an_alias_that_collides_with_a_page_is_rejected() {
    let root = fixture("alias-conflict");
    fs::write(
        root.join("content/blog/moved.md"),
        "+++\ntitle = \"Moved\"\naliases = [\"/blog/hello/\"]\n+++\n\nBody.\n",
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

#[test]
fn check_scans_home_and_section_bodies() {
    let root = fixture("check-index");
    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\n+++\n\nSee [the archive](/archive/).\n",
    )
    .unwrap();
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\n+++\n\nWelcome **home**. Read [about](/about/).\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    assert!(!report.ok(), "a broken _index.md link must fail the check");
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("archive") && error.contains("_index.md")),
        "{:?}",
        report.errors
    );
}

#[test]
fn check_validates_anchor_targets() {
    let root = fixture("check-anchor");
    fs::write(
        root.join("content/blog/target.md"),
        "+++\ntitle = \"Target\"\ndate = 2024-01-02\n+++\n\n## Deep Section\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/anchors.md"),
        "+++\ntitle = \"Anchors\"\ndate = 2024-01-01\n+++\n\n## Real Heading\n\n\
         <span id=\"raw-target\"></span>\n\n\
         [ok](#real-heading)\n\n[raw](#raw-target)\n\n[bad](#nope)\n\n\
         [code](#fake-id)\n\n`<div id=\"fake-id\">`\n\n\
         [fenced](#fenced-id)\n\n```html\n<span id=\"fenced-id\"></span>\n```\n\n\
         [good](/blog/target/#deep-section)\n\n[cross](/blog/target/#missing)\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    for anchor in ["#nope", "#missing", "#fake-id", "#fenced-id"] {
        assert!(
            report.errors.iter().any(|error| error.contains(anchor)),
            "{anchor} not reported: {:?}",
            report.errors
        );
    }
    assert!(
        !report.errors.iter().any(|error| {
            error.contains("#real-heading")
                || error.contains("#raw-target")
                || error.contains("#deep-section")
        }),
        "valid anchors were reported: {:?}",
        report.errors
    );
}

#[test]
fn check_accepts_a_clean_site() {
    let root = fixture("check-clean");
    let report = agno::check::check(&root, false).unwrap();
    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(report.pages, 2);
}

#[test]
fn check_reports_broken_internal_links() {
    let root = fixture("check-broken");
    fs::write(
        root.join("content/blog/broken.md"),
        "+++\ntitle = \"Broken\"\ndate = 2024-01-01\n+++\n\n[ok](/blog/hello/)\n\n[bad](/nope/)\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    assert!(!report.ok());
    assert!(
        report.errors.iter().any(|error| error.contains("/nope/")),
        "{:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("broken.md")),
        "{:?}",
        report.errors
    );
}

#[test]
fn check_flags_a_missing_page_template() {
    let root = fixture("check-template");
    fs::write(
        root.join("content/blog/odd.md"),
        "+++\ntitle = \"Odd\"\ntemplate = \"nope.html\"\n+++\n\nBody.\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("template nope.html does not exist")),
        "{:?}",
        report.errors
    );
}

#[test]
fn minify_collapses_template_whitespace_but_not_code() {
    let root = fixture("minify");
    build(
        &root,
        &BuildOptions {
            drafts: false,
            clean: true,
            minify: true,
        },
    )
    .unwrap();

    let post = text(&root.join("dist/blog/hello/index.html"));
    assert!(post.starts_with("<!doctype html><html"), "{post}");
    assert!(!post.contains("    <meta"), "indentation survived");
    assert!(post.contains("<pre class=\"code\">"), "code block missing");
    assert!(post.contains("main</span>"), "code block lost its content");
}

#[test]
fn rebuild_recreates_an_output_directory_that_was_removed() {
    let root = fixture("dir-reuse");
    fs::create_dir_all(root.join("content/blog/2024")).unwrap();
    let source = root.join("content/blog/2024/first.md");
    let body = "+++\ntitle = \"First\"\ndate = 2024-01-01\n+++\n\nBody.\n";
    fs::write(&source, body).unwrap();
    build_incremental(&root);
    assert!(root.join("dist/blog/2024/first/index.html").is_file());

    // Dropping the only page under blog/2024 lets the stale cleanup delete that
    // directory, which the per-thread mkdir cache has already recorded.
    fs::remove_file(&source).unwrap();
    build_incremental(&root);
    assert!(!root.join("dist/blog/2024").exists(), "stale dir survived");

    // Restoring the page has to recreate the directory the cache still claims.
    fs::write(&source, body).unwrap();
    build_incremental(&root);
    assert!(root.join("dist/blog/2024/first/index.html").is_file());
}

#[test]
fn a_no_op_rebuild_leaves_generated_files_untouched() {
    let root = fixture("noop-mtimes");
    build_incremental(&root);
    let rss = root.join("dist/rss.xml");
    let sitemap = root.join("dist/sitemap.xml");
    let manifest = root.join(".agno-cache/outputs.json");
    let before = (modified(&rss), modified(&sitemap), modified(&manifest));

    build_incremental(&root);

    assert_eq!(
        before,
        (modified(&rss), modified(&sitemap), modified(&manifest)),
        "a rebuild rewrote a feed or the manifest even though nothing changed"
    );
}

#[test]
fn a_card_neutral_edit_leaves_the_list_pages_alone() {
    let root = fixture("card-key");
    build_incremental(&root);
    let home = root.join("dist/index.html");
    let before = modified(&home);

    let post = root.join("content/blog/hello.md");
    let original = fs::read_to_string(&post).unwrap();
    fs::write(&post, format!("{original}\n\nA trailing paragraph.\n")).unwrap();
    build_incremental(&root);

    assert_eq!(
        before,
        modified(&home),
        "the home list was rewritten although no card changed"
    );
    assert!(
        text(&root.join("dist/blog/hello/index.html")).contains("trailing paragraph"),
        "the edited page itself was not re-rendered"
    );
}

#[test]
fn cards_use_the_rendered_summary_when_no_description_is_set() {
    let root = fixture("summary-cards");
    fs::write(
        root.join("content/blog/nodesc.md"),
        "+++\ntitle = \"No Description\"\ndate = 2024-03-03\n+++\n\nAuto summary sentence here.\n\nBody follows.\n",
    )
    .unwrap();

    build_incremental(&root);
    let home = text(&root.join("dist/index.html"));
    assert!(home.contains("Auto summary sentence here."), "{home}");

    let feed = text(&root.join("dist/rss.xml"));
    assert!(feed.contains("Auto summary sentence here."), "{feed}");

    // The second build reuses the cached page; the summary must survive.
    build_incremental(&root);
    let home = text(&root.join("dist/index.html"));
    assert!(home.contains("Auto summary sentence here."), "{home}");
}

#[test]
fn nested_sections_render_a_listing_for_their_own_directory() {
    let root = fixture("nested-sections");
    fs::create_dir_all(root.join("content/docs/guide")).unwrap();
    fs::write(
        root.join("content/docs/guide/_index.md"),
        "+++\ntitle = \"Guide\"\n+++\n\nGuide landing.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/docs/guide/one.md"),
        "+++\ntitle = \"One\"\ndate = 2024-01-01\n+++\n\nFirst guide page.\n",
    )
    .unwrap();

    clean_build(&root);

    let guide = text(&root.join("dist/docs/guide/index.html"));
    assert!(guide.contains("Guide landing."), "{guide}");
    assert!(guide.contains("One"), "{guide}");
    assert!(
        !root.join("dist/docs/index.html").exists(),
        "a directory without _index.md must not gain a listing"
    );
}

#[test]
fn page_bundle_assets_are_copied_and_checked() {
    let root = fixture("bundle-assets");
    fs::create_dir_all(root.join("content/blog/photo")).unwrap();
    fs::write(
        root.join("content/blog/photo/index.md"),
        "+++\ntitle = \"Photo\"\ndate = 2024-01-01\n+++\n\n![shot](photo.jpg)\n",
    )
    .unwrap();
    fs::write(root.join("content/blog/photo/photo.jpg"), b"jpeg-bytes").unwrap();

    clean_build(&root);
    assert_eq!(
        fs::read(root.join("dist/blog/photo/photo.jpg")).unwrap(),
        b"jpeg-bytes"
    );
    let report = agno::check::check(&root, false).unwrap();
    assert!(report.ok(), "{:?}", report.errors);

    fs::remove_file(root.join("content/blog/photo/photo.jpg")).unwrap();
    build_incremental(&root);
    assert!(
        !root.join("dist/blog/photo/photo.jpg").exists(),
        "a removed bundle asset must be swept from the output"
    );
}

#[test]
fn check_decodes_percent_escaped_links_and_skips_other_schemes() {
    let root = fixture("check-encoded");
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/my file.pdf"), "pdf").unwrap();
    fs::write(
        root.join("content/blog/encoded.md"),
        "+++\ntitle = \"Encoded\"\ndate = 2024-01-01\n+++\n\n\
         [pdf](/my%20file.pdf)\n\n[sms](sms:+15551234)\n\n[mail](mailto:a@example.com)\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    assert!(report.ok(), "{:?}", report.errors);
}

#[test]
fn a_static_file_cannot_silently_replace_a_page() {
    let root = fixture("static-page-collision");
    fs::create_dir_all(root.join("static/about")).unwrap();
    fs::write(root.join("static/about/index.html"), "static wins").unwrap();

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

    let report = agno::check::check(&root, false).unwrap();
    assert!(!report.ok());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("duplicate output URL")),
        "{:?}",
        report.errors
    );
}

#[test]
fn a_static_file_cannot_replace_a_paginated_page() {
    let root = fixture("static-pagination-collision");
    fs::write(
        root.join("content/blog/_index.md"),
        "+++\ntitle = \"Blog\"\npaginate_by = 1\n+++\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/second.md"),
        "+++\ntitle = \"Second\"\ndate = 2024-05-02\n+++\n\nSecond post.\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("static/blog/page/2")).unwrap();
    fs::write(root.join("static/blog/page/2/index.html"), "static wins").unwrap();

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

#[test]
fn check_reports_source_lines_and_reference_links() {
    let root = fixture("check-lines");
    fs::write(
        root.join("content/blog/lines.md"),
        "+++\ntitle = \"Lines\"\ndate = 2024-01-01\n+++\n\n![shot](/nope.png)\n\n\
         [ref][missing]\n\n[missing]: /also-nope/\n",
    )
    .unwrap();

    let report = agno::check::check(&root, false).unwrap();
    let errors = report.errors.join("\n");
    assert!(errors.contains("lines.md:6"), "{errors}");
    assert!(errors.contains("lines.md:8"), "{errors}");
}

/// A docs-shaped section: three pages, no dates, explicit weights.
fn docs_fixture(name: &str, sort_by: &str) -> PathBuf {
    let root = fixture(name);
    fs::create_dir_all(root.join("content/docs")).unwrap();
    fs::write(
        root.join("content/docs/_index.md"),
        format!("+++\ntitle = \"Docs\"\nsort_by = \"{sort_by}\"\n+++\n"),
    )
    .unwrap();
    for (slug, title, weight) in [
        ("usage", "Usage", 30),
        ("intro", "Intro", 10),
        ("install", "Install", 20),
    ] {
        fs::write(
            root.join(format!("content/docs/{slug}.md")),
            format!("+++\ntitle = \"{title}\"\nweight = {weight}\n+++\n\nBody of {title}.\n"),
        )
        .unwrap();
    }
    root
}

fn build_once(root: &Path, clean: bool) {
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

#[test]
fn weight_orders_a_section_and_links_neighbours() {
    let root = docs_fixture("docs-weight", "weight");
    build_once(&root, true);

    let listing = text(&root.join("dist/docs/index.html"));
    let intro = listing.find(">Intro<").expect("intro card");
    let install = listing.find(">Install<").expect("install card");
    let usage = listing.find(">Usage<").expect("usage card");
    assert!(intro < install && install < usage, "{listing}");

    let intro_page = text(&root.join("dist/docs/intro/index.html"));
    assert!(!intro_page.contains("rel=\"prev\""), "{intro_page}");
    assert!(intro_page.contains("rel=\"next\" href=\"/docs/install/\""));

    let install_page = text(&root.join("dist/docs/install/index.html"));
    assert!(install_page.contains("rel=\"prev\" href=\"/docs/intro/\""));
    assert!(install_page.contains("rel=\"next\" href=\"/docs/usage/\""));

    let usage_page = text(&root.join("dist/docs/usage/index.html"));
    assert!(usage_page.contains("rel=\"prev\" href=\"/docs/install/\""));
    assert!(!usage_page.contains("rel=\"next\""), "{usage_page}");
}

#[test]
fn sort_by_title_orders_a_section_alphabetically() {
    let root = docs_fixture("docs-title", "title");
    build_once(&root, true);

    let listing = text(&root.join("dist/docs/index.html"));
    let install = listing.find(">Install<").expect("install card");
    let intro = listing.find(">Intro<").expect("intro card");
    let usage = listing.find(">Usage<").expect("usage card");
    assert!(install < intro && intro < usage, "{listing}");

    let intro_page = text(&root.join("dist/docs/intro/index.html"));
    assert!(intro_page.contains("rel=\"prev\" href=\"/docs/install/\""));
    assert!(intro_page.contains("rel=\"next\" href=\"/docs/usage/\""));
}

#[test]
fn pagination_follows_the_section_order() {
    let root = docs_fixture("docs-paginated-weight", "weight");
    fs::write(
        root.join("content/docs/_index.md"),
        "+++\ntitle = \"Docs\"\nsort_by = \"weight\"\npaginate_by = 1\n+++\n",
    )
    .unwrap();
    build_once(&root, true);

    let first = text(&root.join("dist/docs/index.html"));
    assert!(first.contains(">Intro<"), "{first}");
    let second = text(&root.join("dist/docs/page/2/index.html"));
    assert!(second.contains(">Install<"), "{second}");
    let third = text(&root.join("dist/docs/page/3/index.html"));
    assert!(third.contains(">Usage<"), "{third}");
}

#[test]
fn a_section_sorts_by_date_unless_told_otherwise() {
    let root = fixture("docs-default-order");
    fs::write(
        root.join("content/blog/second.md"),
        "+++\ntitle = \"Second\"\ndate = 2024-06-02\n+++\n\nLater.\n",
    )
    .unwrap();
    build_once(&root, true);

    let listing = text(&root.join("dist/blog/index.html"));
    let hello = listing.find(">Hello<").expect("hello card");
    let second = listing.find(">Second<").expect("second card");
    assert!(second < hello, "{listing}");
    assert!(
        text(&root.join("dist/blog/hello/index.html"))
            .contains("rel=\"prev\" href=\"/blog/second/\""),
        "the older post points back at the newer one"
    );
}

#[test]
fn the_home_page_honours_sort_by() {
    let root = fixture("home-sort");
    fs::write(
        root.join("content/_index.md"),
        "+++\ntitle = \"Home\"\nsort_by = \"weight\"\n+++\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/heavy.md"),
        "+++\ntitle = \"Heavy\"\nweight = 2\ndate = 2024-07-01\n+++\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/light.md"),
        "+++\ntitle = \"Light\"\nweight = 1\ndate = 2024-01-01\n+++\n\nBody.\n",
    )
    .unwrap();
    build_once(&root, true);

    // Date order would put Heavy first; the home page asked for weight.
    let home = text(&root.join("dist/index.html"));
    let light = home.find(">Light<").expect("light card");
    let heavy = home.find(">Heavy<").expect("heavy card");
    assert!(light < heavy, "{home}");
}

#[test]
fn an_unknown_sort_by_is_rejected() {
    let root = docs_fixture("docs-bad-sort", "popular");
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
    assert!(err.contains("invalid sort_by \"popular\""), "{err}");
    assert!(err.contains("date"), "{err}");
}

#[test]
fn changing_a_neighbours_date_rerenders_the_page() {
    let root = fixture("neighbour-date");
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/page.html"),
        "{{ page.title }}|prev={{ page.prev.date_display }}",
    )
    .unwrap();
    fs::write(
        root.join("content/blog/second.md"),
        "+++\ntitle = \"Second\"\ndate = 2024-06-02\n+++\n\nLater.\n",
    )
    .unwrap();
    build_once(&root, true);
    let before = text(&root.join("dist/blog/hello/index.html"));
    assert!(before.contains("June 2, 2024"), "{before}");

    // Only the neighbour changes; hello itself is untouched, but what it
    // renders about its neighbour is not.
    fs::write(
        root.join("content/blog/second.md"),
        "+++\ntitle = \"Second\"\ndate = 2024-06-03\n+++\n\nLater.\n",
    )
    .unwrap();
    build_once(&root, false);

    let after = text(&root.join("dist/blog/hello/index.html"));
    assert!(after.contains("June 3, 2024"), "{after}");
}

#[test]
fn inserting_a_page_rerenders_its_neighbours() {
    let root = fixture("docs-insert");
    fs::create_dir_all(root.join("content/docs")).unwrap();
    fs::write(
        root.join("content/docs/_index.md"),
        "+++\ntitle = \"Docs\"\nsort_by = \"weight\"\n+++\n",
    )
    .unwrap();
    for (slug, title, weight) in [("a", "Alpha", 10), ("c", "Gamma", 30)] {
        fs::write(
            root.join(format!("content/docs/{slug}.md")),
            format!("+++\ntitle = \"{title}\"\nweight = {weight}\n+++\n\nBody.\n"),
        )
        .unwrap();
    }
    build_once(&root, true);
    let last = text(&root.join("dist/docs/c/index.html"));
    assert!(last.contains("rel=\"prev\" href=\"/docs/a/\""));

    // A new page in the middle changes what both existing pages link to, even
    // though neither of them was edited.
    fs::write(
        root.join("content/docs/b.md"),
        "+++\ntitle = \"Beta\"\nweight = 20\n+++\n\nBody.\n",
    )
    .unwrap();
    build_once(&root, false);

    let first = text(&root.join("dist/docs/a/index.html"));
    assert!(first.contains("rel=\"next\" href=\"/docs/b/\""), "{first}");
    let middle = text(&root.join("dist/docs/b/index.html"));
    assert!(middle.contains("rel=\"prev\" href=\"/docs/a/\""));
    assert!(middle.contains("rel=\"next\" href=\"/docs/c/\""));
    let last = text(&root.join("dist/docs/c/index.html"));
    assert!(last.contains("rel=\"prev\" href=\"/docs/b/\""), "{last}");
}
