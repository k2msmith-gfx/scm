# The first conformance number

2026-10-12. `cargo run --release -p scm-conform -- <suite> <out.md>`

Oracle: **`r7rs-tests.scm` from Chibi-Scheme**, 2,530 lines and 989 `test`
forms, vendored unedited at `scm-conform/suites/`. The point of it is that it
was written by other people against the spec with no knowledge of this
implementation, so the rule is that **the suite is never edited** —
`harness.scm` adapts to the suite, and whatever is missing shows up in the
number.

```
forms:          1191
unreadable:       19
forms aborted:    29

tests passed:   1022
tests failed:    142
conformance:    87.8% of 1164 assertions reached
```

Note the denominator moved. Ports did not only make assertions *pass*, they
made 201 more assertions **reachable** — a form that raised outside a test
used to take all of its tests with it, and aborted forms went 150 → 37. So
"+167 passing" and "+201 reached" are both true and both worth saying.

| | passed | % |
|---|---|---|
| first run | 419 | 44.5% |
| after the three bugs below | 432 | 45.9% |
| after `quasiquote` | 439 | 46.6% |
| after `guard`/`raise` | 457 | 48.5% |
| after ports | 624 | 54.6% |
| after multiple values | 675 | 59.1% |
| after the cheap tail | 899 | 78.0% |
| after range arguments | **934** | **81.0%** |
| after the wrong-answer pass | **1022** | **87.8%** |
| after S9's first batch | **1048** | **89.9%** |
| after records / values / binary ports | **1078** | **92.5%** |
| after `call/cc` and `let-syntax` | **1093** | **93.0%** |
| after the last S9 batch | **1103** | **93.9%** |

`quasiquote` gained exactly the 7 assertions the table below predicted, which
is a useful calibration: the priority list can be trusted to be roughly
right about size as well as about order.

That is the baseline, and a low number is the correct outcome rather than a
grade: the suite's own header says it "assumes full-unicode", which §2.3
explicitly does not claim, and it imports `(scheme complex)`, `(scheme lazy)`,
`(scheme eval)`, `(scheme file)` and `(scheme read)` — none of which exist
yet.

## What it is worth: the number is not the finding

**Of 510 failures, only 22 were wrong answers.** Everything else was absence.
What exists is mostly right; what is missing is missing. That is a much better
position than the reverse and it is not something unit tests could have told
me, because I wrote them.

And it immediately reordered the work. `sort` and `get-in` — the two things I
was about to implement from the survey's checklist — **do not appear in the
failure list at all.**

## Three bugs it found, all mine

1. **`set!` did not resolve renamed identifiers.** `lookup` resolved macro
   aliases and `bind` did not, so a macro expanding to `(set! x …)` on a
   definition-site global failed with the renamed name `x.237`. It took 42
   forms down, including the harness's own `test-begin`. Any real macro that
   mutates a global would have hit it.
2. **`modulo` was wrong for a negative divisor.** `(modulo 13 -4)` gave -1
   where R7RS wants -3: `modulo` takes the sign of the *divisor*, and
   `rem_euclid` is not that operation.
3. **The harness compared inexact numbers exactly**, which is my bug and not
   the implementation's. chibi's `test` compares them approximately and the
   suite depends on it — its expected values are written to ~15 significant
   digits, so `(test 3.14159265358979 (atan 0 -1))` fails on *any*
   implementation under `equal?`. Thirteen of the 22 "wrong answers" were
   this. Fixing it is the harness adapting to the suite, which is the allowed
   direction.

Fixing 1–3 moved the number from 44.5% to 45.9% — which is the honest scale of
it. The value was the bugs, not the percentage.

## What the suite says to build, in its own order of weight

Counted by how many assertions each absence costs:

