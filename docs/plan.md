# scm — plan

An R7RS-small Scheme in Rust: embeddable as a crate, runnable standalone, with
an LLVM-backed JIT and native AOT compilation on x86-64 Linux and aarch64
macOS — and ray's scripting language.

Status: **planning**, S0 partly banked. This document is the argument for a
shape, not a schedule.

## Sources

This is a merge of two documents written on the same day:

- **this plan**, the language-side argument (tiers, continuations, GC, REPL,
  conformance), with [s0-findings.md](s0-findings.md) behind it.
- **`~/devel/ray/docs/scheme-plan.md` @5c2c2f6**, the consumer-side
  requirements: what ray's 218 verbs and 26,700 lines of authored Janet demand
  of a host language, and what ray does *not* need.

Where they disagreed, §3 says so and resolves it. ray's convention is carried
over: *intent from before the build; where this and the code disagree, the
code wins.*

---

## 1. Requirements, merged

**Language-side (as given):**

1. Support R7RS-small — see §3.1, which qualifies this.
2. Easy embedding in a Rust application, as a crate.
3. Native compilation on x86-64 Linux and aarch64 macOS. **Decided
   2026-10-12: aarch64 macOS first, x86-64 Linux caught up later** — see §2.5.
   Linux stays a requirement, not a non-goal; it is sequenced, not dropped.
4. Leverage LLVM — `inkwell` as the interface for IR.
5. Run standalone.
6. Support JIT compilation.
7. Strong REPL support.
8. **A runtime that cannot abort the host** — a panic cannot unwind out of
   `extern "C"`, and a zero NURBS weight killed a live modeler session
   mid-edit (@7d8c8a1). Plus interrupt, timeout and fuel, which ray has none
   of today.

**Consumer-side, from ray's plan:**

9. **Keyword and optional arguments, and a keyword datatype.** R7RS-small has
   none of the three. This is the single largest language-design consequence
   of having ray as the consumer — see §4.
10. **Error text is a contract.** `VerbDesc.errors` holds every refusal
    verbatim and `pyray/tests/conformance.py` asserts the wording survives the
    crossing. R7RS error objects differ in shape from Janet's; the strings must
    not.
11. **Dynamic variables**, 7 in use. `parameterize` maps onto `with-dyns`, and
    `:ray-rng` is load-bearing for `:seed` reproducibility — `grow-branches`
    is only replayable because of it.
12. **A macro set that must survive**: `(material …)`, `def-tool`, the
    `[label form]` idiom, the `(cond …)`-shaped authoring helpers. R7RS-small
    offers `syntax-rules` only, and `(material :albedo …)` is keyword-shaped,
    so this depends on (9).
13. **Compiled field and texture functions callable from rayon workers** with
    no VM dependency. This is the *capability* that justifies the project, and
    it collides with ray's deliberately thread-local store — §6.
14. **Two constraints carried from `rust-api-plan.md`:** the Janet API is
    preserved until a Scheme replaces it verb-for-verb, and UI chrome stays
    scripted — toolboxes, marking menus, keymaps, `init.janet`, session
    persistence, the editor pane, SLIME sends, the AI pane. A second language
    must not have to re-author any of it.

### Non-goals

R7RS-large. Threads and parallelism *in the language* (distinct from (13),
which is about compiled leaf code). Windows. Self-hosting. An FFI beyond what
embedding needs. Making scene loading faster — see §3.2.

---

## 2. What was verified, not recalled

### 2.1 inkwell for IR, `llvm-sys::orc2` for the JIT

inkwell 0.10.0 supports LLVM 12.0–22.1 behind per-version feature flags. But
its `ExecutionEngine` calls `LLVMLinkInMCJIT` — the **legacy MCJIT** path —
and its only ORC code is behind `experimental` and targets the **removed**
ORCv1 C API. The modern LLJIT bindings are in `llvm-sys`'s `orc2` module.

**S0 ran 2026-10-12 and passed on aarch64-darwin**: inkwell built the IR, both
MCJIT and ORCv2/LLJIT compiled and ran it, and a second module added
afterwards resolved while the first stayed callable. Warm-up is sub-millisecond
per function. Full results and six API gotchas in
[s0-findings.md](s0-findings.md). x86-64 Linux is unverified, and by the
decision in §2.5 that no longer holds S0 open.

### 2.2 The toolchain is pinnable

`nixpkgs#llvmPackages_21.llvm` is **21.1.8**, matching inkwell's `llvm21-1`.
`llvm-sys` is already published at **231** (LLVM 23), two majors ahead, so the
flake pin is what keeps the build reproducible while that moves. The Nix shell
is needed only for the LLVM-enabled build, never for the default crate.

### 2.3 Dropping bignums costs nothing in conformance terms

R7RS requires exact integers only "throughout the range of numbers permitted
as indexes of lists, vectors, bytevectors, and strings"; beyond that an
implementation "may report the violation of an implementation restriction".
Unbounded exact integers and exact rationals are advertised **optionally**,
through the `exact-closed` and `ratios` feature identifiers. `full-unicode` is
the same kind of flag.

This matters for §3.1: two of the four cuts ray's plan proposes are not
deviations at all.

### 2.4 The verb counts, re-measured

From `api/ray/api.jsonl`, 218 verbs:

| | count |
|---|---|
| verbs with keyword arguments | 67 |
| verbs with `&opt` | 79 |
| verbs with **both** | 59 |
| **union — verbs whose calling convention is at stake** | **87** (40%) |
| `&named` | 27 |
| signatures containing a `[x y z]` vector literal | 30 |

ray's plan says "the calling convention changes for **146 of 218** verbs".
That is 67 + 79, and 59 verbs have both, so the union is **87**. The
conclusion is unchanged and the decision is the same — but it is 40% of the
API, not 67%.

### 2.5 One platform first: aarch64 macOS

**Decided 2026-10-12.** Development targets aarch64 macOS only; x86-64 Linux
is caught up later. It stays requirement (3), sequenced rather than dropped.

This is the cheap direction to defer, for a specific reason: **the hard
platform is the one we have.** S0 found the only platform-specific hazard on
the macOS side — W^X on Apple Silicon, `MAP_JIT`, and the hardened-runtime
`com.apple.security.cs.allow-jit` entitlement. x86-64 is LLVM's best-supported
JIT target and has no equivalent complication, so deferring it defers the
easier half.

Three things it does genuinely risk, worth writing down now rather than
meeting them in month five:

1. **The small-vector return convention is ABI-specific.** §6's item (5) wants
   a 3×`f64` returned in registers with no heap traffic. AAPCS64 does that as
   a homogeneous float aggregate; SysV x86-64 classifies the same struct by a
   different rule. The *design* is portable, the *lowering* is not, and it is
   the one leaf-compiler decision that cannot be validated on one platform.
   Keep the convention behind a target hook from the first commit of
   `scm-llvm`, even while only one arm of it exists.
2. **`llvm-config` discovery and the flake differ per platform**, and NixOS is
   where ray already hit a startup crash over GUI libs. Expect the Linux arm
   of the flake to be its own small job, not a free `#` away.
3. **A deferred target rots.** Every binding that is not the primary one rots
   — ray's own API plan says so about Python, and it is the same mechanism.
   *Mitigation: the no-LLVM default build is pure Rust, so `cargo test` on
   Linux stays meaningful from day one even with no LLVM arm; add x86-64 to CI
   as soon as there is CI, and run the S0 spike there before S11 needs the
   leaf compiler's ABI to be real.*

Until then, "both targets" in §7's exit criteria reads "aarch64 macOS, with
x86-64 Linux owed".

---

## 3. Where the two plans disagreed

### 3.1 "R7RS-small" vs. a deliberate subset

**The conflict.** Requirement (1) says support R7RS-small, and the language
plan made full multi-shot `call/cc` the thing the whole architecture was built
around. ray's §3 cuts four features as things ray has no use for: full
`call/cc` (escape-only instead), `dynamic-wind`, the numeric tower, and full
Unicode.

**Resolution.** The four cuts are not the same kind of thing:

| cut | status |
|---|---|
| numeric tower | **free** — optional via `exact-closed` / `ratios` (§2.3) |
| full Unicode | **free** — optional via `full-unicode` |
| `dynamic-wind` | **a real deviation** — it is in `(scheme base)` |
| multi-shot `call/cc` | **a real deviation** — also `(scheme base)` |

And §4 adds a third, from the other direction: reading `:at` as a keyword
rather than as the symbol R7RS says it is.

So two of the four cost nothing but an honest `(features)` answer, and only
two are genuine non-conformance.

And for those two, **the explicit stack (§5) means multi-shot `call/cc` is
deferrable rather than forfeited.** The expensive thing is retrofitting
re-entrant continuations onto a design that keeps frames on the C stack; onto
an explicit stack, capture is a data-structure operation you can add later.
`dynamic-wind` is cheap and `parameterize` interacts with it anyway, so it
goes **in**.

#### Continuations are a three-rung ladder, not a yes/no

Both source documents wrote this as full `call/cc` versus escape-only, which
hides the rung in the middle — and the middle rung is the one ray would
actually use.

| rung | what you may do with `k` | implements | cost |
|---|---|---|---|
| **escape-only** (`call/ec`) | invoke it only while the extent is live, only outward | exceptions, early return, `break` | an unwind; nearly free |
| **one-shot** | invoke it **once**, possibly after the extent returned | coroutines, generators, resumable tools | the frame chain must outlive the extent, but it may be **moved** — no copy, and frames may still be mutated in place |
| **multi-shot** | invoke it **any number of times**, re-entering fresh each time | backtracking (`amb`), re-running from a saved point | the captured chain must stay reusable: copy-on-capture, or immutable frames — which costs straight-line speed |

**One-shot is the cheap middle option**, and the gap between it and multi-shot
is larger than the gap between it and escape-only. One-shot needs the frame
chain to survive the extent returning; it does *not* need the chain to survive
being resumed, so nothing has to be copied and the interpreter may keep
writing locals straight into frame slots. Multi-shot is what forces either an
expensive capture or an immutable-frame discipline, and that discipline is the
only part of this that taxes ordinary code.

**And one-shot is the rung with a named future use.** ray is already
hand-rolling it: `rust-api-plan.md` §3.5 and `modeler-tool-layer-review.md`
both specify the minibuffer hook as `(read-tool-field prompt seed k)` — that
`k` is a continuation, passed explicitly because a tool that wants input has
to give the runtime something to resume. The same shape is the whole
begin/preview/commit tool lifecycle: a tool waiting on the next drag event is
a paused computation modelled as a state machine. With one-shot continuations
a multi-step tool is straight-line code:

