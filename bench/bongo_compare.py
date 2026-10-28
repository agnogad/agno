#!/usr/bin/env python3
"""Benchmark agno against bongo on the same Markdown corpus.

    python3 bench/bongo_compare.py --count 1000 --profile code --runs 5

Both sites get the posts that `gen.py` writes. agno's site is the `agno new`
scaffold; bongo's is the `bun create bongo` scaffold in `bench/bongo/` plus a
sitemap and robots.txt. bongo runs through Node, its default launcher. A Bun
run (`--bun`) was measured too and landed inside the noise, but it cannot share
this site: bongo keys its build cache on the runtime, so alternating runtimes
would throw the incremental results away.

Set AGNO_BIN to override the agno binary. bongo needs `bun` on PATH.
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
sys.path.insert(0, str(ROOT))
import run as base  # noqa: E402

AGNO = base.AGNO
BONGO_FIXTURE = ROOT / "bongo"
BUN = shutil.which("bun")


def run(cmd, cwd=None):
    proc = subprocess.run(cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed ({proc.returncode}): {' '.join(map(str, cmd))}\n"
            + proc.stderr.decode(errors="replace")[-4000:]
        )


def setup_bongo(count: int, profile: str, highlighter: str) -> Path:
    site = base.WORK / "bongo-site"
    if site.exists():
        shutil.rmtree(site)
    shutil.copytree(
        BONGO_FIXTURE,
        site,
        ignore=shutil.ignore_patterns("node_modules", "public", "bun.lock"),
    )
    if highlighter == "sugarhigh":
        shutil.copy(site / "bongo.config.sugarhigh.ts", site / "bongo.config.ts")
    (site / "node_modules").symlink_to(BONGO_FIXTURE / "node_modules", target_is_directory=True)
    base.generate(site / "content", count, profile)
    # gen.py replaces content/blog, which takes the section page with it.
    shutil.copy(BONGO_FIXTURE / "content/blog/index.hbs", site / "content/blog/index.hbs")
    return site


def series(agno_site: Path, bongo_site: Path, runtime: str):
    bongo_cmd = (
        [BUN, "--bun", "run", "build"]
        if runtime == "bun"
        else [str(bongo_site / "node_modules/.bin/bongo"), "build"]
    )
    return {
        "agno": {
            "cmd": [str(AGNO), "build", "--root", str(agno_site)],
            "cwd": None,
            "out": base.dist("agno", agno_site),
            "cache": [agno_site / ".agno-cache"],
            "post": agno_site / "content/blog/post-0000.md",
        },
        "bongo": {
            "cmd": bongo_cmd,
            "cwd": bongo_site,
            "out": bongo_site / "public",
            "cache": [bongo_site / "node_modules/.cache/bongo"],
            "post": bongo_site / "content/blog/post-0000.md",
        },
    }


def time_one(entry, scenario: str, original: str):
    if scenario == "cold":
        shutil.rmtree(entry["out"], ignore_errors=True)
        for path in entry["cache"]:
            shutil.rmtree(path, ignore_errors=True)
    if scenario == "edit":
        entry["post"].write_text(original + "\nAn extra paragraph.\n", encoding="utf-8")
    started = time.perf_counter()
    proc = subprocess.Popen(
        entry["cmd"], cwd=entry["cwd"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    _, status, usage = os.wait4(proc.pid, 0)
    elapsed = (time.perf_counter() - started) * 1000
    if scenario == "edit":
        entry["post"].write_text(original, encoding="utf-8")
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError(f"command failed: {' '.join(map(str, entry['cmd']))}")
    return elapsed, usage.ru_maxrss / 1024


def summarise(times, rss):
    ordered = sorted(times)
    return {
        "min_ms": round(ordered[0], 1),
        "median_ms": round(statistics.median(ordered), 1),
        "rss_mb": round(statistics.median(rss), 1),
        "raw_ms": [round(t, 1) for t in times],
    }


def footprint(path: Path):
    files = [p for p in path.rglob("*") if p.is_file()]
    return len(files), round(sum(p.stat().st_size for p in files) / 1e6, 1)


def startup(cmd, cwd, runs=9):
    times = []
    for _ in range(runs):
        started = time.perf_counter()
        run(cmd, cwd)
        times.append((time.perf_counter() - started) * 1000)
    return round(statistics.median(times), 1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--highlighter", default="shiki", choices=["shiki", "sugarhigh"])
    parser.add_argument("--runtime", default="node", choices=["node", "bun"])
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    if not AGNO.exists():
        sys.exit(f"missing {AGNO}; run `cargo build --release` first")
    if not BUN:
        sys.exit("missing bun")
    if not (BONGO_FIXTURE / "node_modules").exists():
        print("installing bongo (first run only)…", file=sys.stderr)
        run([BUN, "install"], cwd=BONGO_FIXTURE)

    agno_site = base.setup_agno(args.count, args.profile)
    bongo_site = setup_bongo(args.count, args.profile, args.highlighter)
    entries = series(agno_site, bongo_site, args.runtime)
    originals = {name: e["post"].read_text(encoding="utf-8") for name, e in entries.items()}

    for entry in entries.values():
        shutil.rmtree(entry["out"], ignore_errors=True)
        run(entry["cmd"], cwd=entry["cwd"])  # warmup

    scenarios = ("cold", "noop", "edit")
    times = {name: {s: [] for s in scenarios} for name in entries}
    rss = {name: {s: [] for s in scenarios} for name in entries}
    order = list(entries)
    for scenario in scenarios:
        for i in range(args.runs):
            # Rotate the order so a load spike hits a different generator each run.
            rotated = order[i % len(order):] + order[: i % len(order)]
            for name in rotated:
                elapsed, peak = time_one(entries[name], scenario, originals[name])
                times[name][scenario].append(elapsed)
                rss[name][scenario].append(peak)

    results = {
        name: {s: summarise(times[name][s], rss[name][s]) for s in scenarios}
        for name in entries
    }
    for name, entry in entries.items():
        files, mb = footprint(entry["out"])
        results[name]["out_files"] = files
        results[name]["out_mb"] = mb
    results["agno"]["startup_ms"] = startup([str(AGNO), "--version"], None)
    results["bongo"]["startup_ms"] = startup(
        [str(bongo_site / "node_modules/.bin/bongo"), "--version"], bongo_site
    )
    results["bongo"]["runtime"] = args.runtime

    print(f"\n{args.count} posts ({args.profile}), {args.runs} runs, wall time in ms\n")
    print(f"{'generator':12}{'scenario':10}{'min':>9}{'median':>9}{'rss MB':>9}")
    for name in entries:
        for scenario in scenarios:
            row = results[name][scenario]
            print(
                f"{name:12}{scenario:10}{row['min_ms']:9.1f}{row['median_ms']:9.1f}"
                f"{row['rss_mb']:9.1f}"
            )
    print()
    for name in entries:
        row = results[name]
        print(
            f"{name:12}out {row['out_files']} files, {row['out_mb']} MB · "
            f"startup {row['startup_ms']} ms"
        )

    ratios = {}
    for scenario in scenarios:
        ratios[scenario] = {
            name: round(results["agno"][scenario]["median_ms"] / results[name][scenario]["median_ms"], 2)
            for name in entries
        }
    print("\nagno / other, median (below 1 = agno faster):")
    for scenario in scenarios:
        print(f"  {scenario:8} " + "  ".join(f"{k} {v:.2f}×" for k, v in ratios[scenario].items()))

    payload = {
        "count": args.count,
        "profile": args.profile,
        "runs": args.runs,
        "bongo_highlighter": args.highlighter,
        "bongo_runtime": args.runtime,
        "agno_version": subprocess.run([str(AGNO), "--version"], capture_output=True, text=True).stdout.strip(),
        "bongo_version": "1.2.0",
        "node": subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip(),
        "bun": subprocess.run([BUN, "--version"], capture_output=True, text=True).stdout.strip(),
        "results": results,
        "ratios": ratios,
    }
    target = args.json or ROOT / "bongo-results.json"
    target.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"\nwrote {target}")


if __name__ == "__main__":
    main()
