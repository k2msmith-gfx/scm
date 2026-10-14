# S12 — T1, the bytecode tier: results

2026-10-13, aarch64 macOS. `scm/src/code.rs`, `scm/src/vm/compile.rs`,
`scm/src/vm/run.rs`. `SCM_TIER=t0` forces the interpreter; `(%tier-stats)`
reports what ran where.

## The shape: one stack, two engines

Plan §5 said *same stack, same semantics*, and that turned out to be the
whole design. A closure compiles on its first call into a `Code` object —
a flat instruction vector plus its constants, its nested lambdas and a
table of its call sites — and a new `Frame::Code` runs it on the frame
stack T0 already owns. Everything else is shared:

- **the environment objects**: a compiled frame is an `Obj::Env` with
  named slots, exactly what T0 makes, so a compiled closure can capture an
  interpreted frame and the reverse;
- **`apply`**: one dispatch for both tiers, which also compiles a closure
  the first time it is called;
- **the catch, wind, parameter and continuation-mark frames**: `%try` and
  `%parameterize` compile to a sub-block run as a nested frame under the
  same `Catch` and `ParamRestore` frames T0 pushes, so an error raised in
  compiled code is caught by `guard` in interpreted code and `dynamic-wind`
  runs its `after` whichever tier is unwinding;
- **the collector's roots**: the frame stack, plus one value stack, traced
  whole.

What T1 does once that T0 does on every evaluation: a variable becomes a
(depth, index) into the chain or a global cell; a macro use expands once;
a special form is decided once; arguments go straight onto the value stack
instead of through a vector per call; and a primitive with a plain
signature is called on a slice of that stack with nothing built at all.

**Anything the compiler is unsure of, it declines, and T0 runs the unit.**
Declines are counted, and the count is the coverage number: after boot it
is **0**, and it is 0 on every one of the 53 benchmark programs. The 34
`define-syntax` forms in the prelude are the one thing handed to T0 by
design, as sub-forms, since a macro definition is the interpreter's.

## Agreement

`cargo test` is 276 tests written against T0 over ten stages; every one
passes with T1 as the default engine. The conformance suite gives the same
1106 passed, 74 failed, 1180 reached under either tier, with **identical
failure lines**. The 53 benchmark programs return the recorded answers
under both.

Backtraces are the same frame for frame. T0's frames *are* the pending
calls; a compiled frame instead carries, per call site, the pc range over
which its arguments were being evaluated, and the calls pending at a pc
are the ones whose range contains it. Nothing is recorded per call.

## Speed

| | T0 | T1 | |
|---|---|---|---|
| `(fib 30)` | 1.34 s | 0.26 s | 5.0× |
| `(tak 18 12 6)` | 36 ms | 6.3 ms | 5.7× |
| the 53 benchmark programs, reduced sizes | 53.7 s | 8.0 s | **6.7×** |
| the texel loop (`fbm`, 4 octaves), per texel | 79 µs | 12.6 µs | 6.3× |

The outliers in the reduced suite are the ones that should be: `sum1`
(1.0×) reads a file of numbers, which is the reader's time; `equal` (1.2×)
compares in Rust. `nboyer` is 17×, `triangl` 10.8×.

**The §3.2 gate.** The compiled leaf does the texel in **63 ns**: 199× T1.
That gap is what a general tier (S13) would compete for, and the plan's
estimate stands — a general tier over dynamically typed, allocating,
redefinable Scheme would be single-digit multiples over T1, not 199×,
because the leaf earns its number by dropping everything. For ray's hot
code the leaf is the answer; for everything else T1 is now the engine.

## What the oracle found

Three bugs, each caught by a test written against T0:

1. **An unwind must drop the value stack above the catch.** A `%try`
   whose body raised left its half-built call on the value stack; the next
   call applied `(#t . #f)`. Found by the conformance suite as 24
   assertions that stopped being reached.
2. **A macro defined inside a body cannot be a run-time sub-form**, because
   the uses after it in the same body were compiled before it ran. A
   top-level `define-syntax` can be, since nothing in its unit was
   compiled against it. Found as `swap!` unbound.
3. **A lambda inside a loop is a new closure per iteration**, and each was
   compiled — or declined — on its first call. `triangl` compiled 161,938
   times and ran slower than T0. Compile results are now cached per source
   cell, which every closure from one `lambda` form shares.

And a fourth that was T0's: a failure before a form's first call was blamed
on the *previous* form's last one, so `(%tier-stats)` unbound at the REPL
was reported "in `(%make-parameter init)`", the prelude's last call.

## Decided, in the code

- **Compile on first call**, not on definition: the environment the
  closure captured exists by then, so a closure T0 created resolves its
  free variables against the real chain, and the two engines can nest in
  either order.
- **A redefined macro takes effect**: `Code` records the macro table's
  generation, and a closure compiled under an older one recompiles on its
  next call — as T0, which expands lazily, would see it.
- **Internal defines are slots**, found by a scan of the body through
  `begin` and the branches of `if`, which is where T0's dynamic `bind`
  would reach them. A `define` anywhere else declines the unit.
- **Booleans, numbers, everything is a `Value`**; there is no unboxing and
  no type speculation. That is where the next multiple lives, and it is
  S13's.

## Not done

- No instruction-level fast paths for arithmetic: `(+ a b)` is a global
  read, two pushes and a primitive call on a slice. Cheap to add once the
  global is known to still hold the original primitive.
- An `Obj::Env` is a `Vec` per call frame. A pool would delete the one
  allocation left per call.
- `eval` of a run-time form runs on T0.