| | cost | what |
|---|---|---|
| 1 | ~~231~~ | ~~**Ports**~~ — **done**, and it was the cheapest block per assertion of any of them: most of what the suite reaches through a port is the reader and writer that already existed, so this mostly *exposed* S1's work. +167 passing, +201 reachable |
| 2 | 24 | **Bytevectors** — `bytevector`, `bytevector?`, `bytevector-append`, and the rest |
| 3 | ~~11~~ | ~~**`guard`**~~ — **done**, and it gained 18 rather than the 11 predicted: it also unblocked the error-object tests |
| 4 | ~~~20~~ | ~~**Multiple values**~~ — **done**, and it gained 51 rather than the ~20 predicted: `call-with-values` is reached by far more of the suite than the obvious entries |
| 5 | ~~7~~ | ~~**`quasiquote`**~~ — **done**, and it gained exactly the 7 predicted |
| 6 | 6 | **`call/cc`** — escape continuations are the §3.1 rung we committed to |
| 7 | ~~~45~~ | ~~**A long tail of absent procedures**~~ — **done**, ~60 of them, and it gained **259** rather than the 45 predicted. The prediction counted only the assertions that named a missing procedure; it missed that most failures had *several* causes, so filling one often fixed a test counted against another |

### What the tail cost, and what it found

Two real bugs, both of which the suite caught by *hanging the entire run*
rather than by failing a test:

1. **`exact-integer-sqrt` looped forever near `i64::MAX`.** `(r + 1) * (r + 1)`
   overflows and wraps negative, so `while … <= n` never ended. The suite
   tests the boundary.
2. **`write` looped forever on a cycle.** Adding `set-cdr!` made cycles
   constructible for the first time — a consequence S1 predicted and parked —
   and the writer had no cycle detection. R7RS wants datum labels, and *only*
   where a cycle would make the output infinite: `(write (list x x))` on
   shared-but-acyclic structure must still read `((1 2 3) (1 2 3))`, which is
   what separates `write` from `write-shared`. All three now match the suite's
   expected strings.

Plus two smaller ones: the reader did not decode `\x…;` escapes inside
`|…|` symbols, so `|H\x65;llo|` came through as `|Hx65;llo|`; and the harness
compared infinities with `(abs (- a b))`, which is NaN for two infinities, so
every `+inf.0` in the suite failed against itself.

Plus 19 unreadable forms, most of them `1/3`-style exact rationals that §2.3
deliberately does not support — those are *correctly* failing and belong in
the explained remainder, not in the work list.

## `%try`, and what is deliberately not catchable

Counting 989 tests needs an error to be catchable per test, which did not
exist. `%try` is now a VM special form: it pushes a `Catch` frame, and an
error unwinds to the nearest one, restoring any `parameterize` bindings
installed inside it. `(%try expr)` yields `(#t . value)` or `(#f . message)`.

**Interrupt, out-of-fuel and out-of-memory are deliberately not catchable.**
They are the host saying stop, and a script able to swallow them would defeat
requirement 8 — `(%try (spin))` must not be a way to ignore `C-c`. There is a
test.

This is S9's machinery arriving early for the same reason the REPL did: it
unblocks a measurement. `guard` and `raise` will be built on it.

## The wrong-answer pass: 65 → 11, all of them declines

Everything above was bought by *filling absences*. This pass did the opposite:
it took the failures where something existed and gave the **wrong answer**,
because those are the only ones that say the implementation is incorrect
rather than incomplete. Absence is a work item; a wrong answer is a bug.

Finding them needed a change to the harness first. The report counted
failures but did not say what they were, so `failure-lines` now emits one
legible line per failure — section, expression, expected and got. That is the
whole reason this pass was possible at all, and it is worth saying plainly
that six passes had gone by without it.

**65 wrong answers → 11, and all 11 are deliberate absences** — four
`full-unicode`, six `(scheme complex)`, one `call/cc`. There is no remaining
failure in 1,164 assertions where this implementation has the feature and
gets the answer wrong. That is the claim the pass was for, and it is a
stronger one than the percentage.

### Two hygiene bugs, both of which only a stranger's suite would find

1. **A macro-defining macro collapsed its pattern variables.** When an outer
   expansion has already renamed an identifier, the inner `syntax-rules`
   pattern variable arrives renamed — and resolving it before matching
   collapsed it onto the *user's* identifier of the same spelling. So

   ```scheme
   (define-syntax foo
     (syntax-rules ()
       ((foo bar y) (define-syntax bar (syntax-rules () ((bar x) 'y))))))
   (foo bar x)
   (bar 1)
   ```

   gave `1` where it must give `x`. The fix is that pattern variables are
   keyed on the **raw** `SymId`, never the resolved one: a pattern variable is
   a position in a pattern, not a reference to a binding. Every unit test I
   had written passed throughout, because I had never written a macro that
   writes a macro.

