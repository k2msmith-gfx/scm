# S5b — the leaf probe: results

2026-10-12, aarch64 macOS, LLVM 21.1.8, `spikes/s5b-leaf`.

```
cargo run                  # the analysis. No LLVM anywhere.
cargo run --features llvm  # and codegen + JIT, with LLVM_SYS_211_PREFIX set.
```

## Why this stage exists

Plan §9 put the leaf compiler at S11. The dependency half of that ordering was
wrong, and a measurement settled it: ray's **entire real texture stack** —
`hash2` → `fade` → `vnoise` → `fbm` → a colour blend, with the accumulator
loop, a closure over a precomputed table and the 3-vector return — **runs in
scm today**, at S5. Nothing between S5 and S11 is a prerequisite.

The slip-insurance half survives, so S11 keeps its place. But "the IR has to
stop moving before the compiler consumes it" had the dependency backwards: the
compiler is a *consumer* of the representation, so its requirements are an
input to that representation's design. S5b buys those answers at 0.2 instead
of reworking them at 0.8.

**S5b is not the leaf compiler.** It compiles eight functions in order to
learn five things, and it found three bugs on the way — two of them in the
interpreter, not the compiler.

## Verdict

| §6 item | verdict |
|---|---|
| (1) transitive compilation, naming the blocking call | **free** |
| (2) captured immutable data | **awkward** — see below |
| (3) local mutation, bounded loops, conditionals | **free, after undoing a macro** |
| (4) monomorphised higher-order calls | **a prerequisite, not a nicety** |
| (5) no-heap return convention for a small float vector | **free on aarch64** |

```
  compiled    fract fade mix hash2 vnoise fbm ridge
  compiled    texel3  -> {double,double,double}
  the module verifies.
  IR: 140 lines, 0 alloca

  343 samples across 7 functions, worst |native - T0| = 0.000e0
  64x64 texels:  native 26.0 ns/texel   T0 61436.4 ns/texel
```

**Bit-exact agreement with T0 on every sample**, which is the differential
method the plan promised, exercised for the first time.

## The five items

**(5) the return convention is free, on this platform.** A
`{double,double,double}` return is an AAPCS64 homogeneous float aggregate and
comes back in registers; `[f64; 3]` on the Rust side matches with no glue, and
the values agree with T0 to the bit. **This is the one finding that does not
generalise** — SysV x86-64 classifies small float aggregates by different
rules, so §2.5's "keep it behind a target hook from the first commit" is
confirmed as necessary rather than cautious. One platform cannot settle it and
this spike did not.

**(1) purity and reachability is free.** `fbm` reports
`calls {fade fract hash2 mix vnoise}` and each refusal names the call that
caused it, including through a chain: `impure-reach` is refused with
*"`display` is not a pure primitive (reached through `impure-io`)"*. The
analysis is fail-closed — an unrecognised primitive is impure until listed —
because the failure mode of an optimistic guess is a data race in a render
thread.

**(4) is a prerequisite, not an optimisation.** The leaf survey listed
"monomorphised higher-order calls" fourth of five, as codegen polish. It is
not. Every texture in the corpus returns its colour through
`(map (lambda (c) …) (list a b c))`, and scm's `map` is **variadic and builds
its result with `cons` + `reverse`**. There is no compiling that. The analysis
refused `texel` outright until the **call site** was specialised — a literal
list of known length and an inline lambda become *n* applications, and `map`
never appears. Without that rewrite, item (4) blocks items (1) and (5) too.

**(2) captured immutable data is the awkward one.** `h-at` reports
`captures {heights}` and the analysis accepts it, but accepting is the easy
half. `Obj::Env { parent, slots: Vec<(SymId, Value)> }` is a linked chain with
linear lookup *by name* — exactly right for an interpreter and the wrong shape
for a compiler, which wants lexical addressing resolved statically. Codegen
here handles only globals that are scalar constants; a captured *table* needs
a read-only snapshot with a layout decision behind it. **Still open, and now
open with a name.**

## (3) and the three findings underneath it

Item (3) reads like the easy one — "local mutation, bounded loops,
conditionals: ordinary SSA work, free from LLVM". It is free. Getting to it
was not, and this is the part worth having learned at S5.

### Finding 1: `let` is an applied lambda, so the compiler must beta-reduce

`let` expands to `((lambda (v …) body) init …)`. Without beta-reduction every
`let` is a frame allocation, and every `let*` is a chain of them. With it, the
emitted module has **0 allocas** — no frame for any `let` in any of the eight
functions.

### Finding 2: a named `let` erases the loop

Every loop in the corpus is a named `let`, and it expands to

