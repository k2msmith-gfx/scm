# cl — plan

A subset of Common Lisp in Rust, compiled, embeddable, scoped to **band 2:
runs most portable CL**. Full ANSI compliance is a non-goal (§1, Non-goals).

**Where this document lives.** The implementation gets its own repository;
this plan sits in `scm/docs` for the same reason [port-plan.md](port-plan.md)
does — every decision in C0 is a decision about *this* repository's value
representation, heap and LLVM glue, and what of it is inherited. Move this
file once the repo exists and leave a pointer behind.

**Status: proposed, nothing built.** No stage below has been started. Every
size is an estimate and marked as one.

## Sources

- The CLHS, fetched per symbol, not recalled. House rule from
  [CLAUDE.md](../CLAUDE.md): two focused fetches — surface first, then exact
  wording for the hard procedures.
- Paul Dietz's `ansi-tests` (~21,000 assertions) as the conformance corpus,
  playing the part `r7rs-tests.scm` plays for scm.
- SBCL as a **live differential oracle** — see §3, the one instrument scm
  never had.
- This repository: [subset.md](subset.md) for what is inherited,
  [s11-leaf.md](s11-leaf.md) and [s13-general-tier.md](s13-general-tier.md)
  for what the LLVM work actually measured.

## 1. Scope

Band 2 means: **Alexandria, cl-ppcre, trivia and ASDF load and pass their own
test suites.** That is C14's exit criterion and it is the whole definition.
It was chosen because it is falsifiable by running someone else's tests, and
because those four between them exercise CLOS, `loop`, `format`, the type
system, the condition system, pathnames and `compile-file` — i.e. reaching it
requires the entire band, and no stage can be quietly skipped.

Reaching it does **not** mean reaching conformance. Expect ansi-tests to sit
somewhere in the 80s or low 90s as a percentage when C14 passes, with the
residue accounted for individually in `docs/conformance.md` — the same
discipline as scm's 1112/71, which is reported as two numbers and never as a
percentage.

### Non-goals

- **Full ANSI compliance (band 3).** Explicitly out. The tail — pathname
  exactness, `*print-circle*` and the pretty printer's corners, `subtypep`'s
  verdicts on compound specifiers, float printing, MOP edges — is where
  ansi-tests is thinnest and where implementations legitimately disagree. The
  mature end of this project is a normative `subset.md`, not a 100% score.
- **Beating SBCL.** SBCL is 40 years of type-derivation work. The speed goal
  is "within a small multiple on declared code," set in C12.
- **Re-entrant continuations.** CL does not ask for them, scm does not have
  them, and that agreement is free.
- **A second platform before C14.** aarch64 macOS first, per scm §2.5.
- **`eval`-less operation.** CL mandates a resident `compile` and `eval`. See
  C12 for how that is met without an LLVM runtime dependency.

## 2. What is inherited, and what is not

The reuse estimate is **25–35% of the code and 80% of the method.** Stated
per subsystem so it can be checked rather than believed:

| from scm | status |
|---|---|
| `heap.rs` — precise non-moving mark-sweep, safepoint-driven, roots = explicit stack + handle table + global cells | **forked, largely intact.** The hardest infrastructure in the project and it exists. |
| the soak harness ([soak.md](soak.md)) — live set and slot high-water over hundreds of thousands of rounds | **inherited as-is.** |
| `bigint.rs` | **inherited**, then extended in C4 for ratios and complex. |
| the explicit stack / spine (§5), proper tail calls, safepoints, fuel, escape continuations | **inherited.** CL's `block`/`return-from`, `catch`/`throw` and `go` are *escape only*, so the existing mechanism is more than enough. |
| `embed.rs` design — `Val`, `#[non_exhaustive]`, zero dependencies, host takes one crate | **inherited as a design**, rewritten for CL's object model in C13. |
| `scm-llvm` — LLJIT creation, ORC context ownership, pinned layouts, the leaf architecture | **inherited**, and C12 reuses the leaf tier's shape directly. |
| three-engine differential agreement + the fuzzer | **inherited as method.** C1a replaces the three engines with something better. |
| `reader.rs` | **~30%.** CL's reader is a different animal: readtable case, `\|…\|`, dispatch macro characters, `#.`, package-qualified names. |
| `syntax.rs` — `syntax-rules`, hygiene | **worth nothing.** `defmacro` is strictly easier and deliberately unhygienic. |
| the `.sld` library system | **worth nothing.** Packages are not libraries; see C1. |

