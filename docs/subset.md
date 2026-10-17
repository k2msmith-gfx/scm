# The supported language

**This is the normative statement of what scm implements.** The plan argues
why ([plan.md](plan.md) §3.1, §4), the conformance report accounts for what
the suite says ([conformance.md](conformance.md)), and the README
introduces it; this document is the single place that says *what the
language is*.

Every claim here is checked by `scm/tests/subset.rs`, so a statement that
stops being true fails the build rather than quietly misleading a reader.

## In one sentence

**R7RS-small, less re-entrant continuations and the two optional numeric
features, plus keywords as a distinct type, plus a documented
simplification of how imports bind.**

## The baseline

R7RS-small, as tested by Chibi-Scheme's `r7rs-tests.scm` run unedited:
**1112 assertions pass, 71 fail**, and every failure is accounted for
individually in [conformance.md](conformance.md) rather than summarised as
a percentage.

All of these import and are a no-op, because the standard bindings are the
prelude:

    (scheme base) (scheme write) (scheme read) (scheme char) (scheme cxr)
    (scheme inexact) (scheme lazy) (scheme case-lambda) (scheme eval)
    (scheme file) (scheme time) (scheme process-context) (scheme repl)
    (scheme load) (scheme r5rs) (scheme complex)

**An import of any `(scheme …)` name succeeds**, including a library whose
procedures are absent. `(import (scheme complex))` is accepted and
`make-rectangular` is then an unbound variable. The import is not a
promise; `(features)` and this document are.

Present and exercised by the suite: `define-record-type`, `syntax-rules`
with nested ellipses and literals, `guard` / `raise` /
`raise-continuable` / `with-exception-handler`, `dynamic-wind`,
`parameterize`, `delay` / `force` / `delay-force` / `make-promise`,
`case-lambda`, `let-values` / `define-values`, string and bytevector
ports, `eval` with `interaction-environment`, `call/cc` within the limit
below, and the full character, string, vector, bytevector and list
surfaces.

## The feature identifiers

`(features)` is the machine-readable half of this document and is kept
honest: an identifier is claimed only when it is true.

| claimed | meaning |
|---|---|
| `r7rs` | this language |
| `ieee-float` | inexact numbers are IEEE 754 doubles |
| `exact-closed` | **exact integers are unbounded** |
| `scm` | this implementation |

| not claimed | consequence |
|---|---|
| `ratios` | no exact rationals; `1/2` is a read error and `(/ 8 3)` is a flonum |
| `exact-complex` | no complex numbers; `make-rectangular` is absent |
| `full-unicode` | no Unicode case *folding* in `string-ci=?`, and no Unicode digit values |

The platform identifiers (`posix`, `unix`, `darwin`, `gnu-linux`,
`aarch64`, `x86_64`, `lp64`) are reported from the build.

## The deviations

Four, and each is a decision with evidence behind it rather than unfinished
work.

### 1. Continuations are escape-only

`call/cc` works for **escaping**: early return, a loop `break`,
exception-shaped control, and abandoning frames mid-flight. Invoking a
continuation whose extent has returned **raises** rather than
misbehaving.

This is the one unambiguous non-conformance, and it is in `(scheme base)`.
It has a cost that is recorded rather than hidden: `(srfi 158)`'s
`make-coroutine-generator` cannot be lazy, so it runs its producer eagerly
and raises past a cap instead of hanging.

[plan.md](plan.md) §3.1 sets out the three-rung ladder — escape-only,
one-shot, multi-shot — and why the explicit frame stack makes one-shot
addable later rather than forfeited.

### 2. No exact rationals

`(/ 8 3)` is the flonum `2.666…`, not `8/3`. **This is load-bearing**, not
an omission: it matches the float division of the language scm replaces, so
a converted scene computes what the original computed. A sibling
implementation's `8/3` silently turned a numerical benchmark into
exact-rational arithmetic with no diagnostic, which is the measured reason
not to want the alternative. See [numeric-tower.md](numeric-tower.md) §5.

Exact *integers* are unbounded, with the ceiling below.

### 3. `:at` reads as a keyword

Keywords are a first-class type, disjoint from symbols. R7RS says `:at` is
a symbol, since `:` is a legal identifier initial. 87 of ray's 218 verbs
depend on the convention.

