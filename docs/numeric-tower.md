# The numeric tower: a stage decision

**Status: PROPOSED 2026-10-15. Not decided, not scheduled.** This document
exists because plan §2.3 declined the numeric tower before there was any
evidence either way, and the thing has come back often enough since to
deserve a decision made on purpose rather than by accumulation.

The recommendation, up front: **add unbounded exact integers and claim
`exact-closed`. Do not add exact rationals. Do not add complex numbers.**
The reason rationals are excluded is not cost — it is that excluding them
is a deliberate decision already made and measured, and reversing it would
change `/` for every ray scene.

## 1. What is actually being asked

R7RS leaves all of this optional, which §2.3 established and which is
worth restating because it means **no part of this is a conformance
question**:

> R7RS requires exact integers only "throughout the range of numbers
> permitted as indexes of lists, vectors, bytevectors, and strings";
> beyond that an implementation "may report the violation of an
> implementation restriction". Unbounded exact integers and exact
> rationals are advertised optionally, through the `exact-closed` and
> `ratios` feature identifiers.

scm currently claims neither, and `Value::Fixnum(i64)` says so in the
code. Arithmetic overflow is a **checked error**, not a silent widening
(§4.5) — `prim.rs:97` raises "exact integer overflow (no bignums: the
`exact-closed` feature is absent)". That was the right call and nothing
here argues otherwise: silent widening to a flonum is the behaviour that
loses data without telling you.

So the question is narrow: should the error become a widening to an exact
bignum, and if so how far up the tower.

## 2. Why it keeps coming back

Nine places, and they are **not** nine bugs — that distinction matters for
sizing the value:

| # | where | citation | kind |
|---|---|---|---|
| 1 | `(srfi 128)`'s hash combiner: `(* 31 h)` overflows on a 62-bit hash | [srfi.md](srfi.md) §tranche three | **worked around correctly** |
| 2 | `(srfi 14)`'s `char-set-hash`: `(* 37 h)` over 256 code points | `lib/srfi/14.sld` | **worked around correctly** |
| 3 | `(srfi 13)`'s `string-hash`: same, over a long string | `lib/srfi/13.sld` | **worked around correctly** |
| 4 | `u64?` stops at 2^63−1, not 2^64−1 | [srfi.md](srfi.md) §tranche five | **capability gap** |
| 5 | `make-random-u64-generator` stops at the same place | [srfi.md](srfi.md) §tranche seven | **capability gap** |
| 6 | `make-random-s64-generator`: the intermediate 2^63 has no exact integer, and **its literal does not even read** | [srfi.md](srfi.md) §tranche seven | **capability gap** |
| 7 | `c64`/`c128` vector element types absent | [srfi.md](srfi.md) §tranche five | complex, not integers |
| 8 | `(srfi 151)`: `(expt 2 63)` has no value; the infinite-model `-1` answers worked out | [srfi.md](srfi.md) §tranche three | partly resolved |
| 9 | benchmark literals; Steel's `8/3` silently turned `lorenz` into bignum arithmetic | plan §4.7, §9 notes | **an argument against rationals** |

Read honestly: **three** are hashes where the reduced-modulus code is not
wrong, merely unconventional — a reader who knows the usual idiom has to
be told why this one differs, three times now. **Three** are real
capability gaps, all of them about the top bit of a 64-bit integer. One is
about complex numbers. One is resolved. And one is evidence *against* part
of the tower rather than for it.

That is a weaker case than "it has come up nine times" sounds, and a
clearer one: what is missing is **the 64th bit and the ability to exceed
it**, not a tower.

## 3. Who needs it

**ray: almost nothing.** Scene files do arithmetic on coordinates,
colours and counts. Nothing in the corpus wants an integer above 2^63, and
the one numeric decision that *does* affect every scene — what `/`
returns — is already settled the other way (§5 below). Track B does not
wait on this.

**The general-purpose goal: this is the first thing it hits.** An
embeddable Scheme that raises on `(* 1000000000 1000000000)` will be found
out immediately, and by exactly the audience goal 2 is for. Three of the
nine surfacings above are a library author working around it; the fourth
through sixth are a library that cannot express its own specification.

So this is a **goal-2 stage**, and it should be scheduled on goal 2's
terms, not smuggled in beside a SRFI tranche.

## 4. It is cheaper than it looks, for three measured reasons

This is the part that changes the decision, and all three were checked in
the code rather than assumed.

**The leaf compiler does not care.** `s11-leaf.md`: "**Numbers are `f64`
throughout.** Where T0 keeps an exact integer — a loop counter, `(sqrt
4)`, `(/ 6 2)` — the compiled code holds the same value inexactly." The
199×–406× hot path has *already* erased exactness. Bignums cannot slow it
down because it never sees an exact integer in the first place. The one
place to be careful is the boundary the doc already names, where T0's
`modulo` and `vector-ref` demand exact arguments.

**The bytecode tier has no inline arithmetic to update.** `code.rs`
contains **zero** occurrences of `Fixnum`; `vm.rs` has one. Every
arithmetic operation in T1 goes through the primitives. There is no
add-fixnum opcode with a fast path to extend, which is the usual reason
this work is expensive in a bytecode VM.