2. **`(... template)` must still substitute.** The ellipsis escape says the
   ellipses *inside* have no special meaning; it does not say the template is
   inert. Returning it untouched left pattern variables as literal symbols.
   `instantiate_no_ellipsis` substitutes without expanding.

### A regression I introduced and then had to find

Scheme wants a point in a flonum's mantissa and a sign on its exponent, so I
made the writer force both — and Rust's `Display` never chooses exponent form,
so `5e-324` began printing as 324 decimal digits. The printer now picks `{:e}`
outside `(1e-7 .. 1e21)` and positional form inside. The round-trip test
caught it, which is the argument for the round-trip test.

### The rest, in one line each

- `looks_like_number` now *asks the reader* (`reads_as_number`) instead of
  guessing, so a symbol that would read back as a number is barred. Guessing
  had missed `+i`, `+inf`, `.` and a leading backslash.
- Alternate exponent markers `s/f/d/l` and case-insensitive `inf.0`/`nan.0`.
- `string->number` must consume the **whole** string: `"1 2"` is `#f`.
- `=` on mixed exact/inexact compares mathematically, through `i128` rather
  than through `f64` — `(= 9007199254740992.0 9007199254740993)` is `#f`.
- `modulo`-style range arguments on `utf8->string`, `string->utf8` and
  `write-string`; `read-char` refuses a closed port.
- `bytevector-copy!` copies through a temporary, so an overlapping range
  behaves as if buffered.
- `(scheme process-context)` and `(scheme time)` arrived whole:
  `get-environment-variable`, `command-line`, `exit`, `current-second`,
  `current-jiffy`, `jiffies-per-second`.
- `(scheme file)`: `file-exists?`, `delete-file`, `open-input-file` — raising
  real **file** errors, so `file-error?` distinguishes them. Stubbing that
  would have been a lie dressed as a pass.
- **`features` is honest.** It reports what `cfg!` can prove and claims
  neither `exact-closed`, `ratios`, `full-unicode` nor `exact-complex`. The
  absences are the point: §2.3 does not implement the numeric tower, so
  `(features)` must not say it does. There is a test asserting each of the
  four is *missing*.

### Three reader bugs the suite found by being pedantic

`#;` comments out the next **datum**, and a lone `.` is not a datum — it is a
list's tail marker. Reading it as one made

```scheme
(a #;. b)        ; read as (a b)     -- must be a read error
(#; #;x #;y . z) ; read as (z)       -- must be a read error
(#; #;x . z)     ; read as (z)       -- must be a read error
```

all succeed, each by silently dropping a dot. The fix is one arm in `datum`:
reaching a tail-marker dot where a datum was asked for is invalid. The
neighbours the suite also tests — `(a . #;b c)` → `(a . c)` and
`(a #; #;b c d)` → `(a d)` — still read, which is what makes this a
distinction rather than a blanket refusal.

And one arity bug: `(apply proc arg1 … args)` has the final list in its
signature, so `(apply +)` is an error. It was returning `0`.

### The 11 that remain, and why they are not a work list

| | what |
|---|---|
| 6 | `(scheme complex)` — `+i`, `+inf.0+inf.0i` and friends, which the reader deliberately cannot read. §2.3 declines the tower and `(features)` does not claim `exact-complex`. |
| 4 | `full-unicode` — `digit-value` of an Arabic-Indic or Gujarati digit, and `string-foldcase` of `ß`/`ſ`, which needs the full case-folding tables. `(features)` does not claim `full-unicode`. |
| 1 | `call/cc` — `with-exception-handler` escaping through a continuation. The §3.1 rung we committed to and have not built. |

The 131 remaining failures are absences with names: `(scheme complex)` (27),
`define-record-type` (5), `let-syntax`/`letrec-syntax` (4), `define-values`
(2), bignums in `exact-integer-sqrt` (3), and a tail of hygiene corners
including a `cond` whose `=>` is shadowed by a local binding — the one entry
on that list that is a genuine bug rather than a decline.

## S9, batch 1: the failures that *raised* were mostly bugs

