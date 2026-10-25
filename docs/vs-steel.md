# scm against Steel

2026-10-13, aarch64 macOS. Steel 0.8.2 (`steel` on PATH), scm at this
commit. `scm-conform/bench/vs-steel/run.py`; the ten programs are beside
it, written in the subset both implementations run: no `guard`, no
`define-record-type`, no `let-values` on Steel's side.

Steel is the other embeddable Scheme in Rust, and the one the plan measured
before scm existed: 109 ms to run an empty file, 61 ms of it `Engine::new`
(§7). It is a bytecode VM over reference-counted values; scm's T1 is a
bytecode VM over a tracing, non-moving collector, and T2 is T1's bytecode
compiled to native code that still returns to the loop at every call.

Best of three, user+system time, wall for startup:

| program | Steel | T1 | T2 | Steel / T1 | Steel / T2 |
|---|---|---|---|---|---|
| `startup` | 55 ms | 7 ms | 7 ms | 7.5× | 7.8× |
| `fib` 30 | 196 ms | 261 ms | 197 ms | 0.8× | 1.0× |
| `tak` 24 16 8 | 178 ms | 246 ms | 178 ms | 0.7× | 1.0× |
| `loop`, 5M iterations with `modulo` | 403 ms | 513 ms | 310 ms | 0.8× | 1.3× |
| `vec`, fill and sum 1M | 219 ms | 218 ms | 142 ms | 1.0× | 1.5× |
| `strings`, append and scan | 86 ms | 104 ms | 125 ms | 0.8× | 0.7× |
| `closures`, 1M created and called | 180 ms | 230 ms | 210 ms | 0.8× | 0.9× |
| `lists`, map over 100k, 20 times, plus `assv` | 1870 ms | 3572 ms | 2942 ms | 0.5× | 0.6× |
| `fbm`, the texel loop, 128² | 147 ms | 135 ms | 169 ms | 1.1× | 0.9× |
| `callcc`, an escape per iteration | 211 ms | 82 ms | 79 ms | 2.6× | 2.7× |
| **geometric mean, excluding startup** | | | | **0.91×** | **1.06×** |

(The startup figures include the harness's own process spawn; `scm` alone
boots in 2–3 ms, Steel in 50.)

## Reading it

**The two are the same class.** T1 runs this set at 0.91× Steel's speed
and T2 at 1.06×. Neither is a compiler; both are interpreters of one kind or
another over boxed values, and the plan's own position is that scm's speed
for ordinary code was never the point -- the leaf compiler is, at 199× T1
on `fbm`.

**Where scm is ahead, and why.** Startup, 7× -- the §7 budget, kept.
Escape continuations, 2.6× -- the explicit frame stack makes one a mark
and an unwind. Float-heavy code, slightly.

**Where Steel is ahead, and why.** `lists` at 2×. Profiled, a third of
scm's time there is the collector: the program keeps 300,000 objects live
through each pass, and a non-generational mark-sweep re-marks them every
collection, where reference counting pays per operation instead. The rest
is the call protocol on 100,000-deep non-tail recursion. `strings` and
`closures` are the generic primitive-call path, which T2 does not help
because it is already a direct Rust call.

**What this exercise found in scm**, which is the other reason to run one:
`call/cc` the short name was missing; a backtrace blamed a call that had
already returned; and the collector pushed every element of a vector
through its worklist and collected again after twice the live *object*
count, which is 1 for a million-element vector -- `vec` went from 0.58 s
to 0.21 s once marking was amortised over allocations.

**What it does not measure.** Embedding ergonomics, which is the reason
either would be chosen: host functions, re-entrancy, rooting, errors as
`Result`, interrupt and fuel. Those are design properties, not timings, and
scm's are in `docs/plan.md` §7a.

## Reproduction, 2026-10-23

Same machine class (Apple M3 Pro), same Steel 0.8.2, fresh clone at
`05bd573`, run from `scm-conform/bench/vs-steel/run.py t1 t2`. The point
was to check the numbers above survive a clean checkout on a different day,
not to replace them: this run had a **load average near 3**, so absolute
times sit high across the board and the ratios are the part worth reading.