```scheme
(define a (read-point "from"))
(define b (read-point "to"))
(commit (make-line a b))
```

Nothing in ray wants multi-shot. The relaxation in `field-points`,
`grow-branches`' collision avoidance and voronoi relaxation are iterative, not
backtracking, and `:seed` + replay already covers re-running. Evidence that
demand is low: Janet *has* fibers and the corpus uses **zero** — not one
`(yield`, `(coro`, `(fiber/new` or `(resume` in `scripts/` or `examples/`.

**Therefore:** ship escape-only first, keep `dynamic-wind`, treat the numeric
tower and Unicode as absent optional features, and declare all of it in
`(features)`. Design the frame chain so that **one-shot is reachable without a
frame-representation change** — that is the rung to leave a door open for, and
it is cheaper to leave open than multi-shot. The claim is "R7RS-small with
multi-shot continuations not yet implemented", which is checkable, and becomes
full conformance later without an architectural change. ray gets exactly what
its §3 asked for; the standalone language (requirement 5) is not painted into
a corner.

### 3.2 What the LLVM work is *for* — and the staging consequence

**The conflict, and it is the big one.** The language plan put the JIT last
(S11/S12) on the reasoning that if the schedule slips, what survives is a
conformant embeddable interpreter. ray's plan says the opposite: compiled code
*is* the justification. Its §1 is blunt that this is **not** for making
scripting faster — the 2026-09-12 measurement found no Janet hot spot and a
~3,200× gap between interpreter time and real work, and scene loading will not
get faster. The one capability is (13): a user's own SDF that render threads
can actually call.

**Resolution: the LLVM work is two different projects and the old plan
conflated them.**

- **The leaf compiler** — compile a *restricted pure* Scheme subset to a
  thread-safe native function pointer. Small, self-contained, needs no GC
  integration, no deopt, no invalidation. **This is requirement (13) and it
  belongs early.** [leaf-survey.md](leaf-survey.md) measured what it is
  actually for, and reordered the justification: `make-texture` speed first
  (**15.22 s measured** in one real scene build, ~40 call sites, 150-200x
  available), field *quality* second (it kills the three documented bake
  artifacts), field thread-safety third and least load-bearing. Target
  textures first — bigger win, easier target, and no semantic change at all
  since a texture is already a bake.
- **The general JIT tier** — compile arbitrary Scheme with assumption
  tracking, invalidation on redefinition, and VM fallback. Big, and by ray's
  own measurement worth little to ray. **Stays last, and may never be worth
  starting** — which ray's §8 already says out loud.

This is the most valuable thing the merge produces. It moves the capability
forward and demotes the expensive half honestly, instead of averaging the two
plans' orderings into something that serves neither.

### 3.3 GC: arena vs. collector

**The conflict.** ray's §5 suggests an arena per evaluation with no collection
may genuinely be enough, since a scene build allocates and then ends. The
language plan specified non-moving mark-sweep.

**Resolution: both, and the collector is not optional.** The arena argument
holds for `--batch` and for a scene build. It fails for the two things
requirements (7) and (14) commit us to: a modeler session that runs for hours,
and a REPL where the user redefines things all afternoon. An arena that is
never collected is a leak with a long fuse.

So: mark-sweep, with the explicit stack as the root set (§5), **plus** an
arena fast path for a bounded evaluation that is known to end — which is
exactly the leaf-function and scene-build case. The collector exists; the
arena is an optimization that skips it.

### 3.4 S0 means two different things

ray's S0 is "the decisions, no code". The language plan's S0 was the LLVM
feasibility spike, which has now run. Merged: **S0 is ray's decision stage**,
and the LLVM spike is banked evidence feeding it (§2.1). The decisions it owes
are §4's calling convention, §3.1's subset, §3.3's GC, and whether §6 is
reachable.

---

## 4. The calling convention: 87 verbs, one function

R7RS-small has no keyword arguments, no optional positionals, and no keyword
datatype. ray needs all three — `(add-sphere :at [0 1 0] :radius 0.8)` is the
house style across 219 example files, and `:aces` has to be a *value*, which
is why `ParamType::Key` exists.

The good news is in ray's plan and it is real: the convention lives in exactly
one place, `#[verb]`'s shim generator in `api-macro/`, so changing it is one
function rather than 87 edits.

### DECIDED 2026-10-12: keywords are a first-class datatype

Not negotiable and not deferred — they are used constantly in authored scenes.
The two alternatives are recorded as rejected rather than deleted: keywords as
symbols with optionals by arity dispatch (conformant, but needs
`(add-sphere ':at …)` or a reader hack, and makes a *misspelled* option
silently mean something); and a trailing options hash or plist (clean in the
language, re-authors 219 example files and every saved scene).

The carried lesson from the Steel work is why it must be a real type and not
an alias: `(define :at ':at)` instead of `#:at` bound a different symbol, every
`opt` lookup silently missed, and cornell-metals rendered a **pure black frame**
with no error anywhere. A keyword has to *be* a keyword, and a misspelled one
has to be an error at the call site.

What the decision commits us to, concretely:

1. **A distinct type.** `(keyword? :at)` → `#t`, `(symbol? :at)` → `#f`,
   `(eq? :at 'at)` → `#f`. Interned, so `eq?` works and keywords are usable
   as hash keys — ray passes `:aces` and friends around as enum *values*,
   which is why `ParamType::Key` exists. `keyword->string` /
   `string->keyword` both ways.
