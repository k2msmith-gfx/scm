# The differential fuzzer

2026-10-13. `scm-conform/src/fuzz.rs`; `scm-fuzz` in `scm-conform/src/bin`;
the gate in `scm-conform/tests/fuzz.rs`.

```
cargo run --release -p scm-conform --bin scm-fuzz -- --seed 42 --count 2000
cargo run --release -p scm-conform --bin scm-fuzz -- --seed 5 --count 300 --stress
cargo run --release -p scm-conform --features t2 --bin scm-fuzz -- --seed 9 --count 500 --t2
cargo run --release -p scm-conform --bin scm-fuzz -- --seed 1 --count 3 --show
```

Two implementations of one language is the differential method (plan §5).
The suites exercise it on programs people wrote; this exercises it on
programs nobody wrote. A seeded generator builds random, mostly well-typed
programs from a grammar that reaches every form the tiers handle
specially -- `let` and named `let`, closures created in loops and called
after, internal defines, `set!`, `call/cc`, `guard` and `raise`,
`parameterize`, `dynamic-wind`, `do`, `case`, `cond` with `=>`,
`let-values`, records, string ports, vectors, top-level and local macros,
mutual and bounded self-recursion, and fourteen kinds of deliberate error
under `guard` -- and runs each under T0, T1 and, with `--t2`, T2. Any
difference in value, output or error message is a mismatch; the program is
written to `target/fuzz/<seed>-<index>.scm` for replay with `scm` and
`SCM_TIER=t0`. `--stress` collects at every safepoint on T1, which turns a
rooting mistake into a wrong answer. Nothing in the generator parses
Scheme: it writes text, and the implementations do the rest.

## What it found on its first afternoon

**A different error for a use before definition.** An internal `define`'s
variable read inside its own initialiser is an error in R7RS, and both
tiers raised one -- T0 "unbound variable", because its slot did not exist
yet, and T1 "`*`: not a number: #<unspecified>", because it had
pre-allocated the slot. T1's was the misleading one. Those slots now hold
a distinct `Undefined` marker and are read through a checked op, which
costs nothing on parameters and `let` variables; the message is now the
same on every tier, and `set!` before the define is "set!: unbound
variable" as on T0.

**And one thing about the fuzzer itself**: a recursive procedure whose
step called an earlier procedure that called it back with a fresh
argument was unbounded recursion, stopped only by fuel, slowly. The
generator no longer lets a step call out, and fuel is capped at 100,000
safepoints.

## The gate

`cargo test -p scm-conform --test fuzz`: 120 programs T0 against T1, 30
under GC stress, and a check that at least 80 of 100 programs produce a
value rather than an error, so the comparison is of answers and not only of
messages. Fixed seeds, so a failure is a number.