### Why this is a fork and not a dependency

The CL object model diverges from scm's *at the representation level*, in
three places that touch everything:

1. **`nil` is the empty list, false, and a symbol, all at once.** scm keeps
   `'()` and `#f` distinct. This is the deepest single divergence and it
   cannot be layered on top.
2. **Two namespaces.** A symbol has a value cell and a function cell;
   `#'foo` and `foo` are different references. scm has one.
3. **Symbols are objects** with a home package, a property list and those two
   cells — not interned strings.

Sharing a heap across both would constrain both. Fork `heap.rs` and
`bigint.rs`, keep the zero-dependency rule, and let the two diverge.

### The naming question, which scm already hit

`scm` was taken on crates.io and the package publishes as `schemers`. Settle
the crate name at C0 rather than at publication.

## 3. The instrument

scm's velocity came from the verdict function, not the typing: three engines
and a fuzzer gave an automatic answer on every change. **The instrument
available here is strictly better**, and this is the single strongest
argument that the schedule in §9 is real:

- **SBCL is a live oracle.** The differential fuzzer can generate a program,
  run it under SBCL and under this implementation, and compare value, output
  and error class. scm could only compare its engines to *each other* —
  self-consistency, not correctness. A conforming reference implementation is
  a categorically stronger gate.
- **ansi-tests is ready-made**, ~21,000 assertions, and the failure list is
  the deliverable the way `()` is the deliverable for an scm SRFI test.
- **Two engines still, not three.** A tree-walker as the oracle and a
  bytecode VM as the engine, agreeing — scm's T0/T1 relationship. T2's LLVM
  tier is not replicated; §4 says why.

Where the oracle stops helping is where the work gets hardest: pathnames,
the pretty printer, `subtypep` exactness. Those are precisely the band 3
items, and they are out of scope for that reason as much as any other.

## 4. MEASURED: LLVM is not the speed story

This is not an estimate. It is this repository's own measurement, and it
governs C12.

- **T2** — LLVM over the bytecode, [s13](s13-general-tier.md) — is **1.3×**
  on `tak` and `fib` and **0.65× (a net loss)** across the 53-program
  benchmark gate. IR optimisation passes "changed nothing measurable."
- **The leaf compiler's 902×** — [s11](s11-leaf.md) — holds on a restricted,
  pure, all-`f64`, non-allocating subset. That subset is the antithesis of CL
  semantics.

Dynamically typed CL without declarations spends its time in generic
arithmetic, dynamic type checks, generic-function dispatch and allocation.
LLVM optimises none of those. SBCL is fast because of **type derivation**;
CLASP *is* Common Lisp on LLVM and is generally not faster than SBCL while
paying heavy compile latency.

The consequences, both load-bearing:

1. **C12 is a type-inference stage with an LLVM backend attached**, not an
   LLVM stage. The inference pass earns the speed; the backend cashes it on
   declared, monomorphic code — which is the leaf tier, already built and
   already proven.
2. **LLVM stays an opt-in feature.** CL mandates a resident `compile`, but
   the standard only requires `compile` to return a function — compiling to
   bytecode satisfies it. So the bytecode VM is the mandatory engine and the
   zero-dependency rule survives. The alternative is handing every embedder
   the `libffi`/`zlib`/`libxml2` linking tax that [CLAUDE.md](../CLAUDE.md)
   spends four paragraphs on.

## 5. The bootstrap chain is the stage order

Every serious CL is bootstrapped in CL, and not by accident: CLOS, `loop`,
`format`, the pretty printer and the compiler's own front end are far cheaper
in Lisp than in the host language. Even CLASP did not write its front end
from scratch — Cleavir and Eclector are both CL-hosted.

