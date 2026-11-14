# scm against Chibi, Guile, Steel and Racket

2026-11-13, Apple M4 Pro, macOS. scm T1 at `b740171`, Chibi-Scheme 0.12,
Guile 3.0.11, Steel 0.8.2, Racket CS 9.1. `scm-conform/bench/vs-others/run.py`;
the nine programs beside it are `vs-steel`'s, scaled 5-20× (`fib` 35
instead of 30, `lists` five times the work, and so on) so that a native
compiler's run is not lost inside its own startup.

[vs-steel.md](vs-steel.md) races one other interpreter. This asks where scm
sits among the Schemes people actually reach for: the embeddable R7RS VM
(Chibi), the embeddable VM with an optimising compiler and a JIT (Guile),
the other Rust Scheme (Steel), and a native compiler for scale (Racket CS,
which is Chez underneath).

## How it is timed

**Inside the program.** Each program reads a wall clock before its body
and after it and prints the difference to stderr, so startup, library
loading and compilation are outside every number. This is the only way a
precompiled Racket module (`raco make`) and an auto-compiled Guile script
(compiled on an untimed first run, cached in `.go`) compare fairly with
interpreters that compile as they read. scm's 8 ms boot against Racket's 49
ms is real and matters to a host, but it is not what this measures.

Best of three, runs interleaved across all five so load drift lands on
everyone. Load average 0.9 at the start. Every implementation printed the
same output as scm on every program (Steel's `#true` normalised to `#t`).
Racket's copy of `vec` writes `when` for the one-armed `if`, which
`racket/base` rejects; no other source differs.

| program | scm T1 | Chibi | Guile | Steel | Racket CS |
|---|---|---|---|---|---|
| `fib` 35 | 920 ms | 434 ms | 129 ms | 1473 ms | 25 ms |
| `tak` 24 16 8, ×20 | 1870 ms | 1065 ms | 189 ms | 2274 ms | 42 ms |
| `loop`, 50M with `modulo` | 1998 ms | 3819 ms | 135 ms | 3341 ms | 152 ms |
| `vec`, fill and sum 10M | 718 ms | 357 ms | 47 ms | 1540 ms | 36 ms |
| `strings`, 60k appends and a scan | 226 ms | 3213 ms | 399 ms | 231 ms | 128 ms |
| `closures`, 20M created and called | 2415 ms | 1421 ms | 303 ms | 2421 ms | 16 ms |
| `lists`, `vs-steel`'s ×5 | 5901 ms | 7408 ms | 268 ms | 8831 ms | 136 ms |
| `fbm`, the texel loop, 512² | 903 ms | 2348 ms | 539 ms | 1372 ms | 149 ms |
| `callcc`, 5M escapes | 1235 ms | 2159 ms | 4241 ms | 2587 ms | 109 ms |
| **geometric mean, time / scm** | **1.00×** | **1.32×** | **0.22×** | **1.48×** | **0.05×** |
| excluding `callcc` | 1.00× | 1.28× | 0.16× | 1.42× | 0.05× |

## Reading it

**Racket ≫ Guile ≫ scm > Chibi > Steel.** Among interpreters scm is first;
against anything that compiles to machine code it is a long way behind.

**Steel, 1.4-1.5× slower than scm.** The reversal recorded in vs-steel.md
holds on the larger inputs: Steel was slightly ahead before S14 and is now
behind on every program but `strings`, where the two tie.

**Chibi, 1.3× slower, and the like-for-like comparison** -- a small
embeddable R7RS-small VM in C. It is not uniformly behind:

- *Chibi wins on calls*: `fib`, `tak`, `closures` and `vec` are 1.7-2.1×
  faster there. Its call path is cheaper than scm's, which is the same weak
  spot `lists` exposed against Steel.
- *scm wins on arithmetic, floats and strings*: `loop` 1.9×, `fbm` 2.6×,
  `strings` 14×. Chibi boxes every flonum, and its string building is
  slow enough that `strings` is its worst row by far.

**Guile, 4.5-6× faster.** Not a fair fight for an interpreter, and the
next section says why. `lists` is 22×: Guile's `assv` is C, its pairs come
from thread-local free lists, and a call costs very little once
JIT-compiled, where scm builds a frame per call and runs `assv` as
interpreted Scheme.

**Racket CS, ~20× faster**, as a native compiler should be. `closures` at
150× is mostly Chez inlining `adder` and never allocating the closure; read
that row as an optimiser demonstration, not a call cost.

**`callcc` favours scm and should be discounted**, as in vs-steel.md. scm's
continuations are escape-only, so its `call/cc` is a mark and an unwind;
Chibi, Guile and Steel pay for full re-entrant continuations they never
use here. Guile's is particularly expensive (4.2 s).

## Why Guile is so far ahead

Guile 3 compiles to bytecode too, but three layers sit on top of it, and
the question is which of them buys the gap. The first is an optimising
compiler in front of the bytecode -- a CPS intermediate form with inlining,
contification, type inference and unboxing of floats and 64-bit integers.
The second is a register VM whose instructions are close to machine
operations once types are known. The third is a template JIT, on by
default since 3.0.

`GUILE_JIT_THRESHOLD=-1` switches the JIT off and leaves the first two.
Same harness, same three runs, at a load average of 5-6 (scm's column sits
about 5% above the quiet run, so the ratios are the part to read):

| program | scm T1 | Guile, no JIT | Guile, JIT | what the JIT bought |
|---|---|---|---|---|
| `fib` | 984 ms | 504 ms | 129 ms | 3.9× |
| `tak` | 1980 ms | 628 ms | 189 ms | 3.3× |
| `loop` | 2109 ms | 400 ms | 135 ms | 3.0× |
| `vec` | 747 ms | 216 ms | 47 ms | 4.6× |
| `strings` | 228 ms | 407 ms | 399 ms | 1.0× |
| `closures` | 2531 ms | 676 ms | 303 ms | 2.2× |
| `lists` | 6070 ms | 814 ms | 268 ms | 3.0× |
| `fbm` | 926 ms | 728 ms | 539 ms | 1.4× |
| `callcc` | 1251 ms | 4636 ms | 4241 ms | 1.1× |
| **geometric mean, time / scm, excluding `callcc`** | 1.00× | **0.38×** | **0.16×** | |

(The JIT column is the quiet run above.)

**Roughly half the gap is the JIT, half is everything before it.** Even
interpreted, Guile is 2.6× scm -- that is the optimiser and the low-level
instruction set, on programs where type inference can strip most checks.
The JIT then buys another 2.4×, almost all of it on call- and loop-heavy
code; it buys nothing on `strings` and little on `fbm`, which spend their
time in C primitives. That is the same split scm sees: `strings` and `fbm`
are where scm is closest to everyone, because there scm is also running
Rust.

What this says about scm's own options, without deciding anything:

- The JIT half is what T2 was built to be. Since S14, T2 declines the new
  flat-frame ops and runs these programs as T1 (vs-steel.md, "After S14");
  S14f, teaching T2 those ops, is the step that would put a jit column back
  in this table.
- The interpreter half is the call protocol and boxed arithmetic, both
  already identified in vs-steel.md and s34-list-primitives.md. Guile
  shows the headroom is at least 2.6× without any native code.
- `docs/plan.md`'s position stands: ordinary-code speed was never the
  point, and the leaf compiler (199× T1 on `fbm`) is the answer for the
  hot paths ray actually has. This table locates scm; it does not reopen
  the speed campaign, which S34 stopped deliberately.

## Reproduction

    cargo build --release -p scm-cli
    export CHIBI=$(nix build --no-link --print-out-paths nixpkgs#chibi)/bin/chibi-scheme
    export GUILE=$(nix build --no-link --print-out-paths nixpkgs#guile)/bin/guile
    export RACKET=$(nix build --no-link --print-out-paths nixpkgs#racket)/bin/racket
    scm-conform/bench/vs-others/run.py 3                  # all five
    GUILE_JIT_THRESHOLD=-1 scm-conform/bench/vs-others/run.py 3 guile

`steel` is taken from PATH (or `$STEEL`). Naming implementations after the
run count limits the table to scm and those. The binary measured here was
built `--features jit`; that changes nothing for T1 except startup, which
this harness does not count.