Grouping the 142 remaining failures by cause, rather than reading them in
order, showed something worth the five minutes it took: **fourteen of them
raised an error instead of answering wrongly**, which means the feature
existed and was broken. An absence is a work item; a raise is a bug wearing
an absence's coat.

**+24 assertions, 87.8% → 89.9%**, and only one of the six was really a
missing feature.

| | what it was |
|---|---|
| `string-copy` | **ignored its own range arguments.** R7RS is `(string-copy string [start [end]])`; it took one and refused the rest. Six assertions. `string->vector` and `vector->string` had the same hole, and the range helper they needed already existed — they just never called it |
| `vector-map`, `vector-for-each` | took **one** vector. R7RS takes any number and stops at the shortest, like `map`. Also removed `vector-length*` and `vector-for-each*`, two dead aliases that existed only to clutter completion |
| `numerator` / `denominator` of an inexact | refused, blaming the absent `ratios` feature — **the wrong excuse.** An `f64` is already exactly *p*/2ᵏ, so its rational form needs no rational *type*: double until integral and count. `(denominator 5.5)` is `2.0`. What `ratios` genuinely costs is still next door, where `(denominator (/ 6 4))` cannot be the exact `2` because §4.5 makes `(/ 6 4)` the flonum `1.5` |
| string line continuations | R7RS §6.7 is `\ ⟨intraline whitespace⟩* ⟨line ending⟩ ⟨intraline whitespace⟩*`, and only the middle worked. A backslash *immediately* before a newline was fine; a trailing space after the backslash — which no editor shows and every editor leaves behind — was an "unknown string escape" |
| `\|` in a string | simply missing. Two lines |
| **datum labels on input** | the real find, below |

### The printer could write what its own reader could not read

`#0=` and `#0#` were added to the **writer** at S5, because `set-cdr!` made
cycles constructible and `write` would otherwise spin forever. The reader was
never taught them. So for several stages the printer emitted output its own
reader rejected — a round-trip hole in the middle of a feature of mine, and
exactly the kind of thing only a stranger's suite finds, because I had tested
the one direction I wrote first.

Reading them properly means reading a *cycle*: `#0=(1 . #0#)` refers to
itself before it exists. A placeholder is registered under the label first,
the datum is read, and then every occurrence of the placeholder is replaced
by the datum — which is what makes the result an actual cycle rather than a
structure with a stray marker in it. That substitution walk is **iterative
with a visited set**, because by the time it runs the structure may already
be circular; recursing there would have been the fourth latent abort of
precisely the same kind as the writer, `list?` and `length`.

A label is scoped to its datum, so `#0=(1) #0=(2)` read from one port gives
`((1) (2))` and not a complaint.

### And one genuine feature: `#!fold-case`

A reader *directive*, not a datum — it changes how later symbols read and
produces no value, so it belongs with the things that get skipped rather than
in the `#` dispatch, which has to return something. `|MiXeD|` is deliberately
not folded, which is the whole reason that syntax exists.

## S9 complete: 99.0% of the declared subset, and the 11 that remain

**The denominator matters, and I was quoting the wrong one.** S9's exit
criterion is "conformance ≥ 95% **of the declared subset**, every failure
individually explained". §2.3 declines `(scheme complex)`, bignums and
`full-unicode`, and **61 of the 72 remaining failures are exactly those
declines**:

| | |
|---|---|
| of all assertions reached | 1103 / 1175 = **93.9%** |
| **of the declared subset** | 1103 / 1114 = **99.0%** |

So the criterion is met. Here is the other half of it: all 11.

### Declined, by a decision recorded before they were measured

| | why |
|---|---|
| **61** — `make-rectangular`, `complex?`, `real-part`, `3.0+inf.0i` and the rest; `exact-integer-sqrt` of 2¹¹⁹; `digit-value` of an Arabic-Indic digit; `string-foldcase "Maß"` | §2.3: no numeric tower, no bignums, no `full-unicode`. `(features)` claims none of `exact-closed`, `ratios`, `full-unicode` or `exact-complex`, and there is a test asserting each absence |
| **1** — the R7RS `dynamic-wind` example that re-enters a saved continuation | §3.1 chose the **escape-only** rung of the continuation ladder. Re-entry needs the control stack copied; the explicit stack (§5) makes that possible later without making it necessary now |
| **3** — `(environment '(scheme base))` and `eval` against it | `(scheme eval)`'s first-class environments. **Declined now, explicitly**, rather than left looking like an oversight: scm has one interactive environment, which is what §8 obligation 1 committed to, and a second kind of environment object is a library-system question (S5's remainder) rather than an `eval` question |