2. **Self-evaluating**, so `(add-sphere :at [0 1 0])` needs no quoting
   anywhere, in authored scenes or in saved ones.
3. **Round-trips through `write`** as `:at`, which is what makes S1's gate —
   every saved corpus scene printed through `Form::to_scheme()` reads back
   equal — actually testable.
4. **`ParamType::Key` maps straight onto it** across the bridge, with no
   coercion step to get wrong.

### DECIDED 2026-10-12: the lambda list keeps Janet's markers, with inline defaults

```scheme
(define (add-sphere-mesh center radius segments rings
                         &opt material
                         &named (smooth #t) name)
  …)
```

**Markers are `&opt`, `&named`, `&keys`, in that fixed order**, after the
required positionals. Janet's names, because ~25k lines of authored Janet use
them and the translation should be mechanical: 105 `&opt`, 81 `&named`, 8
`&keys` in `scripts/` alone. `&keys` binds the whole keyword collection as one
value. True rest args stay R7RS: `(lambda (a . rest))`, no `&rest`.

This is a **cheaper deviation than the keyword spelling**, and for a reason
worth recording: `&` is an R7RS ⟨special initial⟩, so `&opt` is already a legal
symbol. Nothing about the *reader* changes — only what `lambda` does with a
parameter list. (In strict R7RS `(lambda (a &opt b) …)` is a two-argument
procedure with a parameter named `&opt`; nobody writes that.)

**The parameter name *is* the keyword.** `&named smooth` binds from `:smooth`.
One name, not two — which is also what keeps a generated manual honest.

#### Inline defaults are forced, because Scheme has no nil

This is the part that cannot be translated literally, and getting it wrong
would be silent. Janet writes defaults in the *body*, 80 times in the prelude:

```janet
(defn add-sphere-mesh-impl [center radius segments rings &opt material &named smooth name]
  (default smooth true)   # = (if (nil? smooth) true smooth)
```

That works only because an absent argument is `nil` and **`nil` is not
`false`**. Scheme has no nil; `#f` is the only false value. So if an absent
`&named` bound `#f`, then `:smooth #f` and omitting `:smooth` would be
indistinguishable, and a body-side `(default smooth #t)` would silently
rewrite an explicit `#f` to `#t`.

That is not hypothetical. ray documents the distinction:

- `:smooth` — *"interpolated vertex normals · default true (**false = flat
  facets**)"*, three sites
- `:apply` in `camera-match` — *"**false** solves without touching the camera
  or film · default true"*
- `:repro` — `(default repro true)`

A port that bound absent to `#f` would turn flat facets smooth, and make
`:apply false` touch the camera anyway, with no error at all.

**So defaults move into the lambda list**, where the default expression is
evaluated only when the argument is absent and an explicit `#f` passes
through untouched. Absent with no declared default binds **`#f`**, which
preserves Janet's falsy-`nil` ergonomics so the 26 `(nil? x)` / `(if x …)`
supplied-checks translate directly. Every parameter in ray where `#f` is
meaningful already has a default, so no supplied-predicate is needed; if a
case ever turns up, a CL-style third slot is the escape hatch to add *then*.

Migrating the 80 `(default …)` calls into lambda lists is mechanical, and it
deletes a drift source: today "default true" is written in the docstring prose
*and* `(default smooth true)` in the code, two places that can disagree.

#### One genuine ambiguity, with a rule

21 prelude lambda lists have **both** `&opt` positionals and `&named`
parameters, so this has to be answered rather than assumed:

> A keyword argument begins the named section **only if it names a declared
> `&named` parameter.** Otherwise it is an ordinary positional value.

Both real cases fall out correctly:

- `(add-sphere-impl c r :name :ball)` against
  `[center radius &opt material &named name]` — `:name` *is* declared, so the
  named section starts and `material` stays absent.
- `(set-tone-map :aces)` — `set-tone-map` declares no `&named` called `aces`,
  so `:aces` is the positional value it looks like. This matters: keywords are
  passed as plain values at hundreds of call sites.

Two errors, both at the call site: a declared keyword with no value after it
("named argument `:x` has no value"), and — importantly — **an undeclared
keyword in the named section**. Janet *silently ignores* an unknown named
argument, which means a misspelled `:radiuss` does nothing and says nothing.
scm rejects it by name. That is the same class of bug as the Steel black frame
and the cheapest possible place to catch it.

### And it is a genuine deviation, which the first draft got wrong

The first version of this section said option 1 "costs conformance nothing —
extra datatypes are permitted". The datatype is indeed free. **The spelling is
not.** R7RS §7.1.1 defines

```
⟨special initial⟩ → ! | $ | % | & | * | / | : | < | = | > | ? | ^ | _ | ~
```

so **`:` is a valid identifier initial and `:at` is a legal R7RS symbol.**
Reading it as a keyword changes the meaning of conformant code. That is why
Racket, Steel and Guile all spell it `#:at` — `#:` is not valid R7RS datum
syntax, so it cannot collide. Janet spells it `:at`, which is where ray's
house style comes from, and scm follows Janet here: we are taking the
deviation on purpose rather than taking the prefix to avoid it.

So this is a **third real deviation**, alongside multi-shot `call/cc` (§3.1),
and it needs handling rather than hiding:

- **`:at` reads as a keyword. Full stop — there is no prefix and no alias.**
  That is ray's spelling across 219 files, it is not being re-authored, and
  scm owns its own reader so there is nothing to work around.
