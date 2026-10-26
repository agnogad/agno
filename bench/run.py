#!/usr/bin/env python3
"""Benchmark agno against Hugo, Astro and Zola on the same Markdown corpus.

Usage:
    python3 bench/run.py --count 1000 --runs 5
    python3 bench/run.py --count 1000 --profile plain

Set AGNO_BIN / HUGO_BIN / ZOLA_BIN to override binary paths. Astro deps are
installed on first run with `npm install` (needs network).
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
WORK = ROOT / "work"
STYLE = REPO / "theme" / "style.css"
AGNO = Path(os.environ.get("AGNO_BIN", REPO / "target/release/agno"))
HUGO = Path(os.environ.get("HUGO_BIN") or shutil.which("hugo") or "hugo")
ZOLA = Path(os.environ.get("ZOLA_BIN") or shutil.which("zola") or "/tmp/zola/zola")
NPM = shutil.which("npm")

# Zola scaffolds nothing, so the pages `agno new` creates are written here with
# TOML frontmatter instead.
HOME = """+++
title = "My Agno Site"
+++

Welcome. This site is built with **agno** — a small, fast static site generator.
"""

ABOUT = """+++
title = "About"
description = "About this site"
+++

This is the about page. Edit `content/about.md` to change it.
"""

SECTION = """+++
title = "Blog"
sort_by = "date"
+++
"""


def run(cmd, cwd=None):
    proc = subprocess.run(
        cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed ({proc.returncode}): {' '.join(map(str, cmd))}\n"
            + proc.stderr.decode(errors="replace")[-4000:]
        )


def generate(content_dir: Path, count: int, profile: str = "code"):
    run(
        [
            sys.executable,
            str(ROOT / "gen.py"),
            str(content_dir),
            str(count),
            "--profile",
            profile,
        ]
    )


def setup_agno(count: int, profile: str = "code") -> Path:
    site = WORK / "agno"
    # Recreate the site every time, like the other fixtures: a site left over
    # from an earlier, larger run would otherwise keep its stale sections and
    # output, which makes the comparison unfair.
    if site.exists():
        shutil.rmtree(site)
    run([str(AGNO), "new", str(site)])
    generate(site / "content", count, profile)
    return site


def to_toml(src: str) -> str:
    """Rewrite the generator's YAML frontmatter as Zola-compatible TOML."""
    if not src.startswith("---\n"):
        return src
    end = src.index("\n---", 4)
    front = src[4:end]
    body = src[end + 4:]
    keys = []
    tags = []
    for line in front.strip().splitlines():
        key, _, value = line.partition(":")
        key = key.strip()
        value = value.strip()
        if key == "tags":
            tags = [
                item.strip().strip('"')
                for item in value.strip("[]").split(",")
                if item.strip()
            ]
        else:
            keys.append(f"{key} = {value}")
    out = "+++\n" + "\n".join(keys) + "\n"
    if tags:
        out += "\n[taxonomies]\ntags = [" + ", ".join(f'"{tag}"' for tag in tags) + "]\n"
    return out + "+++\n" + body


def setup_hugo(count: int, profile: str = "code") -> Path:
    site = WORK / "hugo"
    if site.exists():
        shutil.rmtree(site)
    shutil.copytree(ROOT / "hugo", site)
    (site / "static").mkdir(exist_ok=True)
    shutil.copy(STYLE, site / "static" / "style.css")
    generate(site / "content", count, profile)
    return site


def setup_zola(count: int, profile: str = "code") -> Path:
    site = WORK / "zola"
    if site.exists():
        shutil.rmtree(site)
    shutil.copytree(ROOT / "zola", site)
    (site / "static").mkdir(exist_ok=True)
    shutil.copy(STYLE, site / "static" / "style.css")
    (site / "content").mkdir(exist_ok=True)
    (site / "content/_index.md").write_text(HOME, encoding="utf-8")
    (site / "content/about.md").write_text(ABOUT, encoding="utf-8")
    generate(site / "content", count, profile)
    (site / "content/blog/_index.md").write_text(SECTION, encoding="utf-8")
    for post in sorted((site / "content/blog").glob("post-*.md")):
        text = post.read_text(encoding="utf-8")
        post.write_text(to_toml(text), encoding="utf-8")
    return site


