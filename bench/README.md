# Benchmark

Compares agno, Hugo, Astro and Zola on the same generated Markdown corpus.

```sh
cargo build --release
HUGO_BIN=/path/to/hugo ZOLA_BIN=/path/to/zola python3 bench/suite.py
```

`suite.py` runs the full matrix and writes `results.json` plus `REPORT.md`.
For a quick single scenario:

```sh
python3 bench/run.py --count 1000 --runs 5 --profile code
```

## Comparing two builds

`suite.py` measures one generator at a time, so a machine that drifts during
the run favours whoever was measured in the quiet window. `ab.py` is the tool
for a claim like "0.3.7 is faster than 0.3.6": it interleaves the two binaries
on one site, round-robin, and adds a third series that runs the first binary
again as a control.

```sh
python3 bench/ab.py --a old/agno --b new/agno --count 1000 --profile code
python3 bench/ab.py --a target/release/agno --b target/release/agno   # calibrate
```

The control is the point. Two series of the *same* binary differ by whatever
the machine was doing, so their ratio is the noise floor, measured rather than
assumed. A result only counts if it beats that floor on both the median and the
fastest run — cold builds are right-skewed, since interference can add time but
never remove it, so the fastest sample is the least polluted one. With the same
binary on both sides the tool refuses to declare a winner and reports the floor
instead.

On the 12-core laptop used for the tables in the root README, that floor is
about 15% on a loaded machine. Effects below it need an idle box, not more runs.

## Search index cost

`search_index.py` runs the same corpus with `search_index` off and on. It
reports whole-build wall time, the `lists+feeds` phase that writes
`search-index.json`, and output size, then writes `SEARCH_INDEX.md` and
`search-index.json`.

```sh
python3 bench/search_index.py --counts 1000 10000 --runs 9 --profile code
```

## Section-heavy sites

`sections.py` builds a documentation-shaped corpus — N sections, each with a
`_index.md` that has two code blocks and M pages — and reports cold, no-op and
single-edit builds. Section landing pages are Markdown that only has to be
rendered when the listing is written, so this is the corpus that shows it.

```sh
python3 bench/sections.py --sections 300 --per-section 5 --runs 7
```

## Zola comparison

Zola is part of the matrix above. `zola_compare.py` is the focused head-to-head:
it builds the same corpus for both generators and reports cold, rebuild and
single-edit times plus peak RSS and startup, alternating the two generators
inside every scenario so a load spike hits both. Zola only reads TOML
frontmatter, so the generator's YAML is converted on the way in; the Markdown
bodies stay byte-identical.

```sh
python3 bench/zola_compare.py --count 1000 --profile code --runs 5
```

Set `ZOLA_BIN` if `zola` is not on `PATH`. Results and caveats:
[`ZOLA.md`](ZOLA.md).

## Corpus

`gen.py` writes N posts (YAML frontmatter, tags, deterministic prose) into each
project's content directory:

| Profile | Content |
| --- | --- |
| `plain` | three paragraphs, no code |
| `code` | three paragraphs + one Rust code block |
| `heavy` | six paragraphs + three code blocks (Rust/JS/CSS), a table, a footnote |

## Fixtures

- `hugo/`, `zola/` and `astro/` are minimal projects with layouts equivalent to
  the embedded agno theme: single page, list, home, 404, stylesheet.
- `run.py` prepares all four sites under `work/` (gitignored), recreating each
  one from its fixture so no site inherits content from an earlier run. Astro
  dependencies are installed once with `npm install`.
- Zola only reads TOML frontmatter, so `run.py` rewrites the generator's YAML
  frontmatter as TOML for the Zola site. The Markdown bodies are byte-identical.
- agno highlights code with syntect/Oniguruma; Hugo uses Chroma and Zola uses
  Giallo. All three do real syntax highlighting, so their timings include it.
- Astro's fixture renders Markdown pages only: no syntax highlighting,
  taxonomies, RSS or sitemap. Hugo, Zola and agno do all of those here, so
  Astro's timings are optimistic.

## Scenarios

- **Cold build:** output directory deleted before each timed run; a warmup run
  precedes the samples. Command lines are exactly what a user runs.
- **Single-edit rebuild:** one post gets an extra paragraph, then the normal
  build command runs with the previous output present. Zola has no incremental
  mode, so this is a full rebuild for it. Astro is skipped here: its rebuild
  empties the output directory first, and that deletion dominates the run on a
  compressed filesystem (36 s against a 4.8 s cold build at 1,000 posts), so the
  number would measure the disk rather than the generator.
- **Tiny site:** two posts, to show fixed startup cost.
- **Scaling:** 10,000 posts for agno, Hugo and Zola (Astro omitted for runtime).
- **Startup:** `agno --version`, `hugo version`, `astro --version`,
  `zola --version`.
- **RSS / footprint:** peak RSS via `wait4`, binary and `node_modules` size,
  generated output size.

Runs: 7 (100 posts), 5 (1,000), 3 (3,000), 2 (10,000), 10 (tiny/startup).
Medians are reported; raw samples are in `results.json`.

Set `SKIP_ASTRO=1` to skip Astro, or `AGNO_BIN` / `HUGO_BIN` / `ZOLA_BIN` to
point at other binaries.
