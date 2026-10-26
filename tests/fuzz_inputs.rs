//! Randomised input tests: the parsers and the minifier must never panic.
//!
//! The generators are seeded, so a failure is reproducible, and the alphabet
//! is deliberately nasty: frontmatter delimiters, quotes, angle brackets,
//! multi-byte characters and lone continuation bytes.

use agno::content::{humanize, parse_date, slugify, split_frontmatter};
use agno::markdown;
use agno::minify::minify_html;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, limit: usize) -> usize {
        // `limit` is a small constant at every call site, so both conversions
        // are exact.
        usize::try_from(self.next() % u64::try_from(limit).unwrap()).unwrap()
    }
}

const ALPHABET: &[&str] = &[
    "+",
    "-",
    "\n",
    "\r\n",
    " ",
    "\t",
    "---",
    "+++",
    "title",
    "=",
    "\"",
    "'",
    "<",
    ">",
    "&",
    "/",
    "\\",
    "%",
    "{",
    "}",
    "`",
    "#",
    ":",
    "a",
    "Z",
    "0",
    "ş",
    "ğ",
    "ı",
    "🚀",
    "\u{feff}",
    "\u{0}",
    "<!--",
    "-->",
    "<pre>",
    "</pre>",
    "<script>",
    "</script>",
    "id=\"x\"",
    "href=",
    "&amp;",
];

fn random_text(rng: &mut Rng, max_parts: usize) -> String {
    let parts = rng.below(max_parts) + 1;
    let mut out = String::new();
    for _ in 0..parts {
        out.push_str(ALPHABET[rng.below(ALPHABET.len())]);
    }
    out
}

#[test]
fn frontmatter_parser_never_panics() {
    let mut rng = Rng(0x5eed_0001);
    for _ in 0..2_000 {
        let input = random_text(&mut rng, 40);
        // Errors are fine; a panic or a hang is not.
        let _ = split_frontmatter(&input);
    }
}

#[test]
fn minifier_never_panics_and_is_stable() {
    let mut rng = Rng(0x5eed_0002);
    for _ in 0..2_000 {
        let input = random_text(&mut rng, 60);
        let once = minify_html(&input);
        assert_eq!(minify_html(&once), once, "not idempotent for {input:?}");
    }
}

#[test]
fn markdown_renderer_never_panics() {
    let mut rng = Rng(0x5eed_0003);
    for _ in 0..500 {
        let input = random_text(&mut rng, 40);
        let rendered = markdown::render(&input, true);
        assert!(!rendered.toc_html.is_empty() || rendered.toc.is_empty());
    }
}

#[test]
fn slug_and_date_helpers_never_panic() {
    let mut rng = Rng(0x5eed_0004);
    for _ in 0..2_000 {
        let input = random_text(&mut rng, 20);
        let slug = slugify(&input);
        assert!(!slug.starts_with('-') && !slug.ends_with('-'), "{slug:?}");
        let _ = humanize(&input);
        let _ = parse_date(&input);
    }
}
