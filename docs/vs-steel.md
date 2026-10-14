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