**A bignum needs no new value tag.** `Value`'s variants are read by
native code, and the enum says so — `Undefined` is deliberately last "so
every other tag keeps its number (S13's native code reads them)". A
bignum does not have to disturb that: it belongs in the heap as
`Obj::Bignum`, reached through the existing `Value::Obj` tag, beside
`Obj::NumVector` and the rest. The GC already traces `Obj`, so it is a new
variant in one enum and nothing in the tag contract moves.

Taken together: the work is concentrated in `prim.rs`, the reader and the
writer, and does not touch the two tiers that make scm fast.

## 5. Why rationals are excluded, and it is not about cost

This is the part worth getting right, because "numeric tower" invites
doing all of it.

`(/ 8 3)` returns the flonum `2.666…` in scm **on purpose**. plan §4.7:
Janet's `/` is float division and scm matches it, so that no `->f` appears
in authored code and a converted scene computes what the original
computed. The plan's own conversion-hazard table calls this one out as
*safe* precisely because the behaviours agree.

And there is a measurement attached. The benchmark notes record that
Steel's `8/3` **silently turned `lorenz` into bignum arithmetic** — a
numerical program quietly becoming an exact-rational program and losing
its performance characteristics without any diagnostic. That is the
failure mode `ratios` brings, observed in a sibling implementation on this
project's own benchmark suite.

So claiming `ratios` would (a) change `/` for every ray scene, undoing a
deliberate compatibility decision, and (b) reintroduce a hazard this
project has already seen bite. **Rationals are declined on the evidence,
not deferred for budget.** If a caller wants exact division they can have
`numerator`/`denominator` on request later; what they must not get is `/`
changing under them.

**Complex numbers** are declined too, and more simply: the only thing that
ever asked for them was `c64`/`c128` as `(srfi 160)` *element types*,
which is a question about packed storage, not about arithmetic. If those
vectors are ever wanted, they can hold pairs of floats without the
language growing a complex number.

## 6. Where the work actually is

Scoped to **exact integers only**:

- **`Obj::Bignum`** — a sign-magnitude or limb-vector integer in the heap,
  plus GC tracing (one arm) and `equal?`/`eqv?`.
- **`prim.rs` arithmetic.** The central `arith()` helper already funnels
  `+`, `-` and `*` through `i64::checked_*` and raises on `None`; that
  `None` branch becomes "widen". Then the named operations with their own
  edges: `expt` (`prim.rs:525`), `abs` (`:573`), and
  `quotient`/`remainder`/`modulo`, whose `i64::MIN % -1` case the code
  already documents.
- **Normalisation**, which is the thing that makes it correct rather than
  merely working: a bignum result that fits in an `i64` must come back as
  a `Fixnum`, or `eqv?`, `equal-hash` and every `match` on `Value` start
  disagreeing with themselves.
- **Reader and writer** — a literal wider than `i64` must read (today
  2^63's literal does not), and must print back identically.
- **`number->string` / `string->number`**, including radix.
- **Comparisons** across fixnum/bignum/flonum, which is where mixed-exactness
  bugs live.
- **`equal-hash`** must hash a bignum consistently with an equal fixnum.
- **`(srfi 151)` bitwise** on bignums, or an explicit refusal.
- **`(features)`** gains `exact-closed`.
- **The fuzzer** must learn to generate big literals, or it will never
  exercise any of this.
- **The three hashes** (#1–#3 above) can then be written the conventional
  way, and should be, with their modulus comments removed.

What is explicitly *not* in scope: the leaf compiler, T2 codegen, the
bytecode instruction set, and the `Value` tag numbering.

## 7. What must be measured, not hoped

Two gates already exist and are the right ones:

- **The benchmark suite** (53 programs, `cargo test -p scm-conform --test
  benchmarks`). The fixnum path must not regress. This is the real risk:
  an extra branch on every arithmetic operation, or a lost `Copy`, would
  show up here. A result that costs more than a few percent on the
  fixnum-heavy programs is a reason to stop and reconsider the
  representation, not to proceed.
- **The differential fuzzer and the tier agreement.** T0, T1 and T2 must
  agree on bignum arithmetic as they agree on everything else — and T2's
  f64 erasure means the fixture question from `s11-leaf.md` applies here
  too.

Add one: a **round-trip property** over big literals, read → write →
read, since the reader and writer are the two new halves most likely to
disagree.

## 8. The proposed stage

| | stage | size | exit criterion |
|---|---|---|---|
| **S18** | **Unbounded exact integers.** `Obj::Bignum` behind the existing `Value::Obj` tag; `arith()`'s overflow branch widens instead of raising; normalisation back to `Fixnum` whenever the result fits; reader, writer, `number->string`/`string->number`, mixed comparisons, `equal-hash`, `(srfi 151)`. Claim `exact-closed`. Teach the fuzzer big literals. Rewrite the three reduced-modulus hashes conventionally. **Rationals and complex numbers remain declined** — see §5, which is evidence, not budget. | 0.6 | `(* 1000000000 1000000000)` is exact and prints exactly; big literals round-trip; T0/T1/T2 agree under the fuzzer; `exact-closed` is claimed and true; **the benchmark suite does not regress** on the fixnum programs |

0.6 in ray's unit, against S17's 0.7 for linalg and S9's 0.6 for the rest
of R7RS-small. Most of it is `prim.rs` and the reader; none of it is the
tiers.

## 9. What I would not do

- **Do not do this instead of checking Track B.** ray's API refactor was
  "a few days" out on 2026-10-13. If it has landed, converting one real
  scene end-to-end will teach more than this stage will, and survey §8a
  already says those decisions belong to the port.
- **Do not add rationals "while we are in there."** §5.
- **Do not take silent widening to flonum as a shortcut.** The current
  checked error is better than that, and would remain better.
- **Do not claim `exact-closed` before the fuzzer exercises it.** A
  feature identifier is a promise.