So the target ratio is more extreme than scm's 17k Rust / 8.4k Scheme:
**~25k lines of Rust under 60–100k lines of CL.** The Rust is a quarter of
the job. The rest is written in a language whose compiler is being debugged
at the same time, which is the shape scm already works in — and it is why
**C3 is the hinge of the whole plan.** Nothing expensive can be written until
`defmacro`, `setf` and `eval-when` work.

The chain, which is why §9's stages are numbered and not a menu:

    C1 reader+packages → C2 evaluator → C3 macros+setf+eval-when
                                            ↓
       C4 tower   C5 conditions*  C6 types → C7 CLOS → C10 pretty printer
                                            ↓              ↑
                              C8 sequences+arrays      (needs CLOS)
                                            ↓
                                        C9 loop

`*` **C5 ordering hazard.** `define-condition` is officially a CLOS class,
but a usable condition system is wanted long before C7. Bootstrap conditions
on a minimal struct layer at C5 and **re-base them on real CLOS inside C7** —
that re-basing is part of C7's size, not a follow-up.

## 6. Decisions C0 must settle before any code

Each of these is cheap to decide and expensive to change:

- **`nil` punning.** One object that is the empty list, false and a symbol.
  How it is tagged, and whether `endp`/`atom`/`null` are one check.
- **Two namespaces** in the symbol object and in the calling convention.
- **Multiple values in the convention from day one.** `values`,
  `multiple-value-bind`, `nth-value`, and the truncation rule: a single-value
  context silently takes the first, a missing value reads as `nil`. Retrofitting
  this is a rewrite.
- **Dynamic binding is the default for `defvar`'d names.** `let` on a special
  rebinds dynamically. scm's `parameterize` machinery covers the mechanism;
  the compiler-level question is how a `let` learns a name is special.
- **Readtable case** — the reader upcases by default.
- **Floats.** CL wants short/single/double/long. Decide how many are distinct
  (two is defensible and legal: `short`=`single`, `double`=`long`).
- **Proper tail calls are kept**, though CL does not require them, because the
  spine already does it and the cost is zero.
- The crate name (§2).

## 7. Startup

scm §7 made startup a design requirement and met it. CL makes it harder: the
image carries CLOS, `loop`, `format` and the pretty printer, all written in
CL. **Settle at C0 whether the CL-hosted half is loaded from source at boot
or restored from a dumped image**, and measure it at C7 — the stage where the
loaded half first gets large — rather than discovering it at C14.

## 8. The gates

Nothing is done until these pass, and commit only on green. The shape is
scm's; the contents differ.

    cargo test
    cargo clippy --all-targets                 # kept at ZERO warnings
    cargo run --release -p cl-conform -- ansi-tests          # the corpus
    cargo run --release -p cl-conform --bin cl-fuzz -- --oracle sbcl --count 2000
    cargo test -p cl-conform --test benchmarks
    cargo run --release -p cl-conform --bin soak -- --minutes 60
    LLVM_SYS_211_PREFIX=… cargo test -p cl-llvm                 # from C12

House rules that carry over verbatim: **a gate that passes suspiciously fast
is not passing** (check its inputs exist); `cargo build -p cl-llvm` is not
evidence that LLVM works, because an rlib never links; and the fuzzer is the
main instrument, not the test suite.

## 9. Staging

**Sizes are in scm units, where all of Track A — the whole language, ~25k
lines of gated code — is 1.0.** Every one is an estimate; none has been
built. The total is **8.0**.

Clock: Track A's 1.0 took about five days of work at observed intensity, but
**8.0 units is not 40 days**, because §5's chain is serial — C7 cannot start
before C6, C10 cannot start before C7 — and because the bottleneck shifts
from writing code to reading specification. The CLHS is ~1,100 pages and 978
external symbols, and the house rule is to fetch rather than recall. Budget
**6–10 weeks** and treat any stage that comes in at half its size as evidence
the estimate was wrong, not as time banked.

