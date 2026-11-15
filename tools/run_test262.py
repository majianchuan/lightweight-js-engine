import argparse
import concurrent.futures
import os
import subprocess
import sys
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "test262.exe")
TEST_ROOT = os.path.join(ROOT, "test262", "test")


def run_one(path, timeout):
    rel = os.path.relpath(path, TEST_ROOT).replace(os.sep, "/")
    try:
        proc = subprocess.run(
            [EXE, "--single", path],
            capture_output=True,
            timeout=timeout,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except subprocess.TimeoutExpired:
        return rel, "FAIL", "timeout"
    out = (proc.stdout or "").strip().splitlines()
    line = out[-1] if out else ""
    if line == "PASS":
        return rel, "PASS", ""
    if line == "SKIP":
        return rel, "SKIP", ""
    if line.startswith("FAIL\t"):
        return rel, "FAIL", line[5:]
    return rel, "FAIL", (line or f"exit {proc.returncode}")[:300]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", default="")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=12)
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("--failures", default="test262-failures.txt")
    args = ap.parse_args()

    paths = []
    for dirpath, _dirnames, filenames in os.walk(TEST_ROOT):
        for name in sorted(filenames):
            if name.endswith(".js"):
                p = os.path.join(dirpath, name)
                if args.filter and args.filter not in p.replace(os.sep, "/"):
                    continue
                paths.append(p)
    paths.sort()
    if args.limit:
        paths = paths[: args.limit]
    total = len(paths)
    print(f"running {total} tests with {args.workers} workers", flush=True)

    counts = {"PASS": 0, "FAIL": 0, "SKIP": 0}
    by_dir = {}
    failures = []
    done = 0
    started = time.time()
    lock = threading.Lock()

    def task(p):
        return run_one(p, args.timeout)

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as ex:
        for rel, status, msg in ex.map(task, paths):
            with lock:
                done += 1
                counts[status] += 1
                top = rel.split("/")[0]
                c = by_dir.setdefault(top, [0, 0, 0])
                c[{"PASS": 0, "FAIL": 1, "SKIP": 2}[status]] += 1
                if status == "FAIL":
                    failures.append((rel, msg))
                if done % 1000 == 0:
                    elapsed = time.time() - started
                    print(
                        f"progress {done}/{total} pass={counts['PASS']} fail={counts['FAIL']} skip={counts['SKIP']} ({elapsed:.0f}s)",
                        flush=True,
                    )
    elapsed = time.time() - started
    ran = counts["PASS"] + counts["FAIL"]
    rate = counts["PASS"] * 100.0 / ran if ran else 0.0
    print(
        f"TOTAL {total} | PASS {counts['PASS']} | FAIL {counts['FAIL']} | SKIP {counts['SKIP']} | PASS_RATE {rate:.2f}% | {elapsed:.0f}s"
    )
    for top in sorted(by_dir):
        p, f, s = by_dir[top]
        r = p * 100.0 / (p + f) if p + f else 0.0
        print(f"  {top:<25} pass={p:<7} fail={f:<7} skip={s:<5} rate={r:.1f}%")
    with open(os.path.join(ROOT, args.failures), "w", encoding="utf-8") as fh:
        for rel, msg in failures:
            fh.write(f"{rel}\t{msg}\n")
    print(f"failures written to {args.failures}")


if __name__ == "__main__":
    main()
