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
forms aborted:    37

tests passed:    934
tests failed:    219
conformance:    81.0% of 1153 assertions reached
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