`ReaderOptions::colon_keywords` turns it off, which is how the conformance
suite runs — so this deviation is a reader mode, not a property of the
evaluator.

### 4. Importing copies bindings

A library is an environment frame, and an import copies its exports into
the importer's frame, or into the globals for a program. So a library that
`set!`s its own exported variable **after** it was imported does not reach
the importer.

R7RS makes `set!` on an imported binding an error, and no library in the
wild relies on the other behaviour. Both engines implement the same
simplification, which the differential fuzzer checks on every run. See
[libraries.md](libraries.md).

## The lambda list

One addition beyond R7RS, because ray's API is keyword-shaped: 113 of its
documented verbs take keyword arguments and 79 take optionals.

    formals := sym | ( req* [&opt spec*] [&named spec*] [&keys sym] [. sym] )
    spec    := sym | (sym default) | [sym default] | kw spec

A default **only runs when the argument is absent**, so an explicitly passed
`#f` survives — which is the whole point, since ray documents options that
default to true and treat `#f` as meaningful. An absent argument with no
default is `#f`.

`&opt`, `&named` and `&keys` must appear in that order; out of order is an
error rather than a parameter that happens to be spelled `&opt`.

**Several accepted spellings, one canonical output.** A keyword datum
declares a keyword parameter, which is the spelling a Scheme or Racket reader
knows, and `:at` and `#:at` are the same datum:

```scheme
(define (add-box size #:at (at #(0 0 0)) #:material material) …)
(define (add-box size :at  (at #(0 0 0)) :material  material) …)
(define (add-box size &named (at #(0 0 0)) material) …)
```

All three declare the same procedure, and a keyword parameter does **not**
open a section — required parameters may follow one, as in Racket, because
keyword arguments are order-independent. `(signature …)` prints `&named`
whichever was written.

## The limits

Not deviations — a conforming implementation may have them — but a caller
has to know them.

| limit | value | what happens at it |
|---|---|---|
| exact integer size | 2^20 bits (~315,000 digits) | an error; "unbounded" must not mean "may exhaust the host" |
| bitwise operations | 64 bits wide | a bignum operand is **refused**, not truncated |
| char-sets (`(srfi 14)`) | Latin-1, code points 0–255 | membership past it is `#f`; adding raises |
| `embed::Val` conversion depth | 128 | the value is **truncated**, not refused: the first 128 levels convert and what is beyond them is `Val::Opaque`. Reach the rest with `Rooted` and Scheme |
| `equal-hash` on a cycle | 256 steps | terminates with a partial hash rather than spinning |

A cyclic or very deep structure therefore crosses the boundary as a
*prefix* of itself. That is deliberate — a host logging or inspecting a
value usually wants the top of it — but it means `Val` is not a faithful
copy of arbitrary Scheme data, and a host that needs one should keep the
`Rooted` handle and ask Scheme.

For Unicode-correct character matching, **pass a predicate rather than a
char-set**: every `(srfi 13)` and `(srfi 130)` procedure that takes a
char-set also takes a procedure, and `char-alphabetic?` and its relatives
are correct over the whole range.

## What a host may bound

A script cannot take its host down, and this is the mechanism rather than
an aspiration:

| | |
|---|---|
| `set_fuel` | bounds work; `OutOfFuel` instead of a hang |
| `set_heap_cap` | bounds memory; `OutOfMemory`, reported only **after** a collection has been tried |
| `interrupt_handle` | another thread can stop a script |
| `eval_with_timeout` | bounds wall-clock time |

A caught panic poisons the interpreter rather than pretending the VM is
intelligible. See the README's Rust API section.

## What is deliberately not promised

- **Any particular hash value.** `string-hash`, `equal-hash` and
  `(srfi 13)`'s `string-hash` agree with themselves within a run and with
  nothing else. The SRFI 13 one is deliberately not the native one.
- **The order of `char-set->list`** or of any other SRFI that leaves an
  order unspecified, beyond what each library's own documentation promises.
- **Which engine runs a given form.** T0, T1 and T2 must agree on value,
  output and error message; which one executed is not observable except
  through `(%tier-stats)`.
- **`Val::Opaque`'s contents.** It means "no host representation", and what
  falls into it may change.
- **Performance characteristics**, which are measured in
  [s12-bytecode.md](s12-bytecode.md) and [vs-steel.md](vs-steel.md) rather
  than promised here.
