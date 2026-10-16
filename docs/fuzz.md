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

## Libraries, and a generator that was testing almost nothing

2026-10-15. Until now the fuzzer generated no `define-library`, no
`import` and no modifier, while the project had reached **twenty embedded
libraries**. The library system has tier-specific paths — a frame whose
definitions are mutually recursive, a macro bound as a value in that
frame, an export resolved against the globals because the library does not
define it — and none of them had ever been compared between T0 and T1
except by the examples in `scm/tests/srfi/`, written by hand.

A generated program now has, half the time, one or two libraries defined
and imported before anything else. What they contain is chosen to hit the
parts where the tiers could disagree:

- a procedure or two, the second able to call the first;
- a **mutually recursive pair**, written in the order that needs the
  frame's bindings visible to each other rather than in sequence, behind a
  wrapper that guards its argument so no call site can hand it a negative
  and never come back;
- an exported **macro**, which a library binds as a value in its frame the
  way `let-syntax` does;
- an exported **variable and setter**, for the copy-at-import
  simplification below;
- an export the library **does not define**, resolved against the globals
  — the fallback `(srfi 133)` and `(srfi 130)` both lean on;
- optionally a dependency on an earlier generated library, so the chain is
  more than one deep.

The import is then one of five shapes — plain, `prefix`, `only`,
`except`+`rename` — and the generator registers *exactly* the names that
shape brings in, which is the part a modifier can get wrong.

### The copy-at-import simplification is now fuzzed, not just documented

[libraries.md](libraries.md) records one deliberate deviation from R7RS: a
binding is copied at import, so a library that `set!`s its own exported
variable afterwards does not reach the importer. That is the kind of thing
a differential fuzzer is uniquely good for — it cannot tell you the
simplification is *right*, but it can tell you both tiers implement the
same one, on every path the generator reaches, which no hand-written test
can claim. A generated program assigns through the setter and reads its own
copy in the same expression.

### Library bodies get their own expression generator

A library body is **not** generated with the ordinary expression
generator, and the reason is worth recording: that one can reach for the
program's top-level `p` parameter and `pt` record type, neither of which
is in scope inside a library. The result would have been a library that
fails to load — a program testing the error path rather than the library
system. The confined generator uses the library's own parameter, its own
earlier definitions, and base arithmetic, with no division and guarded
multiplication, so every generated library loads and every exported
procedure is total.

### The mistake worth writing down

The first version of this emitted libraries and imported them, and was
testing almost nothing. The only path that called an imported procedure
was the generic one-in-fifty `call` arm choosing a name out of *all*
procedures in scope, so across forty programs a library procedure was
called about twice — nine name occurrences in total, and most of those
were the macro and the variable, which have their own arms. The work
looked done.

Library procedures now have their own arm, which took the same count from
**9 to 36 across forty programs**. And
`the_generator_exercises_the_library_system` asserts the coverage
directly: at least 40 of 120 programs define a library, at least 15 of
those *use* an import in the expression whose value the tiers compare, and
at least three distinct modifiers appear. Coverage of a fuzzer's own
output is worth asserting, because nothing else notices it drifting back
to zero.
