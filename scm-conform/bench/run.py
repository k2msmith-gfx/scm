#!/usr/bin/env python3
"""Run ecraven's R7RS benchmarks in scm, the way the suite's own `bench`
script would: prelude + program + common.scm + postlude into one file, the
program's input on stdin, one row per program.

    git clone https://github.com/ecraven/r7rs-benchmarks.git /path/to/suite
    cargo build --release
    scm-conform/bench/run.py /path/to/suite [timeout-seconds] [names...]

The suite is cloned, not vendored: 7 MB of inputs against the conformance
suite's one file. Results so far are in docs/conformance.md.
"""
import concurrent.futures as cf
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SCM = os.path.join(ROOT, "target", "release", "scm")
ALL = ("browse deriv destruc diviter divrec puzzle triangl tak takl ntakl cpstak ctak "
       "fib fibc fibfp sum sumfp fft mbrot mbrotZ nucleic pi pnpoly ray simplex "
       "ack array1 string sum1 cat tail wc read1 "
       "compiler conform dynamic earley graphs lattice matrix maze mazefun nqueens "
       "paraffins parsing peval primes quicksort scheme slatex chudnovsky "
       "nboyer sboyer gcbench mperm equal bv2string").split()


def run(suite, out_dir, timeout, name):
    parts = [os.path.join(HERE, "scm-prelude.scm"),
             os.path.join(suite, "src", f"{name}.scm"),
             os.path.join(suite, "src", "common.scm"),
             os.path.join(suite, "src", "common-postlude.scm")]
    program = os.path.join(out_dir, f"{name}.scm")
    with open(program, "w") as f:
        for p in parts:
            f.write(open(p).read())
    t0 = time.time()
    try:
        with open(os.path.join(suite, "inputs", f"{name}.input")) as stdin:
            p = subprocess.run([SCM, program], stdin=stdin, capture_output=True,
                               text=True, timeout=timeout, cwd=suite)
    except subprocess.TimeoutExpired:
        return name, "TIMEOUT", f">{timeout:.0f}s", ""
    elapsed = time.time() - t0
    csv = [l for l in p.stdout.splitlines() if l.startswith("+!CSVLINE!+")]
    if csv and "INCORRECT" not in csv[0]:
        return name, "ok", f"{float(csv[0].split(',')[-1]):.1f}s", ""
    if csv:
        return name, "INCORRECT", f"{elapsed:.1f}s", ""
    err = p.stderr.strip().splitlines()
    return name, "ERROR", f"{elapsed:.1f}s", (err[0] if err else "")[:100]


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    suite = sys.argv[1]
    timeout = float(sys.argv[2]) if len(sys.argv) > 2 else 120
    names = sys.argv[3:] or ALL
    out_dir = os.path.join(ROOT, "target", "bench")
    os.makedirs(out_dir, exist_ok=True)
    with cf.ThreadPoolExecutor(6) as ex:
        rows = list(ex.map(lambda n: run(suite, out_dir, timeout, n), names))
    counts = {}
    for name, status, took, msg in rows:
        counts[status] = counts.get(status, 0) + 1
        print(f"{name:12s} {status:9s} {took:>8s}  {msg}")
    print(counts)


if __name__ == "__main__":
    main()
