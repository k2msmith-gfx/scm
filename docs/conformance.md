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
forms aborted:   150

tests passed:    432
tests failed:    510
conformance:    45.9% of 942 assertions reached
```

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
| 1 | **231** | **Ports.** `read` (66 assertions, 165 aborted forms), `open-output-string` (16), `open-input-string`, `eof-object?`. This is `(scheme read)` and `(scheme write)` and it is by a wide margin the largest single block |
| 2 | 24 | **Bytevectors** — `bytevector`, `bytevector?`, `bytevector-append`, and the rest |
| 3 | 11 | **`guard`** — exceptions. `%try` exists now as the mechanism (see below) but `guard`/`raise`/`with-exception-handler` do not |
| 4 | ~20 | **Multiple values** — `call-with-values`, `define-values`, `let*-values`, and the 9 `test-values` entries |
| 5 | 7 | **`quasiquote`** — the reader produces it and nothing evaluates it. Small, fundamental, and a model writes it constantly |
| 6 | 6 | **`call/cc`** — escape continuations are the §3.1 rung we committed to |
| 7 | ~45 | **A long tail of absent procedures**, each cheap: `vector-copy!`, `vector-append`, `string-copy!`, `string-foldcase`, `string>=?`, `digit-value`, `list-copy`, `boolean=?`, `real?`, `rational?`, `char-ready?`, … |

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
