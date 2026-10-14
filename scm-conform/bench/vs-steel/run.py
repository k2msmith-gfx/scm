#!/usr/bin/env python3
"""scm against Steel on ten programs in the subset both run.

    cargo build --release -p scm-cli --features jit   # T2 column; or without, for T1 only
    scm-conform/bench/vs-steel/run.py                  # needs `steel` on PATH

Best of three, user+system time (wall for the startup case). Steel 0.8.2
was the Rust Scheme the plan measured before scm existed (plan §7: 109 ms
to run an empty file); docs/vs-steel.md has the numbers and the reading.
"""
import os, resource, statistics, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
SCM = os.path.join(HERE, "..", "..", "..", "target", "release", "scm")
PROGS = ["startup", "fib", "tak", "loop", "vec", "strings", "closures", "lists", "fbm", "callcc"]


def best(cmd, env=None, runs=3):
    b = 1e9
    for _ in range(runs):
        r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
        t = time.perf_counter()
        subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, env=env, cwd=HERE)
        wall = time.perf_counter() - t
        r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
        user = (r1.ru_utime - r0.ru_utime) + (r1.ru_stime - r0.ru_stime)
        b = min(b, wall if wall < 0.05 else user)
    return b


def main():
    tiers = sys.argv[1:] or ["t1", "t2"]
    rows = []
    print("| program | Steel | " + " | ".join(t.upper() for t in tiers) + " | " + " | ".join(f"Steel / {t.upper()}" for t in tiers) + " |")
    print("|---|---|" + "---|" * (2 * len(tiers)))
    for p in PROGS:
        f = p + ".scm"
        s = best(["steel", f])
        ts = [best([SCM, f], dict(os.environ, SCM_TIER=t)) for t in tiers]
        rows.append((p, s, ts))
        print(f"| `{p}` | {s*1000:.0f} ms | " + " | ".join(f"{t*1000:.0f} ms" for t in ts) + " | " + " | ".join(f"{s/t:.1f}×" for t in ts) + " |", flush=True)
    for i, t in enumerate(tiers):
        gm = statistics.geometric_mean([s / ts[i] for p, s, ts in rows if p != "startup"])
        print(f"geometric mean of Steel / {t.upper()}, excluding startup: {gm:.2f}×")


if __name__ == "__main__":
    main()
