#!/usr/bin/env python3
"""Compare two agno binaries on the same site, with a noise control built in.

The suite measures one generator at a time, so a machine that speeds up or
slows down during the matrix favours whichever generator was measured in the
quiet window. This tool does the opposite: it interleaves the two binaries
round-robin on one site, and it adds a third series that runs binary A again.

    A, B, A2, A, B, A2, ...

A2 is the control. Its ratio to A is a direct measurement of how much this
machine moves between two identical runs, so the B/A ratio can be read against
it instead of against zero. A difference smaller than the control is not a
result.

Usage:
    python3 bench/ab.py --a old/agno --b new/agno --count 1000 --profile code
    python3 bench/ab.py --a agno --b agno            # calibrate: noise only
    python3 bench/ab.py --a agno --b agno --threads 1

Run it on an idle machine. It cannot fix a noisy machine, it can only tell you
that the machine is noisy.
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
sys.path.insert(0, str(ROOT))
import run as harness  # noqa: E402  (fixture setup lives there)

SERIES = ("a", "b", "control")


def version(binary: Path) -> str:
    proc = subprocess.run(
        [str(binary), "--version"], capture_output=True, text=True, check=False
    )
    return (proc.stdout or proc.stderr).strip() or "unknown"


def build_once(binary: Path, site: Path, dist: Path, env) -> tuple[float, float]:
    """Delete the output, build, return (wall ms, peak RSS MB)."""
    shutil.rmtree(dist, ignore_errors=True)
    started = time.perf_counter()
    proc = subprocess.Popen(
        [str(binary), "build"],
        cwd=site,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    _, status, usage = os.wait4(proc.pid, 0)
    elapsed = (time.perf_counter() - started) * 1000
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError(f"{binary} build failed ({status})")
    return elapsed, usage.ru_maxrss / 1024


def median_ratio(samples: dict[str, list[float]], numerator: str) -> float:
    """Median of the per-round ratios, so a single spike cannot carry it."""
    base = samples["a"]
    return statistics.median(
        [n / a for n, a in zip(samples[numerator], base) if a > 0]
    )


def min_ratio(samples: dict[str, list[float]], numerator: str) -> float:
    """Ratio of the fastest run of each series.

    Cold-build timings are right-skewed: interference can only add time, never
    remove it, so the fastest sample is the one least polluted by the machine.
    The median is the honest central tendency, the min is the honest floor, and
    a real effect has to show up in both.
    """
    return min(samples[numerator]) / min(samples["a"])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--a", required=True, help="baseline binary")
    parser.add_argument("--b", required=True, help="candidate binary")
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    parser.add_argument(
        "--runs",
        type=int,
        default=9,
        help="rounds; each round builds all three series (default 9)",
    )
    parser.add_argument(
        "--threads",
        type=int,
        default=None,
        help="sets RAYON_NUM_THREADS, for scaling checks",
    )
    args = parser.parse_args()

    binary_a = Path(args.a).resolve()
    binary_b = Path(args.b).resolve()
    for binary in (binary_a, binary_b):
        if not binary.exists():
            sys.exit(f"missing binary: {binary}")
    if args.runs < 3:
        sys.exit("need at least 3 rounds for the control to mean anything")

    same = binary_a == binary_b
    version_a, version_b = version(binary_a), version(binary_b)

    harness.WORK.mkdir(exist_ok=True)
    site = harness.setup_agno(args.count, args.profile)
    dist = site / "dist"

    env = dict(os.environ)
    if args.threads is not None:
        env["RAYON_NUM_THREADS"] = str(args.threads)

    paths = {"a": binary_a, "b": binary_b, "control": binary_a}
    for series in SERIES:  # warm the page cache before any timing
        build_once(paths[series], site, dist, env)

    times: dict[str, list[float]] = {s: [] for s in SERIES}
    rss: dict[str, list[float]] = {s: [] for s in SERIES}
    for round_index in range(args.runs):
        # Rotate the slot each round: with three series and three orders, the
        # first-build advantage is spread evenly instead of landing on one.
        offset = round_index % len(SERIES)
        order = SERIES[offset:] + SERIES[:offset]
        for series in order:
            elapsed, peak = build_once(paths[series], site, dist, env)
            times[series].append(elapsed)
            rss[series].append(peak)

    ratio_b = median_ratio(times, "b")
    ratio_control = median_ratio(times, "control")
    min_b = min_ratio(times, "b")
    min_control = min_ratio(times, "control")

    # The control is two series of the same binary, so its deviation from
    # parity is the largest error this machine produced for identical work.
    # A candidate has to beat that on both statistics before it counts.
    floor = max(abs(ratio_control - 1), abs(min_control - 1))
    effect_median = abs(ratio_b - 1)
    effect_min = abs(min_b - 1)
    distinguishable = effect_median > floor and effect_min > floor

    print(f"\n{args.count} posts ({args.profile}), {args.runs} rounds")
    if args.threads is not None:
        print(f"RAYON_NUM_THREADS={args.threads}")
    print(f"  A       {version_a}  ({binary_a})")
    print(f"  B       {version_b}  ({binary_b})")
    if same:
        print("  note: A and B are the same binary — this is a calibration run,")
        print("        so any difference below is this machine's noise floor.")

    print(f"\n{'round':>5} {'A ms':>9} {'B ms':>9} {'A2 ms':>9} {'B/A':>7} {'A2/A':>7}")
    for i in range(args.runs):
        print(
            f"{i + 1:5} {times['a'][i]:9.1f} {times['b'][i]:9.1f} "
            f"{times['control'][i]:9.1f} {times['b'][i] / times['a'][i]:7.3f} "
            f"{times['control'][i] / times['a'][i]:7.3f}"
        )

    med = {s: statistics.median(times[s]) for s in SERIES}
    print(f"\n{'':10} {'median ms':>10} {'min ms':>9} {'rss MB':>8}")
    for series, label in (("a", "A"), ("b", "B"), ("control", "A2 (control)")):
        print(
            f"{label:10} {med[series]:10.1f} {min(times[series]):9.1f} "
            f"{statistics.median(rss[series]):8.1f}"
        )

    print(f"\n{'':12} {'B/A':>7} {'control':>9}   gap vs control")
    for label, ratio, control in (
        ("median", ratio_b, ratio_control),
        ("min of N", min_b, min_control),
    ):
        print(
            f"{label:12} {ratio:7.3f} {control:9.3f}   "
            f"{abs(ratio - 1) * 100:.1f}% vs {abs(control - 1) * 100:.1f}%"
        )
    print(
        f"\nnoise floor: {floor * 100:.1f}% — this run can only resolve effects "
        "larger than that."
    )

    if same:
        print(
            "\nverdict: calibration only. Both sides are the same file, so "
            f"anything up to {floor * 100:.1f}%\n         here is the machine, "
            "not the binary."
        )
    elif not distinguishable:
        print(
            "\nverdict: not distinguishable. The gap between A and B is no "
            "larger than the\n         gap between A and itself on this "
            "machine; re-run on an idle box before\n         believing it."
        )
    else:
        direction = "faster" if ratio_b < 1 else "slower"
        factor = max(ratio_b, 1 / ratio_b)
        print(
            f"\nverdict: B is {factor:.2f}x {direction} than A on both the "
            f"median and the fastest\n         run, which is outside the "
            f"{floor * 100:.1f}% noise floor."
        )


if __name__ == "__main__":
    main()
