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

The deviations are §*The deviations*; what is here *beyond* the standard —
the lambda list, keywords, bracket vectors, growable vectors and `format` —
is §*The additions*. Nothing in either changes the meaning of a program that
uses only the standard.

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

## The additions

Five things this language has that R7RS-small does not. Each is here for a
named consumer rather than for taste, each is listed so a host knows what it
is relying on, and none of them changes the meaning of a standard program.

### 1. The lambda list

Because ray's API is keyword-shaped: 113 of its documented verbs take keyword
arguments and 79 take optionals.

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
keyword arguments are order-independent.

`(signature …)` prints **one** of them, and prints the familiar one:
`(add-box size [bevel] :at at :material material)`. What it prints is legal
input that declares the same **call shape** — same arity, same keywords, same
rest — though not the same definition, because a default is not shown.

### 2. Keywords

A first-class type, disjoint from symbols: see *The deviations* §3, since it
is both an addition and the one place the reader departs from R7RS.
`#:at` reads as the same datum and is never printed.

### 3. `[a b c]` is a vector constructor

R7RS reserves `[ ] { }` for future extensions, so this costs no conformance.
Two uses, and they do not collide:

- **a value.** `[a b c]` is a vector whose elements are **evaluated**, where
  `#(a b c)` is a constant: `(let ((x 2)) [x x])` is `#(2 2)` and
  `(let ((x 2)) '#(x x))` is a vector of two symbols. ray's scene files are
  full of `[0 1 0]`, which is where this comes from.
- **grouping.** Where a form expects a list — `let` bindings, `cond` and
  `case` clauses, `guard`, `syntax-rules` rules — a bracket vector reads as
  that list, so `(let ([x 1]) …)` is the same as `(let ((x 1)) …)`. That is
  Racket's and R6RS's house style, and this prelude is written in it.

**A vector always prints `#(…)`**, whichever spelling built it: the flag is
not part of the value, so two `equal?` vectors cannot print differently
depending on how each was born.

### 4. Vectors grow

`(vector-push! v x …)` appends, in place, and returns the vector. R7RS
vectors are fixed-length and the standard has no growable sequence at all,
while ray's scene corpus pushes onto an array 248 times across 48 files.

There is one vector type and it grows; there is no second type and no
`growable-vector?` for a predicate to forget. It is **not** the recommended
idiom — accumulate-and-reverse onto a list is the faster path and the one a
model should write — it is the idiom a converted script already has.

### 5. `format`

`(format "~a ~s~%" x y)` — SRFI 28's three directives (`~a`, `~s`, `~%`)
plus `~d` for a number, `~n` for a newline, `~~` for a tilde, and `~,Nf` for
a fixed precision. Anything else is an error naming the directive, and a
left-over argument is an error too. R7RS has no `format` at all, and a
renderer's scripts print numbers constantly;
the Steel port measured rebuilding it from `number->string` and manual padding
as one of three stdlib holes that turned up everywhere.

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
- **The Janet-compatibility names.** `seq`, `for`, `each`, `repeat`, `inc`
  and `dec` are bound in the prelude and work, but they are **not part of
  the supported language** and are expected to move to an optional library.
  They were added so a converted scene could keep its 253 `seq`s and 174
  `for`s; the port that would have done the converting now converts a scene
  by re-saving it through a printer, so those loops never reach this language
  at all, and the code that does reach it is to read as Scheme — `for-each`,
  `map`, a named `let`. See
  [scene-language-survey.md](scene-language-survey.md) §5 for the reversal.
  Write the standard spellings.
