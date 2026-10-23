# Benchmark

Compares agno, Hugo and Astro on the same generated Markdown corpus.

```sh
cargo build --release
HUGO_BIN=/path/to/hugo python3 bench/suite.py
```

`suite.py` runs the full matrix and writes `results.json` plus `REPORT.md`.
For a quick single scenario:

```sh
python3 bench/run.py --count 1000 --runs 5 --profile code
```

## Corpus

`gen.py` writes N posts (YAML frontmatter, tags, deterministic prose) into each
project's content directory:

| Profile | Content |
| --- | --- |
| `plain` | three paragraphs, no code |
| `code` | three paragraphs + one Rust code block |
| `heavy` | six paragraphs + three code blocks (Rust/JS/CSS), a table, a footnote |

## Fixtures

- `hugo/` and `astro/` are minimal projects with layouts equivalent to the
  embedded agno theme: single page, list, home, 404, stylesheet.
- `run.py` prepares all three sites under `work/` (gitignored). Astro
  dependencies are installed once with `npm install`.
- agno highlights code with syntect/Oniguruma; Hugo uses Chroma. Both do real
  syntax highlighting, so their timings include it.
- Astro's fixture renders Markdown pages only: no syntax highlighting,
  taxonomies, RSS or sitemap. Hugo and agno do all of those here, so Astro's
  timings are optimistic.

## Scenarios

- **Cold build:** output directory deleted before each timed run; a warmup run
  precedes the samples. Command lines are exactly what a user runs.
- **Single-edit rebuild:** one post gets an extra paragraph, then the normal
  build command runs with the previous output present.
- **Tiny site:** two posts, to show fixed startup cost.
- **Scaling:** 10,000 posts for agno and Hugo (Astro omitted for runtime).
- **Startup:** `agno --version`, `hugo version`, `astro --version`.
- **RSS / footprint:** peak RSS via `wait4`, binary and `node_modules` size,
  generated output size.

Runs: 7 (100 posts), 5 (1,000), 3 (3,000), 2 (10,000), 10 (tiny/startup).
Medians are reported; raw samples are in `results.json`.

Set `SKIP_ASTRO=1` to skip Astro, or `AGNO_BIN` / `HUGO_BIN` to point at other
binaries.
