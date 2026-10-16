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

## The published sizes, under T1

Run with the suite's own inputs and a two-minute cap per program, the
same run that gave T0 **3 finished and 41 timed out**:

| | T0 | T1 |
|---|---|---|
| finished within 120 s | 3 | **33** |
| timed out | 41 | 20 |
| outside the subset | 4 | 4 |

Against Chibi's published times for the 29 that finished and
have a Chibi time, the geometric mean of T1 / Chibi is **0.6×**: T1 is
in Chibi's class, which is where a bytecode interpreter over boxed values
should be. (T0 measured 13× slower than Chibi on `fib`.)

| program | T1 | Chibi | T1 / Chibi |
|---|---|---|---|
| `string` | 0.9 s | 10.8 s | 0.1× |
| `equal` | 1.6 s | 39.8 s | 0.0× |
| `read1` | 1.7 s | 213.8 s | 0.0× |
| `tail` | 2.6 s | 5.6 s | 0.5× |
| `sum1` | 8.9 s | 94.0 s | 0.1× |
| `ctak` | 13.2 s | | |
| `paraffins` | 15.4 s | | |
| `bv2string` | 17.2 s | 7.2 s | 2.4× |
| `slatex` | 19.3 s | 96.9 s | 0.2× |
| `primes` | 24.9 s | 17.1 s | 1.5× |
| `fft` | 28.5 s | 41.1 s | 0.7× |
| `fibc` | 29.5 s | 295.8 s | 0.1× |
| `fibfp` | 31.6 s | 34.4 s | 0.9× |
| `nucleic` | 46.3 s | 66.9 s | 0.7× |
| `wc` | 48.0 s | | |
| `cat` | 49.3 s | 38.6 s | 1.3× |
| `sumfp` | 50.6 s | 70.1 s | 0.7× |
| `ray` | 53.6 s | 172.7 s | 0.3× |
| `maze` | 53.8 s | 66.7 s | 0.8× |
| `mbrot` | 62.2 s | 158.4 s | 0.4× |
| `nboyer` | 67.9 s | 28.0 s | 2.4× |
| `sboyer` | 78.1 s | 26.4 s | 3.0× |
| `destruc` | 78.2 s | 69.6 s | 1.1× |
| `deriv` | 80.0 s | 81.1 s | 1.0× |
| `browse` | 89.2 s | | |
| `tak` | 89.2 s | 42.5 s | 2.1× |
| `triangl` | 92.4 s | 66.1 s | 1.4× |
| `divrec` | 96.9 s | 46.4 s | 2.1× |
| `diviter` | 98.0 s | 44.5 s | 2.2× |
| `mazefun` | 98.6 s | 92.3 s | 1.1× |
| `simplex` | 99.4 s | 137.8 s | 0.7× |
| `pnpoly` | 99.9 s | 180.1 s | 0.6× |
| `matrix` | 107.4 s | 171.7 s | 0.6× |

Still timing out at two minutes: `puzzle`, `takl`, `ntakl`, `cpstak`, `fib`, `sum`, `ack`, `array1`, `conform`, `dynamic`, `earley`, `graphs`, `lattice`, `nqueens`, `parsing`, `peval`, `quicksort`, `scheme`, `gcbench`, `mperm`.
Chibi itself needs 60 s for `fib` and 131 s for `nqueens` at these sizes.

## The collection threshold kept re-ramping (2026-10-16)

Measured while asking where Steel is faster on lists. The threshold was
recomputed after every collection as `max(min_heap, 2 × live, visited)`,
which is a growth policy — but one with no memory of itself. A program
that repeatedly builds a large structure and drops it collapses its live
set to almost nothing between rounds, so the threshold collapsed with it
and had to **re-ramp by doubling on every round**.

200 rounds of building and dropping a 100k-element list cost **868
collections**, where a 4 MB floor needed 9.

The threshold now also decays toward the live set rather than collapsing
to it, losing 1/16 of itself per collection. Interleaved A/B, three pairs:

| | collections | time |
|---|---|---|
| collapse to the live set | 868 | 2340 ms |
| decay toward it | **324** | **2081 ms** |

**11%, and it costs no memory.** The slot table only ever grows, so the
high-water mark is already paid for; a larger threshold only delays
collections inside a heap that already exists.

The decay was swept rather than guessed — 445 collections at 1/4, 384 at
1/8, 324 at 1/16, 298 at 1/32 — and 1/16 is where the returns stop. What
remains is structural: each round allocates 100k and the threshold is twice
the live set, so **about one collection per round is the floor for any
policy of this shape**. Going below it means tolerating more garbage, which
is a memory decision rather than a free one, and is not taken here.

### And the heap cap now collects before it complains

Holding the threshold at or below the cap fell out of the same change, and
it fixes a false positive. A safepoint collects *before* it checks the cap,
so a threshold above the cap let `used` reach it with **no collection
attempted** — and a program one collection away from fitting was told it
was out of memory. A doctest was relying on exactly that: a million live
pairs under a 1,048,576-slot cap, which fits once the garbage goes.
