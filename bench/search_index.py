#!/usr/bin/env python3
"""Measure the build-time and output-size cost of agno's search index.

Runs each corpus twice: once with `search_index = false`, once with
`search_index = true`. Runs alternate between the two settings so machine drift
affects both. Both are cold builds (the output directory is removed before
every timed run). Writes `bench/search-index.json` and `bench/SEARCH_INDEX.md`.

The `lists+feeds` phase from `agno build --stats` contains the search-index
write. Its delta is the index cost with the content and page-rendering phases
removed from the comparison.

Usage:
    python3 bench/search_index.py
    python3 bench/search_index.py --counts 1000 --runs 10
"""
import argparse
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import run as base  # noqa: E402

DEFAULT_COUNTS = [1000, 10000]


def set_search_index(site: Path, enabled: bool) -> None:
    path = site / "agno.toml"
    text = re.sub(r"(?m)^search_index\s*=.*\n?", "", path.read_text())
    # Root keys must precede TOML tables such as [[menu]].
    path.write_text(f"search_index = {str(enabled).lower()}\n" + text.lstrip())


def output_stats(dist: Path) -> tuple[int, int]:
    total = 0
    for path in dist.rglob("*"):
        if path.is_file():
            total += path.stat().st_size
    index = dist / "search-index.json"
    return total, index.stat().st_size if index.exists() else 0


def build_once(site: Path) -> tuple[float, int]:
    """Return (wall ms, lists+feeds ms) for one cold build."""
    cmd = [str(base.AGNO), "build", "--stats", "--root", str(site)]
    started = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    elapsed = (time.perf_counter() - started) * 1000
    if proc.returncode != 0:
        raise RuntimeError(f"agno build failed:\n{proc.stderr[-2000:]}")
    match = re.search(r"lists\+feeds (\d+) ms", proc.stdout)
    if not match:
        raise RuntimeError(f"missing stats line:\n{proc.stdout}")
    return elapsed, int(match.group(1))


def summarize(samples: list[float]) -> dict:
    values = sorted(samples)
    return {
        "runs": len(values),
        "min_ms": round(values[0], 1),
        "median_ms": round(statistics.median(values), 1),
        "mean_ms": round(statistics.mean(values), 1),
        "max_ms": round(values[-1], 1),
        "stdev_ms": round(statistics.stdev(values), 1) if len(values) > 1 else 0.0,
        "raw_ms": [round(value, 1) for value in values],
    }


def cpu_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def version_of() -> str:
    try:
        return subprocess.check_output(
            [str(base.AGNO), "--version"], text=True
        ).strip()
    except Exception:
        return "unknown"


def measure_count(site: Path, count: int, profile: str, runs: int) -> list[dict]:
    dist = base.dist("agno", site)
    output = {}

    # Warm both settings once; keep the generated output for size comparisons.
    for enabled in (False, True):
        set_search_index(site, enabled)
        shutil.rmtree(dist, ignore_errors=True)
        build_once(site)
        total, index = output_stats(dist)
        if enabled != bool(index):
            raise RuntimeError(
                f"search_index={enabled} but search-index.json is {index} bytes"
            )
        output[enabled] = {
            "output_mb": round(total / 1_000_000, 2),
            "index_mb": round(index / 1_000_000, 2),
            "index_bytes": index,
        }

    wall = {False: [], True: []}
    rest = {False: [], True: []}
    for _ in range(runs):
        for enabled in (False, True):
            set_search_index(site, enabled)
            shutil.rmtree(dist, ignore_errors=True)
            elapsed, rest_ms = build_once(site)
            wall[enabled].append(elapsed)
            rest[enabled].append(rest_ms)

    records = []
    for enabled in (False, True):
        record = {
            "scenario": "search_index",
            "search_index": enabled,
            "count": count,
            "profile": profile,
            **summarize(wall[enabled]),
            "rest_median_ms": round(statistics.median(rest[enabled]), 1),
            "rest_min_ms": min(rest[enabled]),
            "rest_raw_ms": rest[enabled],
            **output[enabled],
        }
        records.append(record)
        print(
            f"  {count:5} posts  search_index={'on ' if enabled else 'off'} "
            f"wall {record['median_ms']:8.1f} ms  "
            f"lists+feeds {record['rest_median_ms']:6.1f} ms  "
            f"{record['output_mb']:6.2f} MB  "
            f"index {record['index_mb']:.2f} MB",
            flush=True,
        )
    return records


