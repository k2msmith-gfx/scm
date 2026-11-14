#!/usr/bin/env python3
"""scm T1 against Chibi, Guile, Steel and Racket CS on the vs-steel programs,
scaled up so a native compiler's run is not lost in its own startup.

    cargo build --release -p scm-cli
    scm-conform/bench/vs-others/run.py [runs] [impl...]

Each program times itself: a wall clock read before the body and after it,
printed to stderr. Startup, library loading and compilation are therefore
outside the number, which is what makes a precompiled Racket module and an
auto-compiled Guile script comparable with an interpreter. Best of `runs`
(default 3), runs interleaved across implementations so load drift lands on
all of them. Every implementation must print the same stdout as scm.

The other implementations are found on PATH, or by $CHIBI, $GUILE, $STEEL,
$RACKET (and $RACO beside it). With nix:

    export CHIBI=$(nix build --no-link --print-out-paths nixpkgs#chibi)/bin/chibi-scheme
    export GUILE=$(nix build --no-link --print-out-paths nixpkgs#guile)/bin/guile
    export RACKET=$(nix build --no-link --print-out-paths nixpkgs#racket)/bin/racket

docs/vs-others.md has the numbers and the reading.
"""
import os, shutil, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCM = os.path.join(HERE, "..", "..", "..", "target", "release", "scm")
PROGS = ["fib", "tak", "loop", "vec", "strings", "closures", "lists", "fbm", "callcc"]
END = "\n(display (- (%ms) %t0) (current-error-port))\n"


def find(var, name):
    path = os.environ.get(var) or shutil.which(name)
    if not path:
        sys.exit(f"{name} not found: put it on PATH or set ${var}")
    return path


def impls(work):
    # name: (file extension, clock prelude, argv prefix, extra environment)
    return {
        "scm": (".scm", "(import (scheme time))\n"
                "(define (%ms) (/ (current-jiffy) 1000.0))\n", [SCM], {}),
        "chibi": (".scm", "(import (scheme base) (scheme write) (scheme inexact) (scheme time))\n"
                  "(define (%ms) (/ (current-jiffy) (/ (jiffies-per-second) 1000.0)))\n",
                  [find("CHIBI", "chibi-scheme")], {}),
        # Guile auto-compiles a script on first run; the cache lives in `work`.
        "guile": (".scm", "(define (%ms) (/ (get-internal-real-time) (/ internal-time-units-per-second 1000.0)))\n",
                  [find("GUILE", "guile")], {"XDG_CACHE_HOME": os.path.join(work, "cache")}),
        "steel": (".scm", "(require-builtin steel/time)\n"
                  "(define (%ms) (current-inexact-milliseconds))\n", [find("STEEL", "steel")], {}),
        # A `#lang racket/base` module, compiled with raco make before timing.
        "racket": (".rkt", "#lang racket/base\n"
                   "(define (%ms) (current-inexact-milliseconds))\n", [find("RACKET", "racket")], {}),
    }


# Racket's `if` must have an else branch.
FIX = {("racket", "vec"): ("(if (< i n) (begin", "(when (< i n) (begin")}


def run(cmd, env):
    p = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                       env=dict(os.environ, **env), timeout=300)
    try:
        ms = float(p.stderr.strip().split()[-1])
    except (IndexError, ValueError):
        return float("nan"), "ERROR " + p.stderr[-300:]
    return ms, p.stdout.strip().replace("#true", "#t").replace("#false", "#f")


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    work = tempfile.mkdtemp(prefix="scm-vs-others-")
    table = impls(work)
    if sys.argv[2:]:
        table = {n: table[n] for n in ["scm"] + [a for a in sys.argv[2:] if a != "scm"]}
    names = list(table)
    raco = os.environ.get("RACO") or os.path.join(os.path.dirname(table["racket"][2][0]), "raco") \
        if "racket" in table else None
    res = {}
    print("| program | " + " | ".join(names) + " |\n|---|" + "---|" * len(names), flush=True)
    for p in PROGS:
        body = open(os.path.join(HERE, p + ".scm")).read()
        cmds = {}
        for n, (ext, head, argv, env) in table.items():
            b = body.replace(*FIX[(n, p)]) if (n, p) in FIX else body
            f = os.path.join(work, f"{n}-{p}{ext}")
            with open(f, "w") as fh:
                fh.write(head + "(define %t0 (%ms))\n" + b + END)
            cmds[n] = (argv + [f], env)
        if raco:
            subprocess.run([raco, "make", cmds["racket"][0][-1]], check=True)
        outs = {n: run(*c)[1] for n, c in cmds.items()}  # also fills Guile's compile cache
        times = {n: [] for n in names}
        for _ in range(runs):
            for n, c in cmds.items():
                times[n].append(run(*c)[0])
        res[p] = {n: min(v) for n, v in times.items()}
        bad = [n for n in names if outs[n] != outs["scm"]]
        note = f"  DISAGREE: {[(n, outs[n]) for n in bad]} against scm's {outs['scm']!r}" if bad else ""
        print(f"| `{p}` | " + " | ".join(f"{res[p][n]:.0f} ms" for n in names) + " |" + note, flush=True)
    g = statistics.geometric_mean
    print("\ngeometric mean of time / scm T1 (above 1 is slower than scm):")
    for n in names:
        a = g([res[p][n] / res[p]["scm"] for p in PROGS])
        b = g([res[p][n] / res[p]["scm"] for p in PROGS if p != "callcc"])
        print(f"  {n}: {a:.2f}x, excluding callcc {b:.2f}x")
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
