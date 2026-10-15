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
| `(srfi 4)` | homogeneous numeric vectors | **native, unboxed** |
| `(srfi 8)` | `receive` | Scheme |
| `(srfi 26)` | `cut`, `cute` | Scheme |
| `(srfi 63)` | arrays and shared arrays | **native views**, Scheme prototypes |
| `(srfi 69)` | basic hash tables | Scheme, on **native hash functions** |
| `(srfi 111)` | boxes | Scheme, on records |
| `(srfi 128)` | comparators | Scheme |
| `(srfi 132)` | the sort libraries | Scheme, merge sort |
| `(srfi 151)` | bitwise operations | Scheme, on **native bit primitives** |
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

## Tranche two: numeric vectors and arrays

2026-10-14. `(srfi 4)` and `(srfi 63)`, the pair ray wants: an image, a
height field or a volume is a rectangular block of numbers, and holding
one as boxed values costs both memory and an indirection per sample.

**`(srfi 4)` is native and unboxed.** `Obj::NumVector` holds a `Vec` of the
element type -- `s8`, `s16`, `u16`, `s32`, `u32`, `s64`, `u64`, `f32`,
`f64` -- and `u8vector` *is* R7RS's bytevector, because two
representations of the vector of octets is how they drift apart. Measured,
four million elements resident:

| | |
|---|---|
| `(make-f64vector 4000000 1.5)` | 35 MB |
| `(make-vector 4000000 1.5)` | 65 MB |

Thirty megabytes is exactly eight bytes a sample against sixteen. Note
what this is *not*: an object count, because a flonum is an immediate
`Value` and a boxed vector of them is a single object too. The gain is
bytes and locality.

The reader and printer know the literals: `#f64(1.0 2.5)`, `#s8(-1 2)`,
`#u16(65535)`. `#f32(` and `#f64(` are matched **before** `#f`, or every
float vector would read as the boolean false -- there is a test that says
so.

The type-agnostic operations are *aliases* of the generic primitives rather
than wrappers: `f64vector-ref` **is** `numvector-ref`, which asks the
vector its own type. A hot loop therefore pays nothing for the per-type
name, and `(f64vector-ref some-s32vector i)` returns the element instead of
complaining -- a case SRFI 4 calls "an error", which leaves it
unspecified. The alternative was a Scheme call per element.

**`(srfi 63)` arrays are native views.** `Obj::Array` is dimensions, a
stride per dimension, an offset and a store, with the index arithmetic in
Rust. So `make-shared-array` is a row, a column, a transpose or a
subrectangle that **shares** the store rather than copying it, and writing
through a transpose is visible in the array it came from. `make-shared-array`
solves its mapper instead of calling it per access: the SRFI requires the
mapper to be affine, so the image of the origin is the offset and the image
of each unit step is that dimension's stride -- rank-plus-one calls at
construction, none afterwards. A view that would reach outside the store is
refused when it is made, not when it is read.

`equal?` is extended to arrays as the SRFI requires: same shape and
`equal?` elements, whatever either side's store or strides, so a transpose
compares equal to a materialised copy of itself. That extension is the
reason the array is a native type rather than a record -- R7RS compares
records by identity, and arrays had to compare structurally without
changing that.

Three things recorded rather than hidden:

- **`array-set!` takes the value second**: `(array-set! a obj k …)`, SRFI
  63's order, inherited from SLIB. SRFI 25 puts it last. Getting it
  backwards is silent, so the library says it twice.
- **Five prototype procedures are absent**: `a:floc128b`, `a:floc64b`,
  `a:floc32b` and `a:floc16b` are complex, which plan §2.3 declines, and
  `a:flor128b` and `a:flor16b` want 128- and 16-bit floats, of which there
  are none. Absent rather than silently widened to something else.
  `a:bool` is backed by a general vector, so it holds real `#t`/`#f`
  rather than being bit-packed.
- **SRFI 47 will not be implemented.** It is withdrawn, and SRFI 63
  supersedes it and SRFI 25.

Still open for ray's sake: the **leaf compiler does not know about
numeric vectors**, so a texel function that samples an `f64vector` falls
outside the compiled subset. That is the next thing worth doing for the
array work to pay off in a render, and it is a `scm-llvm` change rather
than a library one.

## Tranche three: bits, hashes and comparators

