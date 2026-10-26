#!/usr/bin/env python3
"""Detailed benchmark matrix for agno vs Hugo vs Astro vs Zola.

Cold builds across corpus sizes and content profiles, single-edit rebuilds,
peak RSS, startup latency, a tiny site, a 10k-page scaling check, and setup
footprints. Writes bench/results.json and bench/REPORT.md.

Usage:
    python3 bench/suite.py
"""
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import run as base  # noqa: E402

PROFILES = ["plain", "code", "heavy"]
SIZES = [100, 1000, 3000]
RUNS = {100: 7, 1000: 5, 3000: 3}
GENERATORS = ["agno", "hugo", "astro", "zola"]
LABELS = {"agno": "agno", "hugo": "Hugo", "astro": "Astro", "zola": "Zola"}
# Astro is far too slow at 10k pages to be worth the wall clock.
SCALE_GENERATORS = ["agno", "hugo", "zola"]
# Astro's rebuild deletes the whole output tree before rendering; on a
# compressed Btrfs volume that deletion dominates the run (36 s against a 4.8 s
# cold build at 1,000 posts), so the number measures the filesystem instead of
# the generator. Rebuilds are measured for the three generators that keep or
# rewrite their output.
REBUILD_GENERATORS = ["agno", "hugo", "zola"]
TINY_RUNS = 10
SCALE_COUNT = 10000
SCALE_RUNS = 2
STARTUP_RUNS = 10


def execute(cmd, cwd=None):
    started = time.perf_counter()
    with tempfile.TemporaryFile() as stderr:
        proc = subprocess.Popen(
            cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=stderr
        )
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        elapsed = (time.perf_counter() - started) * 1000
        if proc.returncode != 0:
            stderr.seek(0)
            detail = stderr.read().decode(errors="replace")[-4000:]
            raise RuntimeError(
                f"command failed ({proc.returncode}): {' '.join(cmd)}\n{detail}"
            )
    return elapsed, usage.ru_maxrss / 1024


def summarize(samples):
    times = sorted(t for t, _ in samples)
    rss = [r for _, r in samples]
    return {
        "runs": len(times),
        "min_ms": round(times[0], 1),
        "median_ms": round(statistics.median(times), 1),
        "mean_ms": round(statistics.mean(times), 1),
        "max_ms": round(times[-1], 1),
        "stdev_ms": round(statistics.stdev(times), 1) if len(times) > 1 else 0.0,
        "rss_mb": round(statistics.median(rss), 1),
        "raw_ms": [round(t, 1) for t in times],
    }


def dir_size(path: Path) -> int:
    total = 0
    if not path.exists():
        return 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    return total


def html_count(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for _, _, files in os.walk(path) for name in files if name.endswith(".html"))


def setup_all(count, profile):
    sites = {
        "agno": base.setup_agno(count, profile),
        "hugo": base.setup_hugo(count, profile),
        "zola": base.setup_zola(count, profile),
    }
    if os.environ.get("SKIP_ASTRO") != "1":
        sites["astro"] = base.setup_astro(count, profile)
    return sites


def cold_builds(sites, count, profile):
    records = []
    for name, site in sites.items():
        cmd = base.command(name, site)
        cwd = base.cwd_for(name, site)
        dist = base.dist(name, site)
        shutil.rmtree(dist, ignore_errors=True)
        execute(cmd, cwd)  # warmup
        samples = []
        for _ in range(RUNS[count]):
            shutil.rmtree(dist, ignore_errors=True)
            samples.append(execute(cmd, cwd))
        record = {
            "scenario": "cold",
            "profile": profile,
            "count": count,
            "generator": name,
            **summarize(samples),
            "output_mb": round(dir_size(dist) / 1_000_000, 1),
            "html_files": html_count(dist),
        }
        records.append(record)
        print(
            f"  cold {profile:5} {count:5} {name:6} "
            f"{record['median_ms']:8.1f} ms  {record['rss_mb']:6.1f} MB",
            flush=True,
        )
    return records


def rebuilds(sites, count, profile):
    records = []
    for name, site in sites.items():
        if name not in REBUILD_GENERATORS:
            continue
        cmd = base.command(name, site)
        cwd = base.cwd_for(name, site)
        dist = base.dist(name, site)
        if not dist.exists():
            execute(cmd, cwd)
        execute(cmd, cwd)  # warmup, no edit
        target = base.content_dir(name, site) / "post-0000.md"
        samples = []
        for i in range(RUNS[count]):
            with open(target, "a", encoding="utf-8") as fh:
                fh.write(f"\nRebuild edit {i}.\n")
            samples.append(execute(cmd, cwd))
        record = {
            "scenario": "rebuild",
            "profile": profile,
            "count": count,
            "generator": name,
            **summarize(samples),
        }
        records.append(record)
        print(
            f"  edit {profile:5} {count:5} {name:6} "
            f"{record['median_ms']:8.1f} ms  {record['rss_mb']:6.1f} MB",
            flush=True,
        )
    return records


