#!/usr/bin/env python3
"""Run ecraven's R7RS benchmarks in scm, the way the suite's own `bench`
script would: prelude + program + common.scm + postlude into one file, the
program's input on stdin, one row per program.

    scm-conform/bench/run.py --fetch                  # clone into target/r7rs-benchmarks
    cargo build --release
    scm-conform/bench/run.py [--small] [timeout-seconds] [names...]

`--small` uses `inputs-small/` -- the reduced inputs the cargo test gate
uses -- and a run directory with `bib` cut to 400 KB, the same as the gate.
The suite is found at $R7RS_BENCHMARKS or target/r7rs-benchmarks. Results so
far are in docs/conformance.md.
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


def run(suite, out_dir, timeout, name, inputs, cwd):
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
        with open(os.path.join(inputs, f"{name}.input")) as stdin:
            p = subprocess.run([SCM, program], stdin=stdin, capture_output=True,
                               text=True, timeout=timeout, cwd=cwd)
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


def find_suite():
    for p in [os.environ.get("R7RS_BENCHMARKS"), os.path.join(ROOT, "target", "r7rs-benchmarks")]:
        if p and os.path.isfile(os.path.join(p, "src", "common.scm")):
            return p
    sys.exit("r7rs-benchmarks not found: run with --fetch, or set R7RS_BENCHMARKS")


def small_run_dir(suite):
    """inputs/ as the programs expect it, with `bib` cut to 400 KB."""
    import shutil
    d = os.path.join(ROOT, "target", "bench-small")
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(os.path.join(d, "inputs"))
    os.makedirs(os.path.join(d, "outputs"))
    for f in os.listdir(os.path.join(suite, "inputs")):
        src = os.path.join(suite, "inputs", f)
        if f == "bib":
            open(os.path.join(d, "inputs", f), "wb").write(open(src, "rb").read()[:400_000])
        else:
            os.symlink(src, os.path.join(d, "inputs", f))
    return d


def main():
    args = sys.argv[1:]
    if "--fetch" in args:
        dest = os.path.join(ROOT, "target", "r7rs-benchmarks")
        if not os.path.isdir(dest):
            subprocess.run(["git", "clone", "--depth", "1",
                            "https://github.com/ecraven/r7rs-benchmarks.git", dest], check=True)
        args.remove("--fetch")
        if not args:
            return
    small = "--small" in args
    if small:
        args.remove("--small")
    suite = find_suite()
    timeout = float(args[0]) if args else 120
    names = args[1:] or ALL
    out_dir = os.path.join(ROOT, "target", "bench")
    os.makedirs(out_dir, exist_ok=True)
    inputs = os.path.join(HERE, "inputs-small") if small else os.path.join(suite, "inputs")
    cwd = small_run_dir(suite) if small else suite
    if small:
        names = [n for n in names if os.path.isfile(os.path.join(inputs, f"{n}.input"))]
    with cf.ThreadPoolExecutor(6) as ex:
        rows = list(ex.map(lambda n: run(suite, out_dir, timeout, n, inputs, cwd), names))
    counts = {}
    for name, status, took, msg in rows:
        counts[status] = counts.get(status, 0) + 1
        print(f"{name:12s} {status:9s} {took:>8s}  {msg}")
    print(counts)


if __name__ == "__main__":
    main()
