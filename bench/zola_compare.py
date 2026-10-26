#!/usr/bin/env python3
"""Benchmark agno against Zola on the same Markdown corpus.

    python3 bench/zola_compare.py --count 1000 --profile code --runs 5

Zola only reads TOML frontmatter, so the generated corpus (YAML) is converted
before it is handed to Zola; the Markdown bodies are byte-identical. Both sites
use equivalent templates, tag pages, a feed, a sitemap and syntax highlighting.

Set AGNO_BIN / ZOLA_BIN to override the binaries.
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
WORK = ROOT / "work" / "zola-compare"
STYLE = REPO / "theme" / "style.css"
AGNO = Path(os.environ.get("AGNO_BIN", REPO / "target/release/agno"))
ZOLA = Path(os.environ.get("ZOLA_BIN", shutil.which("zola") or "/tmp/zola/zola"))

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
    proc = subprocess.run(cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed ({proc.returncode}): {' '.join(map(str, cmd))}\n"
            + proc.stderr.decode(errors="replace")[-4000:]
        )


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
            tags = [item.strip().strip('"') for item in value.strip("[]").split(",") if item.strip()]
        else:
            keys.append(f"{key} = {value}")
    out = "+++\n" + "\n".join(keys) + "\n"
    if tags:
        out += "\n[taxonomies]\ntags = [" + ", ".join(f'"{tag}"' for tag in tags) + "]\n"
    return out + "+++\n" + body


def setup(count: int, profile: str):
    if WORK.exists():
        shutil.rmtree(WORK)
    WORK.mkdir(parents=True)

    agno_site = WORK / "agno"
    run([str(AGNO), "new", str(agno_site)])
    run(
        [
            sys.executable,
            str(ROOT / "gen.py"),
            str(agno_site / "content"),
            str(count),
            "--profile",
            profile,
        ]
    )

    zola_site = WORK / "zola"
    shutil.copytree(ROOT / "zola", zola_site)
    (zola_site / "static").mkdir()
    shutil.copy(STYLE, zola_site / "static" / "style.css")
    (zola_site / "content").mkdir(exist_ok=True)
    (zola_site / "content/_index.md").write_text(HOME, encoding="utf-8")
    (zola_site / "content/about.md").write_text(ABOUT, encoding="utf-8")
    (zola_site / "content/blog").mkdir()
    (zola_site / "content/blog/_index.md").write_text(SECTION, encoding="utf-8")
    for post in sorted((agno_site / "content/blog").glob("*.md")):
        text = post.read_text(encoding="utf-8")
        (zola_site / "content/blog" / post.name).write_text(to_toml(text), encoding="utf-8")
    return agno_site, zola_site


def command(name: str, site: Path):
    if name == "agno":
        return [str(AGNO), "build", "--root", str(site)]
    return [str(ZOLA), "build"]


def output(name: str, site: Path) -> Path:
    return site / ("dist" if name == "agno" else "public")


def time_one(cmd, cwd, out_dir: Path, clean: bool):
    if clean:
        shutil.rmtree(out_dir, ignore_errors=True)
    started = time.perf_counter()
    proc = subprocess.Popen(
        cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    _, status, usage = os.wait4(proc.pid, 0)
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError(f"command failed: {' '.join(map(str, cmd))}")
    return (time.perf_counter() - started) * 1000, usage.ru_maxrss / 1024


def summarise(times, rss):
    ordered = sorted(times)
    return {
        "min_ms": round(ordered[0], 1),
        "median_ms": round(statistics.median(ordered), 1),
        "rss_mb": round(statistics.median(rss), 1),
        "raw_ms": [round(t, 1) for t in times],
    }


def startup(cmd, runs):
    times = []
    for _ in range(runs):
        started = time.perf_counter()
        run(cmd)
        times.append((time.perf_counter() - started) * 1000)
    return round(statistics.median(times), 1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    if not AGNO.exists():
        sys.exit(f"missing {AGNO}; run `cargo build --release` first")
    if not Path(ZOLA).exists():
        sys.exit(f"missing zola binary ({ZOLA}); set ZOLA_BIN")

    agno_site, zola_site = setup(args.count, args.profile)
    post = sorted((agno_site / "content/blog").glob("*.md"))[0]
    zola_post = zola_site / "content/blog" / post.name
    original = post.read_text(encoding="utf-8")
    zola_original = zola_post.read_text(encoding="utf-8")

    sites = {
        "agno": (command("agno", agno_site), None, output("agno", agno_site)),
        "zola": (command("zola", zola_site), zola_site, output("zola", zola_site)),
    }
    for cmd, cwd, target in sites.values():
        shutil.rmtree(target, ignore_errors=True)
        run(cmd, cwd=cwd)  # warmup

    scenarios = ("cold", "rebuild", "edit")
    times = {name: {s: [] for s in scenarios} for name in sites}
    rss = {name: {s: [] for s in scenarios} for name in sites}
    # Alternate the two generators inside every scenario so a load spike hits
    # both instead of one of them.
    for scenario in scenarios:
        for _ in range(args.runs):
            for name, (cmd, cwd, target) in sites.items():
                path, text = (post, original) if name == "agno" else (zola_post, zola_original)
                if scenario == "edit":
                    path.write_text(text + "\nAn extra paragraph.\n", encoding="utf-8")
                elapsed, peak = time_one(cmd, cwd, target, clean=scenario == "cold")
                if scenario == "edit":
                    path.write_text(text, encoding="utf-8")
                times[name][scenario].append(elapsed)
                rss[name][scenario].append(peak)

    results = {
        name: {s: summarise(times[name][s], rss[name][s]) for s in scenarios}
        for name in sites
    }
    for name in sites:
        results[name]["startup_ms"] = startup(
            [str(AGNO), "--version"] if name == "agno" else [str(ZOLA), "--version"], 9
        )

    print(f"\n{args.count} posts ({args.profile}), {args.runs} runs, wall time in ms\n")
    print(f"{'generator':10}{'scenario':10}{'min':>9}{'median':>9}{'rss MB':>9}")
    for name in ("agno", "zola"):
        for scenario in ("cold", "rebuild", "edit"):
            row = results[name][scenario]
            print(
                f"{name:10}{scenario:10}{row['min_ms']:9.1f}{row['median_ms']:9.1f}{row['rss_mb']:9.1f}"
            )
    print(f"\nstartup: agno {results['agno']['startup_ms']} ms · zola {results['zola']['startup_ms']} ms")
    print(f"binary:  agno {AGNO.stat().st_size / 1e6:.1f} MB · zola {Path(ZOLA).stat().st_size / 1e6:.1f} MB")

    payload = {
        "count": args.count,
        "profile": args.profile,
        "runs": args.runs,
        "zola_version": subprocess.run(
            [str(ZOLA), "--version"], capture_output=True, text=True
        ).stdout.strip(),
        "agno_version": subprocess.run(
            [str(AGNO), "--version"], capture_output=True, text=True
        ).stdout.strip(),
        "results": results,
    }
    target = args.json or ROOT / "zola-results.json"
    target.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"wrote {target}")


if __name__ == "__main__":
    main()