def tiny_site(records):
    sites = setup_all(2, "code")
    for name, site in sites.items():
        cmd = base.command(name, site)
        cwd = base.cwd_for(name, site)
        dist = base.dist(name, site)
        shutil.rmtree(dist, ignore_errors=True)
        execute(cmd, cwd)
        samples = []
        for _ in range(TINY_RUNS):
            shutil.rmtree(dist, ignore_errors=True)
            samples.append(execute(cmd, cwd))
        record = {
            "scenario": "tiny",
            "profile": "code",
            "count": 2,
            "generator": name,
            **summarize(samples),
        }
        records.append(record)
        print(
            f"  tiny {name:6} {record['median_ms']:8.1f} ms  {record['rss_mb']:6.1f} MB",
            flush=True,
        )


def scale_test(records):
    sites = {
        name: getattr(base, f"setup_{name}")(SCALE_COUNT, "code")
        for name in SCALE_GENERATORS
    }
    for name, site in sites.items():
        cmd = base.command(name, site)
        cwd = base.cwd_for(name, site)
        dist = base.dist(name, site)
        shutil.rmtree(dist, ignore_errors=True)
        execute(cmd, cwd)
        samples = []
        for _ in range(SCALE_RUNS):
            shutil.rmtree(dist, ignore_errors=True)
            samples.append(execute(cmd, cwd))
        record = {
            "scenario": "scale",
            "profile": "code",
            "count": SCALE_COUNT,
            "generator": name,
            **summarize(samples),
        }
        records.append(record)
        print(
            f"  scale {name:6} {record['median_ms']:8.1f} ms  {record['rss_mb']:6.1f} MB",
            flush=True,
        )


def startup(records):
    astro_bin = base.WORK / "astro" / "node_modules" / ".bin" / "astro"
    commands = {
        "agno": [str(base.AGNO), "--version"],
        "hugo": [str(base.HUGO), "version"],
        "astro": [str(astro_bin), "--version"],
        "zola": [str(base.ZOLA), "--version"],
    }
    for name, cmd in commands.items():
        cwd = base.WORK / "astro" if name == "astro" else None
        if not Path(cmd[0]).exists():
            continue
        execute(cmd, cwd)
        samples = [execute(cmd, cwd) for _ in range(STARTUP_RUNS)]
        record = {
            "scenario": "startup",
            "profile": "-",
            "count": 0,
            "generator": name,
            **summarize(samples),
        }
        records.append(record)
        print(
            f"  start {name:6} {record['median_ms']:8.1f} ms",
            flush=True,
        )


def fmt(value):
    return f"{value:.1f}"


def speedup(slow, fast):
    return f"{slow / fast:.1f}×" if fast else "-"


def md(cells):
    return "| " + " | ".join(cells) + " |"


def value(record, key="median_ms"):
    return fmt(record[key]) if record else "-"


def versus(fast, slow):
    if not (fast and slow):
        return "-"
    return speedup(slow["median_ms"], fast["median_ms"])


def table_head(lead, generators, ratios):
    cols = lead + [LABELS[g] for g in generators]
    cols += [f"agno vs {LABELS[g]}" for g in ratios]
    align = ["---:"] + ["---"] * (len(lead) - 1)
    align += ["---:"] * (len(generators) + len(ratios))
    return [md(cols), md(align)]