def setup_astro(count: int, profile: str = "code") -> Path:
    site = WORK / "astro"
    if site.exists():
        for child in site.iterdir():
            if child.name == "node_modules":
                continue
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()
    shutil.copytree(ROOT / "astro", site, dirs_exist_ok=True)
    (site / "public").mkdir(exist_ok=True)
    (site / "src/content/blog").mkdir(parents=True, exist_ok=True)
    shutil.copy(STYLE, site / "public" / "style.css")
    generate(site / "src/content", count, profile)
    shutil.copy(
        ROOT / "astro/src/content/blog/about.md",
        site / "src/content/blog/about.md",
    )
    if not (site / "node_modules").exists():
        print("installing Astro (first run only)…", file=sys.stderr)
        run([NPM, "install", "--no-audit", "--no-fund"], cwd=site)
    return site


def command(name: str, site: Path):
    if name == "agno":
        return [str(AGNO), "build", "--root", str(site)]
    if name == "hugo":
        return [
            str(HUGO),
            "--source",
            str(site),
            "--destination",
            str(site / "dist"),
            "--quiet",
            "--cleanDestinationDir",
        ]
    if name == "astro":
        return [NPM, "run", "build", "--silent"]
    if name == "zola":
        return [str(ZOLA), "build"]
    raise ValueError(name)


def cwd_for(name: str, site: Path):
    """Directories the generator must be run from, if it has no --root flag."""
    return site if name in ("astro", "zola") else None


def dist(name: str, site: Path) -> Path:
    return site / ("public" if name == "zola" else "dist")


def content_dir(name: str, site: Path) -> Path:
    if name == "astro":
        return site / "src/content/blog"
    return site / "content/blog"


def measure(name: str, site: Path, runs: int):
    cmd = command(name, site)
    cwd = cwd_for(name, site)
    shutil.rmtree(dist(name, site), ignore_errors=True)
    run(cmd, cwd=cwd)  # warmup
    times = []
    rss = []
    for _ in range(runs):
        shutil.rmtree(dist(name, site), ignore_errors=True)
        started = time.perf_counter()
        proc = subprocess.Popen(
            cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        if proc.returncode != 0:
            raise RuntimeError(f"command failed ({proc.returncode}): {' '.join(cmd)}")
        times.append((time.perf_counter() - started) * 1000)
        rss.append(usage.ru_maxrss / 1024)
    times.sort()
    return {
        "generator": name,
        "runs": runs,
        "min_ms": round(times[0], 1),
        "median_ms": round(statistics.median(times), 1),
        "mean_ms": round(statistics.mean(times), 1),
        "max_ms": round(times[-1], 1),
        "rss_mb": round(statistics.median(rss), 1),
        "raw_ms": [round(t, 1) for t in times],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    parser.add_argument("--only", choices=["agno", "hugo", "astro", "zola"])
    args = parser.parse_args()

    WORK.mkdir(exist_ok=True)
    sites = {}
    if args.only in (None, "agno"):
        if not AGNO.exists():
            sys.exit(f"missing {AGNO}; run `cargo build --release` first")
        sites["agno"] = setup_agno(args.count, args.profile)
    if args.only in (None, "hugo"):
        if not HUGO.exists():
            sys.exit(f"missing hugo binary ({HUGO}); set HUGO_BIN")
        sites["hugo"] = setup_hugo(args.count, args.profile)
    if args.only in (None, "astro"):
        if not NPM:
            sys.exit("missing npm")
        sites["astro"] = setup_astro(args.count, args.profile)
    if args.only in (None, "zola"):
        if not ZOLA.exists():
            sys.exit(f"missing zola binary ({ZOLA}); set ZOLA_BIN")
        sites["zola"] = setup_zola(args.count, args.profile)

    results = [measure(name, site, args.runs) for name, site in sites.items()]
    for row in results:
        row.update(profile=args.profile, count=args.count)
    (ROOT / "results.json").write_text(json.dumps(results, indent=2) + "\n")

    print(f"\n{args.count} posts ({args.profile}), {args.runs} cold runs, wall time in ms\n")
    print(f"{'generator':10} {'min':>8} {'median':>8} {'mean':>8} {'max':>8} {'rss MB':>8}")
    for row in results:
        print(
            f"{row['generator']:10} {row['min_ms']:8.1f} {row['median_ms']:8.1f} "
            f"{row['mean_ms']:8.1f} {row['max_ms']:8.1f} {row['rss_mb']:8.1f}"
        )
    print(f"\nwrote {ROOT / 'results.json'}")


if __name__ == "__main__":
    main()