2026-10-14. `(srfi 151)`, `(srfi 69)` and `(srfi 128)`.

### Bitwise: an `i64` already is two's complement

`bitwise-and`, `-ior`, `-xor`, `-not`, `arithmetic-shift`, `bit-count`,
`integer-length` and `first-set-bit` are native; the other three dozen
operations are the ordinary identities on top of them, in Scheme.

The infinite-precision model this SRFI assumes is free for everything that
fits in a word, because an `i64` *is* two's complement: `(bitwise-and -1
7)` is 7 and `(bitwise-not -1)` is 0 without any special case. Two places
it is not free:

- **A left shift off the end of the word raises.** `(arithmetic-shift 1
  63)` is 2^63, which plan §2.3 has no bignum for, and §4.5 says an
  overflow is an error rather than a silent widening. Bit twiddling is
  exactly where a quiet wrap would be a wrong answer, so it is reported.
- **A right shift saturates at the sign**, so `(arithmetic-shift -1 -100)`
  is -1. That *is* the infinite model's answer and needs no bignum.

`bit-count` of a negative number is the population count of its
complement, which is this SRFI's rule and the one that keeps the answer
finite under the infinite model.

### Hash tables in Scheme, hash functions in Rust

A table is a record holding a vector of buckets, each an association list.
A lookup hashes the key to a bucket and walks it with the table's own
equality; the table doubles when it has more entries than buckets, so the
walk stays short.

**Why not a native table.** A native one would be perhaps five times
faster, but SRFI 69 lets a caller supply *any* equality predicate, and a
native table would have to re-enter the interpreter for every probe -- so
its fast path would be fast only for the four standard equivalences and
its general path slower than this. What is native is where the work
actually is: `equal-hash` walks structure in Rust. A native table for the
standard equivalences is the upgrade if profiling ever asks, and nothing
in this interface would change.

**The one contract**: if two objects are `equal?` they hash alike. So
`equal-hash` walks exactly what `equal?` recurses into -- pairs, vectors,
strings, bytevectors, numeric vectors and arrays -- and hashes everything
else by identity, as `equal?` compares it. It also has to terminate on a
cycle, which `equal?` is required to, so the walk carries a work budget:
the traversal is deterministic, so two equal structures spend the budget
identically and arrive at the same number. A table given `eq?` or `eqv?`
gets the identity hash instead, because a structural hash of a key that is
later mutated would lose an entry `eq?` can still find.

### Comparators, and where the missing bignums showed up

`(srfi 128)` is the four-part bundle -- type test, equality, ordering,
hash -- that `(srfi 125)` and any sorted container take. The default total
order across types is the SRFI's own: the empty list, pairs, booleans,
characters, strings, symbols, numbers, vectors, bytevectors, then
everything else, which is unordered among itself.

Two decisions:

- **`make-eq-comparator` and `make-eqv-comparator` are unordered.** A
  comparator's four parts have to agree, and the default order is
  consistent with `equal?` rather than with identity -- two distinct but
  equal strings are unordered by it and not `eq?`. Offering an ordering
  there would be offering a contradiction.
- **`comparator-register-default!` is absent.** It mutates a global
  ordering from anywhere in a program, and nothing here needs it.

And a bug worth recording, because it is the numeric tower showing up
where nobody looks for it. A combining hash is conventionally
`(+ (* 31 h) x)`, and with an unreduced 62-bit hash that **overflows**,
which here raises rather than wrapping. Every step is now reduced modulo
2^25 -- `(hash-bound)` -- first, so the multiply stays nowhere near the
word. In a Scheme with bignums the same code would have silently grown
arbitrarily large integers instead; in one that wraps, it would have
wrapped. This implementation does neither, so the arithmetic had to be
written to fit.

## What is still absent

`(srfi 125)`, the intermediate hash tables, which are a large API over
`(srfi 128)` and `(srfi 69)`'s machinery; `(srfi 13)`/`(srfi 130)`
strings; `(srfi 14)` char sets; `(srfi 133)` vectors and `(srfi 160)`'s
per-type numeric-vector operations -- of those two, 160 is the one that
matters for graphics, since 133 operates on boxed vectors; and the control
and testing SRFIs. `(srfi 47)` will not be added: it is withdrawn and
`(srfi 63)` supersedes it.
