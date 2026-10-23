#!/usr/bin/env python3
"""Measure agno's incremental build paths.

    python3 bench/incremental.py --count 1000 --runs 7

Reports cold builds, no-op rebuilds and single-edit rebuilds for one corpus.
Every sample is a fresh process; the fastest run is reported next to the
median because wall-clock timings on a busy machine are noisy.
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
WORK = ROOT / "work" / "incremental"
AGNO = Path(os.environ.get("AGNO_BIN", REPO / "target/release/agno"))


def run(args, **kwargs):
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, **kwargs)


def build(site, *extra):
    start = time.perf_counter()
    run([str(AGNO), "build", "--root", str(site), *extra])
    return (time.perf_counter() - start) * 1000.0


def setup(count: int, profile: str) -> Path:
    site = WORK
    if site.exists():
        shutil.rmtree(site)
    site.mkdir(parents=True)
    run([str(AGNO), "new", str(site)])
    run([sys.executable, str(ROOT / "gen.py"), str(site / "content"), str(count), "--profile", profile])
    return site


def summarise(samples):
    return min(samples), statistics.median(samples)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    parser.add_argument("--runs", type=int, default=7)
    args = parser.parse_args()

    site = setup(args.count, args.profile)
    post = sorted((site / "content" / "blog").glob("*.md"))[0]
    original = post.read_text()

    cold, noop, edit = [], [], []
    for _ in range(args.runs):
        cold.append(build(site, "--clean"))
        noop.append(build(site))

        post.write_text(original + "\n\nAn extra paragraph.\n")
        edit.append(build(site))
        post.write_text(original)

    rows = [
        ("cold build (--clean)", cold),
        ("no-op rebuild", noop),
        ("single-edit rebuild", edit),
    ]
    print(f"\nagno incremental builds: {args.count} posts, profile {args.profile}, {args.runs} runs\n")
    print(f"{'scenario':<24}{'min':>10}{'median':>10}")
    for name, samples in rows:
        low, mid = summarise(samples)
        print(f"{name:<24}{low:>9.1f}ms{mid:>9.1f}ms")
    print()


if __name__ == "__main__":
    main()