- **`#:at` is NOT accepted**, and that is deliberate (decided 2026-10-12).
  Keywords are not in R7RS-small at all, so no portable conformant library
  uses `#:at` either — only Racket/Guile/Steel-specific code does, which is
  not portable anyway, and the wider ecosystem is split three ways regardless
  (`#:at` in Racket/Guile/Steel, `at:` in SRFI-88 and Gambit, `:at` in Janet).
  A second spelling would be a permanent tax on the reader, the printer, the
  docs and the error messages, to save an `s/#:/:/` on a pasted snippet. If a
  port ever genuinely needs it, it is a five-line reader branch — add it then,
  and still never *emit* it.
- **A reader parameter restores R7RS symbol reading**, for running the
  conformance suite and for any library that genuinely wants `:at` as a
  symbol. Guile does exactly this with `(read-set! keywords …)`. This one is
  worth keeping, because the conformance number depends on it.
- **Declared in `(features)`**, like every other deviation.

**The real lesson from Steel was the alias, not the prefix.** Steel's reader
spells keywords `#:at` and reads `:at` as a symbol, so preserving the house
style there meant `(define :at #:at)` — and that alias is precisely what
failed. Making `:at` native removes the alias, and with it the entire class of
bug. Any future proposal to reintroduce a second spelling should be read with
that in mind.

Check at S5 whether `r7rs-tests.scm` uses a `:`-initial symbol anywhere — if
it does, the conformance runs need the reader parameter, and a failure there
would otherwise look like a bug rather than a configured dialect.

Three smaller contracts from requirements (10)–(12): error strings pass
through verbatim; `parameterize` must not reproduce Janet's trap where
`with-dyns` around `eval-string` makes top-level definitions vanish; and each
surviving macro must be shown expressible in `syntax-rules` *before*
committing, because the keyword-shaped ones depend on this section.

---

## 5. The spine: one explicit stack

The hard problems — continuations, proper tail calls, GC roots, interrupts,
backtraces — are the same problem wearing different hats: *who knows where the
live values are?* Answer it once.

**The VM owns an explicit, heap-allocated frame stack. Native code borrows it;
it never replaces it.**

Note that §3.1 removed multi-shot `call/cc` from the justification. The stack
survives that easily, because it is load-bearing for five other things:

- **Proper tail calls** are frame reuse — no dependence on LLVM's `musttail`,
  whose guarantees vary by target. ray's §3 keeps TCO explicitly **in**, and
  retrofitting it into a compiler is not a late change.
- **The GC root set is the explicit stack**, plus a handle table for
  embedder-held values. No LLVM statepoints, no shadow stack, no conservative
  scanning of C frames. This is the answer to ray's §5 "a JIT needs a GC
  story".
- **Interrupt, timeout and fuel** (requirement 8) are a safepoint check in the
  loop. ray has none of this today, and it is the one throttle on
  AI-generated code that the Steel evaluation found Steel would not have
  given ray either.
- **Backtraces are a walk over live data**, not a reconstruction — which is
  what makes the break REPL in §8 reachable.
- **Escape-only continuations** are nearly free; **one-shot** — the cheap
  middle rung, and the one a resumable modeler tool would use — needs only
  that a frame chain outlive its extent, not that it survive being resumed;
  multi-shot stays available later at the cost of an immutable-frame
  discipline (§3.1).

The accepted price, stated plainly: frames in memory cap straight-line speed.
The general JIT tier will not turn a Scheme call into a native `call` with
arguments in registers. By ray's own measurement that ceiling costs ray
nothing (§3.2) — and the leaf compiler, which is the part that matters, is
*not* subject to it, because leaf functions have no frames worth the name.

### Tiers

| tier | what | when |
|---|---|---|
| **T0** | AST interpreter over the explicit stack | first, and kept forever as the semantic oracle |
| **T1** | bytecode VM, same stack, same semantics | the default execution engine |
| **TL** | **leaf compiler** — restricted subset → thread-safe native fn | `--features llvm`; requirement (13) |
| **T2** | general LLVM JIT with invalidation | `--features jit`; last, and optional |
| **T3** | AOT to object files, standalone binary | `--features aot` |

T0 is not throwaway. Two implementations of one language is the differential
method that made ray's A2 safe, and it is available here for free: every
conformance test and every corpus scene runs on T0 and T1 and must agree.

### Crate layout

```
scm/              reader, expander, IR, GC, T0, T1, stdlib.  NO LLVM.
scm-llvm/         IR -> LLVM IR (inkwell), leaf compiler, LLJIT, AOT.
scm-cli/          the `scm` binary: REPL, script runner, `scm build`.
scm-conform/      the conformance runner and its vendored suites.
```

`scm`'s default features stay LLVM-free forever. S0 verified the gate in both
directions — the default crate builds with no LLVM anywhere, and
`--features jit` fails loudly without it. A CI job asserting *both* halves
exists from the first commit that has a `Cargo.toml`.

---

## 6. Requirement (13), and the collision ray's plan names

Goal (a) in ray's plan — a field function render threads can call — is the
capability justification, and its §5 calls the thread-locality collision "the
hardest design question here". `REGISTRY` keeps `thread_local` semantics
deliberately, and the modeler's scene-VM worker depends on that.