def write_report(records: list[dict], profile: str, runs: int) -> None:
    by_count = {}
    for record in records:
        by_count.setdefault(record["count"], {})[record["search_index"]] = record

    lines = [
        "# Search index cost\n",
        f"Generated by `bench/search_index.py` with {version_of()} on "
        f"{platform.platform()}.\n",
        f"- CPU: {cpu_model()} ({os.cpu_count() or 0} threads)",
        f"- Corpus: `{profile}` profile; both settings use the same content",
        f"- Runs: {runs} cold builds per setting per corpus, alternated",
        "- The output directory is removed before every timed run.",
        "- `lists+feeds` is the build phase that writes the search index; its "
        "delta isolates the index cost from content loading and page rendering.",
        "",
        "## Results\n",
        "| Posts | `search_index` | Wall median | `lists+feeds` median | Output | `search-index.json` |",
        "| ---: | --- | ---: | ---: | ---: | ---: |",
    ]

    for count in sorted(by_count):
        off = by_count[count][False]
        on = by_count[count][True]
        delta_wall = on["median_ms"] - off["median_ms"]
        delta_rest = on["rest_median_ms"] - off["rest_median_ms"]
        delta_mb = on["output_mb"] - off["output_mb"]
        lines.append(
            f"| {count:,} | off | {off['median_ms']:.1f} ms | "
            f"{off['rest_median_ms']:.1f} ms | {off['output_mb']:.2f} MB | — |"
        )
        lines.append(
            f"| {count:,} | on | {on['median_ms']:.1f} ms | "
            f"{on['rest_median_ms']:.1f} ms | {on['output_mb']:.2f} MB | "
            f"{on['index_mb']:.2f} MB |"
        )
        lines.append(
            f"| {count:,} | **delta** | **{delta_wall:+.1f} ms** | "
            f"**{delta_rest:+.1f} ms** | **{delta_mb:+.2f} MB** | "
            f"**+{on['index_mb']:.2f} MB** |"
        )

    lines.extend(["", "## Raw samples (ms)\n"])
    for count in sorted(by_count):
        lines.append(f"### {count:,} posts\n")
        for enabled in (False, True):
            record = by_count[count][enabled]
            label = "on" if enabled else "off"
            lines.append(
                f"- wall {label}: {', '.join(f'{x:.1f}' for x in record['raw_ms'])}"
            )
            lines.append(
                f"- lists+feeds {label}: "
                f"{', '.join(str(x) for x in record['rest_raw_ms'])}"
            )
        lines.append("")

    lines.extend(
        [
            "The index stores title, URL, date, description, summary, and tags "
            "for each page. It does not change the generated HTML; only "
            "`search-index.json` is added to the output.",
            "",
            "Reproduce with:",
            "",
            "```sh",
            f"python3 bench/search_index.py --counts {' '.join(str(c) for c in sorted(by_count))} --runs {runs} --profile {profile}",
            "```",
        ]
    )
    (ROOT / "SEARCH_INDEX.md").write_text("\n".join(lines) + "\n")
    print(f"\nwrote {ROOT / 'SEARCH_INDEX.md'}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--counts", type=int, nargs="+", default=DEFAULT_COUNTS)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    args = parser.parse_args()

    if not base.AGNO.exists():
        sys.exit(f"missing {base.AGNO}; run `cargo build --release` first")

    base.WORK.mkdir(exist_ok=True)
    records = []
    for count in args.counts:
        site = base.setup_agno(count, args.profile)
        records.extend(measure_count(site, count, args.profile, args.runs))

    (ROOT / "search-index.json").write_text(json.dumps(records, indent=2) + "\n")
    write_report(records, args.profile, args.runs)


if __name__ == "__main__":
    main()