| program | Steel | T1 | T2 | Steel / T1 | Steel / T2 | T2 / T1 |
|---|---|---|---|---|---|---|
| `startup` | 60 ms | 9 ms | 9 ms | 6.8× | 7.1× | — |
| `fib` 30 | 226 ms | 276 ms | 192 ms | 0.8× | 1.2× | 1.44× |
| `tak` 24 16 8 | 203 ms | 266 ms | 180 ms | 0.8× | 1.1× | 1.48× |
| `loop`, 5M with `modulo` | 492 ms | 621 ms | 346 ms | 0.8× | 1.4× | 1.79× |
| `vec`, fill and sum 1M | 244 ms | 255 ms | 152 ms | 1.0× | 1.6× | 1.68× |
| `strings` | 92 ms | 118 ms | 127 ms | 0.8× | 0.7× | 0.93× |
| `closures`, 1M | 199 ms | 245 ms | 213 ms | 0.8× | 0.9× | 1.15× |
| `lists` | 2255 ms | 3471 ms | 2538 ms | 0.6× | 0.9× | 1.37× |
| `fbm`, 128² | 164 ms | 148 ms | 179 ms | 1.1× | 0.9× | 0.83× |
| `callcc` | 240 ms | 95 ms | 86 ms | 2.5× | 2.8× | 1.10× |
| **geometric mean, excluding startup** | | | | **0.93×** | **1.19×** | **1.28×** |

It holds. T1 at 0.93× against the 0.91× above, T2 at 1.19× against 1.06×,
and every per-program ratio within about 0.1× of its earlier value. The
reading in the section above stands unchanged: where T2 helps is arithmetic
and memory traffic, where it does not is `strings` and `fbm`, which are
already a direct Rust call through the generic primitive path.

**`callcc` is not a like-for-like row and should be discounted.** scm's
continuations are escape-only (§1 of `subset.md`), and the program only
ever escapes, so Steel pays for a generality it never uses — confirmed here
by storing a continuation, letting its extent return and invoking it: scm
raises `this continuation has escaped its extent`, Steel re-enters and
loops. Excluding that row as well as startup, T1 is 0.84× and T2 is 1.08×.
Those are the honest headline numbers for ordinary code.

**Two ways this benchmark can lie, both hit on this run.**

`SCM_TIER=t2` is read behind `#[cfg(feature = "jit")]`. In a binary built
without the feature it does not warn, it silently runs T1 — so a T2 column
can be a duplicate of the T1 column beside it. Check the feature, not the
variable.

Building with `--features jit` statically links LLVM and takes the binary
from about 5 MB to 61 MB, which moved startup from 4 ms to 9 ms for **both**
tiers. A T1 measured on the plain binary is therefore not comparable with a
T2 measured on the jit one, and startup is where the gap shows: 16.8× on
the plain binary against 6.8× here. Measure both tiers on the same binary;
the 6.8× is the one to quote.

Verified before timing, since a program that dies early times extremely
well: all ten agree between T1 and T2, and with Steel except `fbm`, where
Steel writes `#true` and scm writes `#t`.

## Where `lists` actually goes, and one thing done about it

2026-10-23. `lists` is the worst row above, and the reading beside it —
the collector re-marking 300,000 live objects — turns out to describe the
smaller half of the program. Split in two and timed separately under T1:

| phase | what it does | Steel | T1 | gap |
|---|---|---|---|---|
| A, `my-map` + `sum`, 20 passes | 100k-deep non-tail recursion, 2M pairs | 545 ms | 770 ms | 225 ms |
| B, `assv` over a 200-entry alist, 200k times | ~20M steps, **allocates nothing** | 1858 ms | 2241 ms | 383 ms |

Phase B is three quarters of the runtime and nearly two thirds of the gap,
and it never allocates a pair. **The collector cannot be the main cause.**
The GC reading stands for phase A; it was being read as the whole row.

**It is also not a missing primitive.** `assv`, `assq`, `memv` and `memq`
are interpreted Scheme in `prelude.scm`, so the obvious guess was that
Steel has a native one. Racing each implementation's library `assv`
against a hand-rolled loop *in its own language* kills that: Steel 1812 ms
library against 1908 ms hand-rolled, scm 2183 against 2173. Neither has a
native `assv`; both interpret the same loop. What is left is the per-step
cost of the interpreter.

Sampling T1 on phase B puts call machinery around 45%, opcode dispatch 35%,
the collector 9% -- on a program allocating no pairs, because every call
heap-allocates an environment -- and the primitives themselves about 4%.

### The one cheap win

`do_call` asked `plain_prim` on *every* primitive call and recomputed the
answer each time from two bounds-checked lookups plus a five-element array
built and linear-scanned on the spot: about 80M times in phase B, for more
than the `car`/`cdr`/`eqv?` work it was gating. The three tables and the
five special ids are all fixed at registration, so it is now a `Vec<bool>`
and one indexed load, with the old logic kept as a `debug_assert` so the
cache cannot drift. Measured as a paired A/B -- both arms `--release
--features jit`, five interleaved rounds so drift lands on both:

