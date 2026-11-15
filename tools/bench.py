import argparse
import json
import os
import statistics
import subprocess
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BENCH = os.path.join(ROOT, "bench")
DEFAULT_BIN = os.path.join(ROOT, "target", "release", "ljs.exe")
if not os.path.exists(DEFAULT_BIN):
    DEFAULT_BIN = os.path.join(ROOT, "target", "release", "ljs")


def run(exe, script, timeout):
    start = time.perf_counter()
    p = subprocess.run(
        [exe, script],
        capture_output=True,
        text=True,
        timeout=timeout,
        encoding="utf-8",
        errors="replace",
    )
    elapsed = time.perf_counter() - start
    if p.returncode != 0:
        raise RuntimeError(f"{os.path.basename(script)} failed: {p.stderr.strip()[:200]}")
    return elapsed, p.stdout.strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--exe", default=DEFAULT_BIN)
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--json", default="")
    args = ap.parse_args()
    scripts = sorted(f for f in os.listdir(BENCH) if f.endswith(".js"))
    rows = []
    for name in scripts:
        times = []
        out = ""
        for _ in range(args.runs):
            t, out = run(args.exe, os.path.join(BENCH, name), args.timeout)
            times.append(t)
        rows.append({"benchmark": name[:-3], "runs": times, "median_s": statistics.median(times), "output": out})
        print(f"{name[:-3]:<12} median={rows[-1]['median_s']:.4f}s  runs={['%.4f' % t for t in times]}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump({"exe": args.exe, "runs": args.runs, "results": rows}, fh, indent=2)
    print("\n| Benchmark | Median (s) |")
    print("|---|---:|")
    for r in rows:
        print(f"| {r['benchmark']} | {r['median_s']:.4f} |")


if __name__ == "__main__":
    main()