def write_report(records):
    def row(scenario, profile, count, generator):
        for record in records:
            if (
                record["scenario"] == scenario
                and record["profile"] == profile
                and record["count"] == count
                and record["generator"] == generator
            ):
                return record
        return None

    lines = []
    lines.append("# Benchmark report\n")
    lines.append(
        "Generated by `bench/suite.py`. Cold builds delete the output directory "
        "before every timed run; process startup is included. Times are wall "
        "clock, median of the stated runs.\n"
    )

    cpu = subprocess.run(
        ["lscpu"], capture_output=True, text=True, check=False
    ).stdout
    model = next((l.split(":", 1)[1].strip() for l in cpu.splitlines() if "Model name" in l), "unknown")
    threads = os.cpu_count() or 0
    kernel = platform_release()
    lines.append(f"- CPU: {model} ({threads} threads)")
    lines.append(f"- Kernel: {kernel}")
    lines.append(f"- agno: {version_of([str(base.AGNO), '--version'])}")
    lines.append(f"- Hugo: {base.HUGO.name} ({version_of([str(base.HUGO), 'version'])})")
    lines.append(f"- Astro: {astro_version()}")
    lines.append(f"- Zola: {zola_version()}")
    lines.append("")

    lines.append("## Cold build (median ms)\n")
    others = [g for g in GENERATORS if g != "agno"]
    lines += table_head(["Posts", "Profile"], GENERATORS, others)
    for profile in PROFILES:
        for count in SIZES:
            cells = [row("cold", profile, count, g) for g in GENERATORS]
            if not cells[0]:
                continue
            cols = [str(count), profile] + [value(c) for c in cells]
            cols += [versus(cells[0], row("cold", profile, count, g)) for g in others]
            lines.append(md(cols))
    lines.append("")

    lines.append("## Single-edit rebuild (median ms)\n")
    rebuild_ratios = [g for g in REBUILD_GENERATORS if g != "agno"]
    lines += table_head(["Posts"], REBUILD_GENERATORS, rebuild_ratios)
    for count in SIZES:
        cells = [row("rebuild", "code", count, g) for g in REBUILD_GENERATORS]
        if not cells[0]:
            continue
        cols = [str(count)] + [value(c) for c in cells]
        cols += [versus(cells[0], row("rebuild", "code", count, g)) for g in rebuild_ratios]
        lines.append(md(cols))
    lines.append("")
    lines.append(
        "Astro is not measured here: its rebuild empties the output directory "
        "before rendering, and that deletion dominates the run on this "
        "filesystem (36 s against a 4.8 s cold build at 1,000 posts).\n"
    )

    lines.append("## Peak RSS, cold builds (median MB)\n")
    lines += table_head(["Posts", "Profile"], GENERATORS, [])
    for profile in PROFILES:
        for count in SIZES:
            cells = [row("cold", profile, count, g) for g in GENERATORS]
            if not cells[0]:
                continue
            lines.append(
                md([str(count), profile] + [value(c, "rss_mb") for c in cells])
            )
    lines.append("")

    tiny = [row("tiny", "code", 2, g) for g in GENERATORS]
    if any(tiny):
        lines.append("## Tiny site (2 posts)\n")
        lines.append("| Generator | median ms | RSS MB |")
        lines.append("| --- | ---: | ---: |")
        for record in tiny:
            if record:
                lines.append(
                    f"| {record['generator']} | {fmt(record['median_ms'])} | {fmt(record['rss_mb'])} |"
                )
        lines.append("")

    scale = [row("scale", "code", SCALE_COUNT, g) for g in SCALE_GENERATORS]
    if any(scale):
        lines.append(f"## Scaling: {SCALE_COUNT} posts (median ms)\n")
        lines.append("| Generator | median ms | RSS MB | pages/s |")
        lines.append("| --- | ---: | ---: | ---: |")
        for record in scale:
            if record:
                lines.append(
                    f"| {record['generator']} | {fmt(record['median_ms'])} | "
                    f"{fmt(record['rss_mb'])} | {record['count'] / (record['median_ms'] / 1000):.0f} |"
                )
        lines.append("")

    starts = [row("startup", "-", 0, g) for g in GENERATORS]
    if any(starts):
        lines.append("## CLI startup (median ms)\n")
        lines.append("| Generator | median ms |")
        lines.append("| --- | ---: |")
        for record in starts:
            if record:
                lines.append(f"| {record['generator']} | {fmt(record['median_ms'])} |")
        lines.append("")

    lines.append("## Footprint\n")
    lines.append("| Item | Size |")
    lines.append("| --- | ---: |")
    lines.append(f"| agno binary | {base.AGNO.stat().st_size / 1_000_000:.1f} MB |")
    if base.HUGO.exists():
        lines.append(f"| Hugo binary | {base.HUGO.stat().st_size / 1_000_000:.1f} MB |")
    if base.ZOLA.exists():
        lines.append(f"| Zola binary | {base.ZOLA.stat().st_size / 1_000_000:.1f} MB |")
    node_modules = base.WORK / "astro" / "node_modules"
    if node_modules.exists():
        lines.append(f"| Astro node_modules | {dir_size(node_modules) / 1_000_000:.0f} MB |")
    for count in (100, 1000, 3000):
        for name in GENERATORS:
            record = row("cold", "code", count, name)
            if record:
                lines.append(
                    f"| {name} output, {count} posts | {record['output_mb']} MB, "
                    f"{record['html_files']} HTML files |"
                )
    lines.append("")

    (ROOT / "REPORT.md").write_text("\n".join(lines) + "\n")
    print(f"\nwrote {ROOT / 'REPORT.md'}")


def platform_release():
    try:
        return subprocess.run(
            ["uname", "-sr"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except Exception:
        return sys.platform


def version_of(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
        return out.strip().splitlines()[0]
    except Exception:
        return "unknown"


def astro_version():
    package = base.WORK / "astro" / "node_modules" / "astro" / "package.json"
    try:
        return json.loads(package.read_text())["version"]
    except Exception:
        return "unknown"


def zola_version():
    if not base.ZOLA.exists():
        return "not installed"
    return version_of([str(base.ZOLA), "--version"])


def main():
    base.WORK.mkdir(exist_ok=True)
    records = []
    for profile in PROFILES:
        for count in SIZES:
            print(f"\n== cold: profile={profile} posts={count}", flush=True)
            sites = setup_all(count, profile)
            records.extend(cold_builds(sites, count, profile))
            if profile == "code" and count in SIZES:
                print(f"== rebuild: posts={count}", flush=True)
                records.extend(rebuilds(sites, count, profile))

    print("\n== tiny site", flush=True)
    tiny_site(records)
    print("\n== scaling", flush=True)
    scale_test(records)
    print("\n== startup", flush=True)
    startup(records)

    (ROOT / "results.json").write_text(json.dumps(records, indent=2) + "\n")
    write_report(records)


if __name__ == "__main__":
    main()