**scm's obligation is narrower than the collision suggests.** A field function
is `f64³ → f64`. It does not need the store, the scene, or the VM. What scm
must deliver is a compiled function that is:

- **VM-free** — no interpreter state, no thread-local reachable from it;
- **re-entrant and `Send`/`Sync`** — N rayon workers call it concurrently;
- **allocation-free**, or arena-per-call with no shared heap;
- **restricted by construction** — *pure*: no scene queries, no store access,
  no I/O, no mutation outliving the call. The compiler rejects anything else at
  the point of definition, with a message naming which call did it.

That last point dissolves the collision rather than solving it: a pure leaf
function compiles; one that queries the scene is rejected loudly. The
thread-local store is never reached from a render thread because the subset
cannot express reaching it.

**The restriction is "pure", NOT "primitives-only"** —
[leaf-survey.md](leaf-survey.md) §4 found that a primitives-only compiler
would reject *every texture in ray's corpus*, because `fbm`, `vnoise`,
`hash2`, `fade` and `mix` are authored Janet, not Rust verbs. Five things the
subset must therefore carry, all of them still pure:

1. **Transitive compilation** over authored pure functions, reporting which
   call made a function uncompilable.
2. **Captured immutable data** as a read-only snapshot — the precomputed
   lookup tables (`heights`, `draws`, `ofx/ofy/ofh`) are built during the scene
   build and never written afterwards.
3. Local mutation, bounded loops, conditionals — ordinary SSA work.
4. **Monomorphised higher-order calls** — `map` over a literal 3-list with an
   inline `fn` must inline, not allocate.
5. **A no-heap return convention** for small float vectors; one allocation per
   texel at 4.2M texels is exactly the cost being deleted.

(1) and (2) need design; the rest is ordinary codegen.

**What this does not cover,** and it should be settled in S0 rather than
discovered: a field that genuinely wants scene data — a texture sampling a
loaded image, say. That needs either an immutable snapshot handed to the
compiled function, or ray's store moving off thread-locality. The first is
scm's problem and is tractable; the second is ray's and is not scm's to
promise.

---

## 7. Startup is a design requirement

Measured one day before this plan: Steel v0.8.3 takes **109 ms to run an empty
file, 61 ms of it `Engine::new`, with 0.8% of that executing Scheme and ~99%
recompiling four `include_str!` constants fixed at build time.** The cache that
would have fixed it was written, commented out, and abandoned because a static
interner invalidated the image.

Design that out rather than discovering it:

- The stdlib is **compiled at build time** into a serializable image; boot is a
  deserialize. This constrains the library system and the interner: **no
  global interner state the image cannot reconstruct.**
- **Boot budget: < 10 ms** for `scm -e '(values)'`, in CI from the first
  release build, failing the build on regression.
- The JIT must be **lazy** — S0 measured sub-millisecond per-function warm-up,
  which is cheap on demand and ruinous if done eagerly at boot.

This also matters to ray directly: a 109 ms per-process tax is nothing in a
modeler session and real across `--batch` sweeps and the demo films.

---

## 8. The REPL is a protocol, not a prompt

Requirement (7), with ray as the consumer, means the terminal prompt is **one
client among several** — the others are Emacs over TCP, the modeler's editor
pane, SLIME sends, and the agent pane, all of which exist on the Janet side
today and all of which send forms and want structured answers back. So the
REPL is a library-level API with a wire protocol, and `scm-cli`'s prompt is
the thinnest client of it. Building the loop first and bolting a socket on
later is how the Janet side ended up with an eval channel per consumer.

Six obligations:

1. **Incremental, environment-addressed evaluation** against a persistent
   interactive environment. R7RS libraries are closed by design; the
   interactive environment is explicitly not, and the two must coexist.
2. **Redefinition that takes effect**, including of procedures already
   captured in closures and already compiled. Globals are mutable cells, never
   inlined values; every compiled unit records what it assumed; redefinition
   invalidates and falls back to the VM tier. Cheap to design in, expensive to
   retrofit — and it is why T2 is hard and TL is not (leaf functions are
   explicitly snapshots, recompiled on demand).
3. **Interruptible and bounded execution** — `C-c`, a wall-clock timeout, and
   a fuel budget (requirement 8).
4. **Errors with source spans and a real backtrace** over live frames (§5).
5. **A reader that distinguishes "incomplete" from "wrong"** — multi-line
   entry and the editor panes' paren handling both depend on it. An S1
   decision, not a REPL-stage one.
6. **Introspection** — completion from the global environment, and docstrings
   as *values* rather than a comment convention an external tool scrapes. ray
   generates its API manual from live docstrings via `(curenv)`; the ray-steel
   work found Steel had no runtime docstrings at all.

Stretch goal the explicit stack makes reachable and no candidate offered: a
**break REPL** — stop on error with the frames live, inspect, resume or
restart.

---

## 9. Staging, merged

Sizes in ray's unit (the API plan's T2 = 1.0). Stages marked ◇ are
language-side and unsized in ray's plan; their numbers are estimates.

