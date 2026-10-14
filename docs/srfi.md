# The SRFI libraries

2026-10-14. Sources in `scm/lib/`, embedded in the binary; test programs in
`scm/tests/srfi/`, gated by `scm/tests/srfi.rs`.

The general-purpose goal needs these: R7RS-small alone is a language, not a
toolkit, and code written for other Schemes imports SRFIs before it does
anything else. `define-library` (S15) made them possible to ship; this is
the first tranche.

## What is in

| library | what | how |
|---|---|---|
| `(srfi 1)` | the list library, ~95 procedures | Scheme |
| `(srfi 2)` | `and-let*` | Scheme |
| `(srfi 8)` | `receive` | Scheme |
| `(srfi 26)` | `cut`, `cute` | Scheme |
| `(srfi 111)` | boxes | Scheme, on records |
| `(srfi 132)` | the sort libraries | Scheme, merge sort |
| `(scm test)` | `check`, `check-raises`, `test-failures` | Scheme |

Written against each specification rather than copied from its reference
implementation, so every procedure is one this project can test and change.
Each is embedded with `include_str!`, so `scm` needs no library directory
beside it and a `scm build` bundle carries them; **a file of the same name
on `SCM_LIBRARY_PATH` wins**, which is both the override and the loop for
editing one without rebuilding.

`(scm test)` is deliberately the thing the SRFIs are tested *with*: a test
program's value is its list of failures, so `()` is a pass whether a person
reads it (`scm scm/tests/srfi/1.scm`) or `cargo test` asserts it. Every
program runs on **both tiers**, because a library is ordinary Scheme.

## Three decisions

**Linear-update variants are the non-destructive ones.** `append!`, `map!`,
`delete!`, `reverse!` and the rest are their safe twins. The SRFI permits
either, and "either" means a caller cannot rely on reuse -- so the version
that cannot surprise someone who kept a reference is the one to ship.

**`(srfi 1)`'s `reduce` replaces the prelude's, and they differ.** SRFI 1's
is `(reduce f ridentity list)` calling `(f elem acc)`; the prelude's, added
for ray's corpus, is `(reduce f init list)` calling `(f acc elem)`. An
explicit import should win, so it does. `map`, `for-each`, `member`,
`assoc` and `filter` are supersets of the R7RS ones and replace them
harmlessly.

**Re-exporting an imported binding works.** `(srfi 1)` exports `make-list`
and `list-copy`, which it does not define -- they arrive from
`(import (scheme base))`, which in scm is a no-op leaving them in the
globals. The export check now falls back to the imported bindings, which is
what R7RS means by re-export.

## What it found: records were not disjoint

`(srfi 111)`'s box is a container whose entire content is its identity, so
`(equal? (box 1) (box 1))` must be `#f`. It was `#t`, because
`define-record-type` built a **tagged vector** -- which also made
`(vector? a-record)` true, and `(vector? (delay 1))` with it. R7RS §5.5
requires a record type to be disjoint from every other type, and the
prelude's own comment had recorded the leak as something "this
representation cannot close without a heap type of its own".

It has one now: `Obj::Record` and `Obj::RecordType`, with five primitives
under the unchanged `define-record-type` macro. A record is not a vector,
two distinct records are not `equal?`, a field is found by scanning the
type's interned names in Rust rather than in Scheme, and a promise is now a
record of a built-in type -- so `promise?` is no longer true of any
three-element vector whose head is the symbol `promise`.

One thing deliberately unchanged: a field the constructor did not name is
`#f`, not `#<unspecified>`. R7RS says only "unspecified", and plan §4
already decided that an absent value in this language is `#f` so that a
supplied-check is `(if field …)`.

## Next

- Homogeneous numeric vectors -- SRFI 4 or 160 -- which need **native
  unboxed storage** to be worth having, and then `(srfi 63)` arrays over
  them. This is the tranche ray wants: an image or a height field stored as
  boxed flonums costs sixteen bytes and an indirection per sample. SRFI 47
  is withdrawn and superseded by 63, so it is not a target.
- Bitwise `(srfi 151)` and hash tables `(srfi 69)`, `(srfi 125)` with
  comparators `(srfi 128)`: both need Rust primitives.
- `(srfi 13)`/`(srfi 130)` strings, `(srfi 14)` char sets, `(srfi 133)`
  vectors, and the control and testing SRFIs.
- The fuzzer does not generate libraries yet.
