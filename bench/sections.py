#!/usr/bin/env python3
"""Measure builds on a section-heavy (documentation-shaped) corpus.

    python3 bench/sections.py --sections 300 --per-section 5 --runs 7

Every section has an `_index.md` with two code blocks, which is the shape where
section landing pages matter: they are Markdown that has to be parsed and
highlighted, but only when the listing is actually written.
"""
import argparse
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
WORK = ROOT / "work" / "sections"
AGNO = Path(os.environ.get("AGNO_BIN", REPO / "target/release/agno"))

LANDING = """+++
title = "Section {index}"
+++

# Section {index}

Landing page body with a code sample.

```rust
fn main() {{
    println!("docs");
}}
```

```js
const x = [1, 2, 3].map((n) => n * 2);
```
"""

PAGE = """---
title: "Section {index} page {page}"
date: 2024-05-01
tags: ["docs"]
---

A short documentation page.

```rust
fn example() -> u32 {{ 42 }}
```
"""


def run(args, **kwargs):
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, **kwargs)


def build(site, *extra):
    start = time.perf_counter()
    run([str(AGNO), "build", "--root", str(site), *extra])
    return (time.perf_counter() - start) * 1000.0


def setup(sections: int, per_section: int) -> Path:
    site = WORK
    if site.exists():
        shutil.rmtree(site)
    site.mkdir(parents=True)
    run([str(AGNO), "new", str(site)])

    content = site / "content"
    links = []
    for index in range(sections):
        name = f"section-{index:03d}"
        directory = content / name
        directory.mkdir()
        (directory / "_index.md").write_text(LANDING.format(index=index), encoding="utf-8")
        for page in range(per_section):
            (directory / f"page-{page}.md").write_text(
                PAGE.format(index=index, page=page), encoding="utf-8"
            )
        links.append(f"- [{name}](/{name}/)")

    (content / "_index.md").write_text("# Docs\n\n" + "\n".join(links) + "\n", encoding="utf-8")
    return site


def summarise(samples):
    return min(samples), statistics.median(samples)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sections", type=int, default=300)
    parser.add_argument("--per-section", type=int, default=5)
    parser.add_argument("--runs", type=int, default=7)
    args = parser.parse_args()

    site = setup(args.sections, args.per_section)
    page = sorted((site / "content" / "section-000").glob("page-*.md"))[0]
    original = page.read_text(encoding="utf-8")

    cold, noop, edit = [], [], []
    for _ in range(args.runs):
        cold.append(build(site, "--clean"))
        noop.append(build(site))

        page.write_text(original + "\nAn extra paragraph.\n", encoding="utf-8")
        edit.append(build(site))
        page.write_text(original, encoding="utf-8")

    pages = args.sections * (args.per_section + 1) + 1
    rows = [
        ("cold build (--clean)", cold),
        ("no-op rebuild", noop),
        ("single-edit rebuild", edit),
    ]
    print(
        f"\nagno section builds: {args.sections} sections, {pages} pages, {args.runs} runs\n"
    )
    print(f"{'scenario':<24}{'min':>10}{'median':>10}")
    for name, samples in rows:
        low, mid = summarise(samples)
        print(f"{name:<24}{low:>9.1f}ms{mid:>9.1f}ms")
    print()


if __name__ == "__main__":
    main()