| | stage | size | exit criterion |
|---|---|---|---|
| **S0** | **The decisions.** §4's calling convention, §3.1's subset, §3.3's GC, §6's reachability. No code. The LLVM spike is banked evidence (§2.1). | 0.2 | this document, amended; x86-64 Linux half of the spike run |
| **S1** | Reader, printer, round trip. Datum syntax, `#(…)`, `[x y z]`, the keyword datatype, comment-depth convention (`Line::Banner`), **incomplete-vs-invalid** reporting. | 0.4 | every saved corpus scene printed through `Form::to_scheme()` reads back to an equal datum; a truncated datum reports *incomplete* |
| **S2** | T0 interpreter: explicit stack, closures, proper tail calls, safepoints. Deliberately before any compiler — it settles semantics cheaply and becomes the oracle. | 0.6 | a 10^8-iteration tail loop is flat in memory (measured); `C-c` interrupts it |
| **S3** ◇ | GC: mark-sweep, roots = explicit stack + handle table, arena fast path. | 0.4 | cyclic structure collected; stress test under a shrunk heap; no leak under `set!` cycles |
| **S4** | **The verb bridge.** `#[verb]`'s shim generator emits Scheme as it emits Janet. Keywords, `&opt`, `&named`, error strings verbatim. | 0.5 | `pyray/tests/conformance.py`-shaped pass through a real host call; `api/scheme/api.md` generated and byte-compared |
| **S5** | `syntax-rules`, the library system, `(scheme base)`, the §1(12) macro set, dynamic variables. | 0.6 ◇ | each surviving ray macro demonstrated; the conformance number reported for the first time |
| **S6** | **The corpus migrates.** ray's §4 script: load with Janet, save through `Form::to_scheme()`. | 0.3 | 191 renderable scenes render identically — compare scene *files*, not image bytes (renders are nondeterministic) |
| **S7** ◇ | **Embedding API.** Safe handles, rooting, host-function registration, errors as `Result`, interrupt/timeout/fuel exposed to the host. | 0.5 | a sample app embeds scm in < 30 lines; **no panic crosses the API** under a fuzzed host-call harness; a runaway script is killable |
| **S8** ◇ | **REPL core** — protocol, interactive environment, redefinition, live-frame backtraces, introspection; terminal client. | 0.6 | a socket client and the terminal client share one API with no divergence |
| **S9** ◇ | The rest of R7RS-small: chars/strings, ports, bytevectors, `eval`, escape continuations, `dynamic-wind`. | 0.6 | conformance ≥ 95% of the declared subset, every failure individually explained |
| **S10** ◇ | Stdlib image; boot budget. | 0.3 | `scm -e '(values)'` < 10 ms in CI on aarch64 macOS (§2.5) |
| **S11** | **TL — the leaf compiler.** §6. The capability that justifies the project. | 0.8 | a user-authored SDF and texture, compiled, called concurrently from rayon workers; the restricted subset rejects non-pure calls with a message naming why |
| **S12** ◇ | T1 bytecode VM. | 0.7 | benchmark suite vs T0; T0/T1 agree on every conformance test and every corpus scene |
| **S13** | **The prelude and the modeler move.** ~19,000 lines plus the prelude. Divisible by file. | 2.0 | this is where "99.6% mechanical" is true or isn't |
| **S14** | **T2 — the general JIT**, with assumption tracking and invalidation. May never be worth starting. | 1.5+ | T1/T2 agree on everything; REPL redefinition invalidates compiled callers |
| **S15** ◇ | T3 AOT + `scm build`. | 0.8 | a hello-world binary runs on aarch64 macOS with no toolchain present; x86-64 Linux owed (§2.5) |

Two ordering notes that are the whole point of the merge:

- **S11 (the leaf compiler) is not last.** It is the capability, it needs no GC
  integration or deopt, and it is what makes the project worth doing at all by
  ray's own §8. The old plan buried it behind the general JIT; that was wrong.
- **S14 (the general JIT) may never run,** and that is a legitimate outcome.
  ray's §8 says it plainly: if S2's interpreter is fast enough, S14 is not
  worth starting — which is a good thing to discover cheaply, and is why S2
  comes before any compiler.

The corpus, the verb bridge and the migration (S4/S6) come early because
requirement (14) means the Janet path must keep working the whole time, and
nothing proves a replacement is verb-for-verb except running the corpus
through it.

---

## 10. Gates

Reuse what exists rather than inventing. Every stage gated on:

- `cargo test` (pure) and `nix develop .#janet -c cargo test --features janet,oidn`
- `cargo build --no-default-features` **on a machine with no LLVM**, plus
  `--features jit` failing loudly without one — both halves, verified in S0
- `scripts/dev/run_corpus.sh` — the 191 renderable scenes, 0 failures
- `pyray/tests/conformance.py` — **the one gate that makes a real call through
  a host.** A0's lesson: seventeen Rust unit tests passed while a generated
  shim silently dropped every option, because each test built the argument
  shape by hand. The Scheme binding needs its own pass of the same shape from
  day one
- both generated manuals byte-identical, and `api/scheme/api.md` joining them
- **language conformance as a number** from S5: vendored `r7rs-tests.scm`
  (Chibi/Larceny) as the primary oracle, written by other people against the
  spec with no knowledge of our implementation, plus the 44-program R7RS
  benchmark suite as a second correctness corpus
- the 6-language embedding benchmark suite as the performance baseline — ECL
  led shader-math, Janet second. Benchmarks must reflect the real workload: a
  procedural generator loop and a **field evaluation**, not scene loading,
  which is not interpreter-bound

Two rules: **every tier runs every test** — a compiler that passes fewer tests
than the interpreter is a bug, not a tradeoff — and **a failing test is either
fixed or individually explained in a file.** A conformance percentage with an
unexplained remainder is a marketing number.

---

## 11. Risks, worst first

