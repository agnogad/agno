# agno vs Zola

Cold builds, rebuilds and single-edit rebuilds on the same Markdown corpus.
Both generators produce the same pages: posts, a section listing, an about page,
tag pages, a feed, a sitemap, `robots.txt` and `404.html`, with real syntax
highlighting.

This is the focused head-to-head. Zola is also part of the main matrix in
[`REPORT.md`](REPORT.md), which adds the corpus profiles and the 10,000-post
scale test.

- agno 0.3.2 + the taxonomy/rebuild branch, release build
- Zola 0.23.6, official `x86_64-unknown-linux-gnu` binary
- Intel i5-13420H (12 threads), CachyOS, Linux 7.2.2, Btrfs (`compress=zstd`)
- 7 runs per cell, medians; wall clock includes process startup
- The two generators alternate inside every scenario, so a load spike hits both

The corpus comes from [`gen.py`](gen.py), the same one used for the Hugo and
Astro matrix. Zola only reads TOML frontmatter, so the YAML frontmatter is
converted before it is handed to Zola; Markdown bodies are byte-identical. Both
sites emit 20 feed items, a sitemap with the same URLs, and the same 10,007
HTML files at the largest size.

```sh
python3 bench/zola_compare.py --count 1000 --profile code --runs 7
```

Raw samples: [`zola-results.json`](zola-results.json).

## Cold build

Output directory removed before every run.

| Posts | Profile | agno | Zola | |
| ---: | --- | ---: | ---: | --- |
| 1,000 | plain | **100.0 ms** | 233.4 ms | 2.3× faster |
| 1,000 | code | **124.2 ms** | 433.2 ms | 3.5× faster |
| 1,000 | heavy | **226.8 ms** | 1,284.9 ms | 5.7× faster |
| 3,000 | heavy | **608.3 ms** | 2,388.7 ms | 3.9× faster |
| 10,000 | code | **1,303.8 ms** | 3,405.2 ms | 2.6× faster |

## Rebuild

Output left in place, no source change. This is where the two designs differ:
agno fingerprints every page and rewrites only what changed, while `zola build`
deletes the output directory and renders the whole site again.

| Posts | Profile | agno | Zola | |
| ---: | --- | ---: | ---: | --- |
| 1,000 | plain | **21.3 ms** | 352.9 ms | 17× faster |
| 1,000 | code | **29.6 ms** | 546.3 ms | 18× faster |
| 1,000 | heavy | **31.8 ms** | 1,317.5 ms | 41× faster |
| 3,000 | heavy | **85.4 ms** | 2,764.1 ms | 32× faster |
| 10,000 | code | **186.9 ms** | 4,990.4 ms | 27× faster |

Zola's rebuild is slower than its cold build because it deletes the previous
output first, so it pays for the removal on top of a full render.

## Single-edit rebuild

One post gets an extra paragraph, then the normal build command runs with the
previous output present.

| Posts | Profile | agno | Zola | |
| ---: | --- | ---: | ---: | --- |
| 1,000 | plain | **21.2 ms** | 346.3 ms | 16× faster |
| 1,000 | code | **27.9 ms** | 568.2 ms | 20× faster |
| 1,000 | heavy | **32.2 ms** | 1,552.7 ms | 48× faster |
| 3,000 | heavy | **84.5 ms** | 2,668.7 ms | 32× faster |
| 10,000 | code | **203.7 ms** | 5,542.3 ms | 27× faster |

## Memory, startup, footprint

Peak RSS of the cold build, `--version` startup, and binary size:

| Posts | Profile | agno RSS | Zola RSS | |
| ---: | --- | ---: | ---: | --- |
| 1,000 | plain | **20.9 MB** | 74.1 MB | 3.5× less |
| 1,000 | code | **21.2 MB** | 76.2 MB | 3.6× less |
| 1,000 | heavy | **24.0 MB** | 114.5 MB | 4.8× less |
| 3,000 | heavy | **51.6 MB** | 171.7 MB | 3.3× less |
| 10,000 | code | **117.3 MB** | 248.8 MB | 2.1× less |

| | agno | Zola |
| --- | ---: | ---: |
| CLI startup (`--version`) | **1.9–2.9 ms** | 12.7–15.9 ms |
| Binary size | **4.3 MB** | 50.8 MB |

## Caveats

- Zola 0.23 highlights code with Giallo (VSCode grammars and themes); agno uses
  syntect/Oniguruma. Both do real highlighting, so both timings include it.
- The 10,000-post cold build is dominated by writing 10,000 files on Btrfs,
  which is why the cold ratio (2.6×) is smaller than at 1,000 posts. Rebuilds
  are not I/O bound and keep a 17–48× gap.
- Templates are equivalent but not identical: both render a title, date, tags,
  content and a page list. Neither minifies.
- Zola cannot reuse output between builds, so its rebuild column is a property
  of the tool, not a tuning choice. `zola build` has no incremental flag.