```scheme
((letrec ((loop (lambda (i amp freq sum) body)))
   loop)
 0 0.5 1.0 0.0)
```

and `letrec` expands again into a binding initialised to `#f` and then `set!`
to the closure. What reaches a compiler is a **mutable cell holding a
self-reference** — the loop structure is gone, and naively compiled it is a
closure allocation and an indirect call per iteration.

It is recoverable: `as_named_let` matches the shape, and `loop_` emits a
header block with phi nodes, a branch back per tail self-call, and a result
phi at the exit. `fbm` and `ridge` compile to real loops. But **the compiler
has to undo the macro**, and it only works because it looks at the
*unexpanded* operator — an earlier version expanded first, which destroyed the
`letrec` and always returned `None`.

That is an argument the plan should take seriously: `let` and named `let` may
belong in the core as real forms rather than as macros that erase their own
structure. Deciding that is cheap now and expensive at S11.

### Finding 3: there is no expansion pass, so every consumer re-implements one

Expansion is lazy, inside the eval loop, so a closure on the heap holds
**unexpanded source** and no pass has normalised the tree. "How expanded is
this subtree?" has no answer a consumer can rely on. Both halves of the spike
got this wrong once each — the analysis missed the operator position, and the
loop matcher expanded away the shape it was looking for.

S5b added the smallest honest fix: `Vm::expand_once` plus `Vm::is_macro`,
exposing one step and letting the caller own the recursion policy. S8's
`macroexpand-1` wants exactly the same primitive.

## Three bugs, two of them in the interpreter

**1. A body macro is re-expanded on every call, and it costs 3.1×.** Measured,
200,000 calls, same arithmetic in two spellings:

| | 200k calls | per call |
|---|---|---|
| `(define (with-let x) (let ([y (* x 2.0)]) (+ y 1.0)))` | 647 ms | 3.24 µs |
| `(define (no-let x) (+ (* x 2.0) 1.0))` | 207 ms | 1.04 µs |

**A `let` in a function body costs 2.2 µs per call and makes the function 3.1×
slower**, purely because the expansion is redone every time. Nearly every
function in the corpus has a `let`. This makes the case for an expansion pass
— or at minimum a memoised expansion — on its own merits, with no compiler
involved.

**2. And it made correct code fail.** The runaway-macro guard counted
*cumulative* expansions per top-level form, which conflated "this macro
expands into itself" with "this program is big". So

```scheme
(define (f x) (let ([y (* x 2)]) (+ y 1)))
(let loop ([i 0] [acc 0])
  (if (= i 20000) acc (loop (+ i 1) (+ acc (f i)))))
```

failed with `macro expansion of let did not terminate`. Ordinary, correct
Scheme, refused — and **every scene build in the corpus would have hit it**,
which would have shown up at S6 as a mysterious corpus failure. Fixed: the
counter measures *consecutive* expansions, reset whenever a value is produced,
because a runaway macro is an unbroken chain of `Eval → Eval` that never
returns anything. A genuine runaway is still refused; there are tests for both
directions. Requirement 8's real backstops are fuel and the interrupt, both
uncatchable, so this guard is a diagnostic and the looser bound is the right
trade.

**3. A seventh inkwell gotcha for the S0 list.** `CallSiteValue::try_as_basic_value()`
returns `ValueKind`, not `Either`, in inkwell 0.10 — the accessor is `.basic()`
and not `.left()`. Also: a function whose body fails to compile must be
deleted from the module, or one refusal leaves a terminator-less function and
the whole module stops verifying, which looks like a broken compiler rather
than a rejected input.

## What the numbers do and do not say

**26.0 ns/texel compiled** is the number that matters, and it is the right
order against leaf-survey's measured 4,112 ns/texel in ray's Janet.

**The 2,366× against T0 is not a speedup claim.** T0 is an AST interpreter
that re-expands macros per call (bug 1 above), so it is ~15× slower than Janet
per texel. The honest comparison for ray's purposes is compiled-vs-Janet, and
the ratio against T0 mostly measures how much room T1 has at S12.

## Recommendations

1. **Decide `let` and named `let` in the core**, not as macros that erase
   their structure (findings 1 and 2). Cheap now.
2. **An expansion pass, or a memoised expansion.** Bug 1 gives it a measured
   3.1× on ordinary code with no compiler in sight, and finding 3 says every
   consumer needs it.
3. **Item (4) moves to the front of §6's list.** It gates items (1) and (5).
4. **Keep the small-vector return behind a target hook** from the first
   commit of `scm-llvm`, as §2.5 already said. This spike confirms the need
   and cannot discharge it.
5. **Item (2) stays open**, with the specific question named: how a captured
   immutable table is laid out for static addressing.

S11's position in the schedule is unchanged.
