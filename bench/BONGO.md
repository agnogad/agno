# agno vs bongo

Cold builds, no-change rebuilds and single-edit rebuilds on the same Markdown
corpus. bongo is a TypeScript static site generator that runs on Node or Bun
and bundles with Rolldown and LightningCSS; agno is a single Rust binary.

- agno 0.3.7, release build
- bongo 1.2.0 (`bun create bongo` scaffold), Node 26.7.0 or Bun 1.4.0
- Intel i5-13420H (12 threads), CachyOS, Linux, Btrfs (`compress=zstd`)
- 5 runs per cell at 1,000 posts, 3 at 10,000; medians; wall clock includes
  process startup
- The two generators alternate inside every scenario, so a load spike hits both

The corpus comes from [`gen.py`](gen.py), the same one used for the Hugo, Zola
and Astro matrix. Both sites are the stock scaffold (`agno new` and
`bun create bongo`); bongo's gets a sitemap and `robots.txt` so the output sets
are close. agno additionally writes tag pages, a feed and `404.html`, which is
a little more work per build.

bongo is measured twice:

- **Node + Shiki** — the scaffold's default: `bongo build` on Node, Shiki
  highlighting the Rust/JS/CSS blocks.
- **Bun + Sugar High** — bongo at its fastest here: `bun --bun run build` with
  the lightweight highlighter from the docs.

The two runtimes cannot share a site: bongo keys its build cache on the runtime,
so alternating them would throw the incremental results away.

```sh
python3 bench/bongo_compare.py --count 1000 --profile code --runs 5
python3 bench/bongo_compare.py --count 10000 --profile code --runs 3 \
    --highlighter sugarhigh --runtime bun
```

Raw samples: [`bongo-1000-code.json`](bongo-1000-code.json),
[`bongo-10000-code.json`](bongo-10000-code.json),
[`bongo-1000-code-sugarhigh-bun.json`](bongo-1000-code-sugarhigh-bun.json),
[`bongo-10000-code-sugarhigh-bun.json`](bongo-10000-code-sugarhigh-bun.json).

## Cold build

Output directory and build cache removed before every run.

| Posts | agno | bongo (Node, Shiki) | bongo (Bun, Sugar High) | agno vs best bongo |
| ---: | ---: | ---: | ---: | --- |
| 1,000 | **129 ms** | 1,839 ms | 1,079 ms | **8.4× faster** |
| 10,000 | **1,488 ms** | 10,903 ms | 6,172 ms | **4.1× faster** |

## Rebuild, no change

Output left in place, nothing edited. Both tools are incremental: agno keeps
fingerprints in `.agno-cache`, bongo in `node_modules/.cache/bongo/memo.json`.

| Posts | agno | bongo (Node, Shiki) | bongo (Bun, Sugar High) | agno vs best bongo |
| ---: | ---: | ---: | ---: | --- |
| 1,000 | **22 ms** | 581 ms | 381 ms | **17× faster** |
| 10,000 | **225 ms** | 1,473 ms | 932 ms | **4.2× faster** |

## Single-edit rebuild

One post gets an extra paragraph, then the normal build command runs with the
previous output present.

| Posts | agno | bongo (Node, Shiki) | bongo (Bun, Sugar High) | agno vs best bongo |
| ---: | ---: | ---: | ---: | --- |
| 1,000 | **23 ms** | 586 ms | 384 ms | **17× faster** |
| 10,000 | **221 ms** | 1,520 ms | 935 ms | **4.2× faster** |

## Memory, startup, footprint

Peak RSS of the cold build and `--version` startup:

| Posts | agno RSS | bongo (Node, Shiki) | bongo (Bun, Sugar High) |
| ---: | ---: | ---: | ---: |
| 1,000 | **22 MB** | 266 MB | 192 MB |
| 10,000 | **114 MB** | 666 MB | 329 MB |

| | agno | bongo |
| --- | ---: | ---: |
| CLI startup (`--version`) | **2 ms** | 58–65 ms |
| Output, 10,000 posts | 10,011 files / 35.8 MB | 10,009 files / 39.6 MB |

## Caveats

- Distances above are ratios on this machine. bongo publishes 2.19 s for 10,000
  pages on an M1 Max with Bun 1.4.3 and twinkleplop; this i5 is slower per core
  and renders the same 10,000 pages in 6.17 s with bongo's best options here.
  agno's own 10,000-post number on this machine (1.3–1.5 s) matches its README,
  so the corpus and the machine agree with the published benchmarks.
- The highlighter matters less than the runtime: Sugar High saves about 18% over
  Shiki at 10,000 posts on Node, while switching Node to Bun saves another 31%.
- bongo bundles the scaffold's CSS and client JS on every build; agno ships a
  built-in theme stylesheet and no client JS. Both costs are included.
- bongo's scaffold renders a post page, a section listing and three static
  pages. agno also writes tag pages, RSS, `robots.txt`, `sitemap.xml` and
  `404.html`, so agno is doing slightly more work per build, not less.
- bongo's advantages are elsewhere: one typed TypeScript config, Rolldown and
  LightningCSS, image resizing, data modules, and a client runtime. Those were
  not measured here.