### Real, and left for what they are: seven hygiene corners

Each is a genuine R7RS requirement and each is small, but they are *deep*
rather than *wide* — seven assertions between them, in the part of the
expander that took the longest to get right.

| | what |
|---|---|
| `(let ((=> #f)) (cond (#t => 'ok)))` | an **auxiliary keyword** shadowed by a local binding. `cond` must notice that `=>` is a variable here and treat the clause as an ordinary test-and-value |
| `(m k)` wants `bound-identifier=?`, gets `free-identifier=?` | the two notions of identifier equality a macro can ask about. Only one is implemented |
| `count-to-2_` with `_` arguments | `_` in a *pattern* matches without binding; as an *argument* it is an ordinary identifier. The two readings are not separated |
| `sequence3`, `elli-lit-1` | ellipsis corners: a template whose ellipsis has no pattern variable under it, and a rule whose literals interact with one |
| `mad-hatter` / `march-hare.NNNN` | a macro-defining-macro whose renamed identifier escapes its expansion. The sibling of the bug fixed in the wrong-answer pass, and not the same one |
| `(dead-clause)` | an arity error from a `case-lambda`-shaped definition the suite builds in a macro |

These are the only places where scm claims a feature and gets it wrong, and
they are recorded here rather than fixed because the cost is in the expander's
hardest region for seven assertions. If a *real* macro ever trips on one, that
changes the arithmetic.

## The R7RS benchmark suite, first run

2026-10-13. [ecraven/r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks),
57 programs, run the way its own `bench` script runs them —
`scm-conform/bench/run.py`. The plan lists this suite as a second
*correctness* corpus for S12; this run was to learn whether scm can run it at
all, before a bytecode VM exists to compare against.

**Every program failed on the first run, for three reasons that took an
afternoon:** the harness reads its inputs with a bare `(read)` and every
input primitive demanded its port; the reader refused `n/2`, `constant-pi/2`
and `inexact-+1/2` as unsupported rationals when they are identifiers; and
`scheme` opens with `caaar`. All three are fixed, with `(scheme cxr)` now
complete and a lazy stdin port that `scm-cli` installs for a script — lazy
because an eager read of stdin hung `scm -e` under every harness that leaves
stdin open, which is every CI. The conformance number moved from 1103 to
1106 on the way.

Second run, release build, two minutes per program:

| outcome | programs | |
|---|---|---|
| **passed** | 3 | `string` 0.8 s, `ctak` 61 s, `fibc` 113 s |
| **timed out** | 41 | the interpreter, not the language — see below |
| needs file *output* | 1 | `ray`: `call-with-output-file`, no file output port kind yet |
| needs bignums | 3 | `pi`, `chudnovsky`, and `compiler` has a literal `9223372036854775808` — declined, plan §2.3 |
| needs complex numbers | 1 | `mbrotZ`: `-1.0-0.5i` — declined |
| needed `call-with-input-file` | 7 | `sum1 cat tail wc read1 parsing slatex` — added after the run, untested at full size |
| `scheme` needed `complex?` | 1 | added after the run |

So the language covers 52 of 57, and the 5 it does not are the numeric tower
scm declined on purpose plus one missing port kind.

**The 41 timeouts are a speed measurement, not a conformance one.** The
suite's inputs are sized for compiled Schemes: Chibi, an interpreter, needs
73 minutes for the whole suite with its own five-minute cap, and takes 60 s
on `fib:40:5`. scm does `(fib 30)` in 1.25 s, which puts `fib:40:5` near
770 s — **T0 is roughly 13× slower than Chibi** on a call-heavy loop. That is
the number S12's bytecode VM and S13's general tier exist to move, and this
suite is now the instrument.

Two results run the other way and are worth keeping: `fibc`, which is
continuation-heavy, finished in 113 s against Chibi's 296 s, and `string` in
0.8 s against 10.8 s. Escape continuations on an explicit frame stack cost
nothing, as §3.1 argued they would.