| | Steel / T1 before | Steel / T1 after | after / before |
|---|---|---|---|
| `lists` | 0.65× | 0.68× | **1.044×** |
| `fbm` | 1.08× | 1.11× | 1.034× |
| `fib` | 0.83× | 0.85× | 1.027× |
| `strings` | 0.78× | 0.78× | 1.002× |
| **geometric mean, excluding startup** | **0.927×** | **0.946×** | **1.020×** |

2% overall, 4.4% on `lists`, nothing on `strings` -- which is the shape to
expect, since `strings` spends its time inside a few large primitives
rather than calling many small ones. The tables above this section were
measured before the change and are that much pessimistic for T1.

**Interleave the arms.** A first attempt compared the new binary against
numbers taken half an hour earlier and reported 7%; the paired run says
4.4%. On a box at load average 3 the drift between two sittings is larger
than the effect being measured.

### Two things tried that did not work

`#[inline]` on `frame_at` with a `depth == 0` fast path: neutral, 2097 ms
against 2084, and reverted. Worth knowing because the first version of it
wrote `ok_or` where the original had `ok_or_else` -- the eager form builds
the error `String` on every *successful* local reference, and that one word
made `lists` 80% slower, 2084 ms to 3761. A non-error path paying for its
error message does not look like a bug in a profile; it looks like the
interpreter being slow.

### What is left is not cheap

The remaining call cost is structural: every interpreted call builds a
`Vec<(SymId, Value)>` and allocates an `Env` on the heap, which is also
what puts a 9% collector bill on a program that allocates no pairs. Flat
frames, or reusing the environment in place on a self-tail-call when
nothing captures it, is the next real win -- `assv` tail-calls itself 100
times per lookup -- and it needs compile-time escape analysis and a
decision about `call/cc` and `set!`. That is a design change, not a tweak.

## After S14, 2026-10-24

Same machine (Apple M3 Pro), same Steel 0.8.2, scm at `36be1bb` (S14:
flat frames, named-let loops as jumps, inline arithmetic --
[s14-flat-frames-plan.md](s14-flat-frames-plan.md)), built with `--features
jit` so both tiers run from one binary: T1, and the jit column (T2,
`SCM_TIER=t2`). Load average ~1.1-1.6, quieter than either run above. One
run of the harness for both columns; a T1-only run a few minutes earlier
read 1.46x for the same set, so treat the second decimal as noise.

| program | Steel | T1, 10-13 | T1 now | jit now | T1 speedup | Steel / T1 | Steel / jit |
|---|---|---|---|---|---|---|---|
| `startup` | 51 ms | 7 ms | 11 ms | 8 ms | — | 4.7× | 6.0× |
| `fib` 30 | 185 ms | 261 ms | 148 ms | 148 ms | 1.8× | 1.2× | 1.2× |
| `tak` 24 16 8 | 165 ms | 246 ms | 147 ms | 148 ms | 1.7× | 1.1× | 1.1× |
| `loop`, 5M with `modulo` | 384 ms | 513 ms | 247 ms | 245 ms | 2.1× | 1.6× | 1.6× |
| `vec`, fill and sum 1M | 205 ms | 218 ms | 90 ms | 91 ms | 2.4× | 2.3× | 2.2× |
| `strings` | 79 ms | 104 ms | 102 ms | 104 ms | 1.0× | 0.8× | 0.8× |
| `closures`, 1M | 171 ms | 230 ms | 145 ms | 153 ms | 1.6× | 1.2× | 1.1× |
| `lists` | 1772 ms | 3572 ms | 1710 ms | 1720 ms | 2.1× | 1.0× | 1.0× |
| `fbm`, 128² | 138 ms | 135 ms | 78 ms | 79 ms | 1.7× | 1.8× | 1.8× |
| `callcc` | 201 ms | 82 ms | 82 ms | 82 ms | 1.0× | 2.4× | 2.4× |
| **geometric mean, excluding startup** | | | | | | **1.40×** (was 0.91×) | **1.38×** (was 1.06×) |

T1 was the slower of the two on 7 of 9; it is now faster on 7 and level on
`lists`. Loops, calls and arithmetic (`fib`, `tak`, `loop`, `vec`, `fbm`)
moved 1.7–2.4×. `lists`, the case the first reading blamed on the
collector re-marking a large live set, halved: far fewer environments are
allocated, so there is far less to collect. `strings` (string primitives)
and `callcc` (already scm's strength) were never S14's to move.

**The jit column is T1's, and that is expected.** T2 declines any compiled
unit that uses S14's new ops (`Arg`, `SetArg`, `JumpCall`, `Arith` …) until
S14f teaches it them, and every one of these programs uses them, so T2 runs
almost nothing natively. Before S14 the jit tier was the faster one (1.06×
against T1's 0.91×); now plain T1 is faster than old T2 was. S14f -- T2
compiling the new ops -- is where a jit column would separate again.