1. **Six months before ray can use anything.** ray's §1 states it: the first
   stages produce nothing ray can run while the Janet path must keep working
   anyway. Of the three justifications, two are strategic and only (13) is a
   capability. *Mitigation: S4 and S6 land a working verb bridge and a
   migrated corpus early, so "nothing usable" ends sooner than S13. And if (13)
   can be had another way — a Rust-authored field DSL — this plan is worth
   less than it looks, which is a thing to check in S0, not month five.*
2. **The calling convention decision shapes the corpus.** If keywords and
   optionals land in a form ray's own scenes read badly in, the corpus stops
   being an asset and becomes 219 files of friction. *Mitigation: §4 option 1
   preserves the existing spelling exactly; decide in S0 and write a sample
   scene by hand before committing.*
3. **The leaf compiler's subset may be too small to be interesting.**
   *Resolved, conditionally, by [leaf-survey.md](leaf-survey.md): the subset is
   big enough if it covers §6's five items. Shipped as primitives-only it
   covers one six-line field and nothing else. Nothing in the corpus wants
   scene data from a leaf function, so the purity restriction itself costs
   nothing.*
4. **LLVM as a dependency.** `llvm-sys` is at 231 while S0 validated 211, one
   feature flag per LLVM version, `llvm-config` discovery, a multi-GB
   toolchain. *Mitigation: flake pin; LLVM off by default; the two-sided CI
   gate.*
5. **macOS arm64 JIT specifics.** S0 found LLJIT handled W^X with no `MAP_JIT`
   work — for an *unsigned CLI binary*. A signed, hardened-runtime app, which
   is how the modeler would ship, still needs
   `com.apple.security.cs.allow-jit`. *Shrunk, not closed.*
6. **R7RS-small is bigger than it reads.** Hygiene, the library system,
   Unicode and ports are each weeks. *Mitigation: the conformance number makes
   progress visible from S5, and §2.3 plus §3.1 legitimately remove three
   large pieces from the critical path.*
7. **GC correctness.** Collector bugs are nondeterministic and awful.
   *Mitigation: roots are one explicit structure (§5); shrunk-heap stress in
   CI from S3.*
8. **Inherited boundary hazards**, all three already written in blood in ray's
   `docs/`: a panic cannot unwind out of `extern "C"` and aborts (@7d8c8a1),
   and now in **both** directions since Rust calling compiled code can no
   longer assume a Rust stack; `with_builder` is a `RefCell` that
   double-borrows and **hangs with no panic**, and compiled code calling a
   verb that calls back into script is a new path into it; and the store is
   thread-local on purpose.

---

## 12. Prior art worth reading before writing

- **Chez Scheme** — fast native code *with* real `call/cc` via stack segments.
- **Guile 3** — a bytecode VM with a JIT that keeps the VM stack; the closest
  precedent for §5.
- **Chicken** — CPS and Cheney-on-the-MTA: the road not taken, and why.
- **Chibi-Scheme** — small, readable, R7RS-complete, and the conformance suite.
- **Racket** — sets of scopes, for hygiene.
- **Extempore** — an LLVM-JIT Scheme-family language; the precedent that
  requirement (6) is achievable at all.
- **Steel** — Rust embedding patterns, and a measured catalogue of what to
  avoid (startup, stdlib holes, 292 `unsafe` occurrences in 97.6k lines).

---

## 13. Open questions

**Answered 2026-10-12:** the first consumer is ray and the modeler. S0's LLVM
half has run and passed on aarch64-darwin. The four §3 conflicts are resolved
above.

**Answered by [leaf-survey.md](leaf-survey.md):** (13) is reachable — nothing
in the corpus wants scene data from a leaf function — but the subset must be
"pure", not "primitives-only" (§6). And the Rust-authored field DSL that was
offered as an alternative **already exists**: `field-code` flattens a composed
field to a postfix program the tracer evaluates, and it covers all 16
`add-implicit` call sites. The leaf compiler's field story is a second backend
for an IR that exists, plus the ability to take authored functions into it.
Its *texture* story is the one with a measured number behind it.

**Done 2026-10-12:** ray's §1 was corrected at ray @f3fd4ee — it gains a goal
(d) for texel speed, and §7's S6 and §8's first trigger were updated to match.
Its §2 verb count was corrected from 146 to 87 at ray @f050791.

Still open:

1. **Does scm aim at eventual full R7RS-small conformance,** or is ray's
   dialect the destination? §3.1 is designed so this need not be answered now
   — multi-shot `call/cc` stays reachable rather than forfeited — but it
   decides whether requirement (5), running standalone, means "a Scheme" or
   "ray's language with a CLI".
2. **Is the resumable-tool rewrite worth one-shot continuations?** §3.1 says
   one-shot is cheap and that ray already passes an explicit `k` for
   minibuffer reads. Whether to *use* it — rewriting the begin/preview/commit
   lifecycle as straight-line code — is a modeler question, not a language
   one, and it is the thing that decides whether the middle rung ever gets
   built. Not an S0 decision; worth asking before S8.
3. **`homoiconic.janet`** walks a quoted tuple with `postwalk` and `eval`s the
   result. It was the one ray example with no clear Steel translation. Does scm
   expose the AST as ordinary indexable data, or as opaque syntax objects?
   Sets-of-scopes hygiene pushes toward the latter, which is the choice that
   broke it. Decide at S5, not at S13.