| | stage | size | exit criterion |
|---|---|---|---|
| **C0** | **The decisions.** §6's list in full, §2's fork boundary, §7's startup choice, the crate name. | 0.1 | this document plus a findings doc; a measurement that the forked heap holds a symbol-with-two-cells object without redesign |
| **C1** | Reader, printer, **packages**. Readtable case, `\|…\|`, `#\`, `#'`, backquote, dispatch macro characters, `#.`. Packages: `intern`, `use`, `shadow`, `export`, `::` vs `:`. Printer: `*print-base*`, `*print-case*`, `*print-escape*`. | 0.5 | the round trip holds on a corpus; ansi-tests' reader and packages sections *reached* (not passed); incomplete-vs-invalid reporting per scm S1 |
| **C1a** | **The oracle** (§3). The ansi-tests harness, and the differential fuzzer against SBCL comparing value, output and error class. | 0.2 | 2000 generated programs agree with SBCL; a known-wrong build is caught by the fuzzer and not by a human |
| **C2** | The evaluator on the forked spine. All **25 special operators**; full lambda lists (`&optional` `&rest` `&key` `&aux`, `supplied-p`, `&allow-other-keys`); two namespaces; `block`/`return-from`; `tagbody`/`go`; `catch`/`throw`; `unwind-protect`; multiple values; special binding. | 0.7 | a 10^8 tail loop holds the control stack at depth 2; `go` to a label backwards through a `tagbody` works; `unwind-protect` runs on every exit path including `throw` |
| **C3** | **The hinge** (§5). `defmacro`, backquote as a macro, `macrolet`, `symbol-macrolet`, `setf` — the expander protocol plus the ~60 standard places — `eval-when` and the three evaluation times, `compiler-let`-free. | 0.5 | the implementation compiles a non-trivial macro-defining file of **its own CL source**; `(setf (getf …))` and `(setf (ldb …))` both work |
| **C4** | The numeric tower **completed**: ratios, complex, the float subtypes, the `floor`/`ceiling`/`truncate`/`round` family returning two values, byte specifiers, `rational`/`rationalize`. This is new work — [subset.md](subset.md) refuses all of it. | 0.4 | differential agreement with SBCL on a generated numeric corpus including mixed-type contagion; ansi-tests numbers section |
| **C5** | Conditions and **restarts** on a minimal object layer (§5's hazard). `define-condition`, `handler-bind` (handlers run before unwinding), `handler-case`, `signal`, `error`, `restart-case`, `restart-bind`, `invoke-restart`, `*debugger-hook*`. | 0.3 | a handler established outside a failing call invokes a restart established inside it, and the stack between them is still live when it runs |
| **C6** | Types and declarations. `deftype`, `typep`, `subtypep`, `the`, `declare`, `check-type`, compound specifiers, `satisfies`. Required before C7 (classes are types) and before C8 (`:element-type`). | 0.4 | `subtypep` returns the right *two* values — verdict and certainty — on ansi-tests' lattice cases, and never a confident wrong answer |
| **C7** | **CLOS and enough MOP.** `defclass` with `:initform`/`:initarg`/`:allocation`/accessors; `defgeneric`/`defmethod`; multiple dispatch; standard method combination, `call-next-method`, `next-method-p`; `define-method-combination`; `make-instance`/`initialize-instance`/`shared-initialize`; `change-class`; class redefinition; `print-object`; the MOP surface libraries use (`class-of`, `find-class`, `slot-value`, `class-slots`, `validate-superclass`). **Re-base C5's conditions on it.** | 1.2 | multiple dispatch on three required arguments picks the same method SBCL picks across a generated lattice; a redefined class updates live instances; the condition hierarchy is real classes |
| **C8** | Sequences, arrays, hash tables, strings. Multidimensional, adjustable, displaced and specialized arrays; fill pointers; the ~40 sequence functions across `:key :test :test-not :start :end :from-end :count`; `sort`/`stable-sort`; the four standard hash-table tests. | 0.7 | the keyword cross-product is generated and differentially tested rather than hand-written; a displaced adjustable array survives `adjust-array` |
| **C9** | **`loop`** — the full ANSI loop — plus `do`, `do*`, `dolist`, `dotimes`, `mapc`/`mapcar` family. Written in CL on C3. | 0.4 | ansi-tests' loop section; the destructuring, `being the hash-key` and multiple-accumulation clauses all work |
| **C10** | **`format`** — the full directive set, in CL — and the **pretty printer**: `pprint-logical-block`, `*print-pretty*`, `*print-circle*`, dispatch tables. Needs C7. | 0.6 | ~100 directives differentially tested against SBCL; `*print-circle*` prints a self-referential structure and reads it back |
| **C11** | Streams, **pathnames**, `load`, `compile-file`, fasls, `load-time-value`, literal coalescing. **Scope the pathname surface explicitly** to a documented subset — this is the band 3 swamp and the line is drawn here, in writing. | 0.5 | a `compile-file`'d fasl loads and behaves identically to `load`ing the source; the pathname subset is stated normatively and tested |
| **C12** | **The compiler** (§4). Type inference honouring `declare` and `the`; fixnum and float unboxing; then the LLVM backend for declared code, reusing the leaf tier's architecture. `compile` means bytecode always; LLVM is opt-in. | 0.8 | a declared `double-float` inner loop is within a small multiple of SBCL; an **undeclared** program is no slower than before the stage; the zero-dependency build still runs every other gate |
| **C13** | The embedding API. CL's object model across the boundary — symbols, packages, multiple values, conditions — `#[non_exhaustive]`, zero dependencies, one crate. | 0.3 | a host embeds it, calls a generic function, catches a condition and invokes a restart from Rust; the crate has no dependencies |
| **C14** | **The band 2 gate.** Alexandria, cl-ppcre, trivia and ASDF load and pass their own test suites. Mostly debugging what those four reveal. | 0.4 | all four green; the ansi-tests number reported as two numbers; the residue accounted for individually in `conformance.md` |

## 10. Risks, in the order they bite

1. **C7 is the stage that can double.** CLOS plus the MOP surface real
   libraries touch is the largest single piece of CL, and 1.2 is the least
   confident number in the table. If it comes in at 2.0 the total is 8.8 and
   the band still closes; if it comes in at 3.0, revisit scope.
2. **The serial chain sets a floor on clock time** that no rate can beat.
   C3 gates everything expensive; C6 gates C7; C7 gates C10.
3. **Pathnames (C11) will try to become band 3.** The mitigation is written
   into C11's exit criterion: state the subset normatively, then stop.
4. **The spec-reading cost is irreducible.** 978 symbols × "fetch, do not
   recall." This is the main reason days scale worse than lines.
5. **`subtypep` can return a confident wrong answer**, which is the worst
   failure class there is: a confidently wrong verdict is indistinguishable
   from a right one at the call site. C6's exit criterion names it for that
   reason.
6. **The oracle makes a bad habit cheap.** Differential agreement with SBCL
   proves agreement, not conformance; where SBCL extends the standard, copying
   it silently is a deviation. Record those in `conformance.md` as they occur,
   not at the end.

## 11. What this costs after it is built

Build time compresses with rate. **Maintenance does not.** scm is ~25k lines
and justified because ray needed a Janet replacement. Band 2 is 100k+ lines
of Rust *and* CL to carry alongside ray, the modeler and scm — and scm is now
ray's default scripting language, so it is not going anywhere either. Ten
weeks to build, then an indefinite tail as sole maintainer of a Common Lisp.

**What it displaces, concretely.** The modeler port stands at 68/117 Scheme
([modeler-port-plan.md](modeler-port-plan.md), next = §2m L, then M4).
Starting C0 before that closes strands the port at 58% with ray's scripting
language half-ported. C0 should not begin until §2m and M4 are done.

**The cheaper alternative, stated so the comparison is on the record.** A
CL-flavoured layer *on scm* — `defmacro`, restarts over the existing
`raise-continuable` machinery, a `defclass`/multimethod library, `loop` as a
macro, CL-style `format` extending what is there, packages mapped onto
libraries — is roughly **0.3 units, about two weeks**, and delivers most of
what is pleasant about CL inside ray and the modeler while keeping zero
dependencies and every existing gate. It runs almost no existing CL, which is
the entire difference, and it is the right answer if the goal is a CL to
*use* rather than a CL to have built.
