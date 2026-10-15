# The SRFI libraries

2026-10-14. Sources in `scm/lib/`, embedded in the binary; test programs in
`scm/tests/srfi/`, gated by `scm/tests/srfi.rs`.

The general-purpose goal needs these: R7RS-small alone is a language, not a
toolkit, and code written for other Schemes imports SRFIs before it does
anything else. `define-library` (S15) made them possible to ship; the
tranches below are the order they were added in.

## What is in

| library | what | how |
|---|---|---|
| `(srfi 1)` | the list library, ~95 procedures | Scheme |
| `(srfi 2)` | `and-let*` | Scheme |
| `(srfi 4)` | homogeneous numeric vectors | **native, unboxed** |
| `(srfi 8)` | `receive` | Scheme |
| `(srfi 26)` | `cut`, `cute` | Scheme |
| `(srfi 27)` | sources of random bits | **native state**, Scheme interface |
| `(srfi 63)` | arrays and shared arrays | **native views**, Scheme prototypes |
| `(srfi 69)` | basic hash tables | Scheme, on **native hash functions** |
| `(srfi 111)` | boxes | Scheme, on records |
| `(srfi 125)` | intermediate hash tables, ~45 procedures | Scheme, same table as `(srfi 69)` |
| `(srfi 128)` | comparators | Scheme |
| `(srfi 132)` | the sort libraries | Scheme, merge sort |
| `(srfi 151)` | bitwise operations | Scheme, on **native bit primitives** |
| `(srfi 160 base)` | `(srfi 4)` under SRFI 160's name | the same bindings |
| `(srfi 160 <type>)` | ~57 operations for each of ten element types | Scheme, written once |
| `(scm test)` | `check`, `check-raises`, `test-failures` | Scheme |
| `(scm hash-table)` | the table `(srfi 69)` and `(srfi 125)` share | Scheme, on **native hash functions** |
| `(scm numvector)` | `(srfi 160)`'s operations, written once | Scheme, on **native unboxed storage** |

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

## Tranche four: the intermediate hash tables

2026-10-14. `(srfi 125)`.

### One table, two interfaces

`(srfi 69)` and `(srfi 125)` are two interfaces onto one data type, so the
table moved into `(scm hash-table)` and each of them now exports a view of
it. That is not only tidier. SRFI 125 says it is compatible with SRFI 69,
and a table made through either has to work through the other, which two
separate implementations could not give. Nothing in SRFI 69's interface
changed.

What 125 adds over 69 is the whole-table work: `fold`, `map`, `map!`,
`map->list`, `for-each`, `prune!`, `find`, `count`, `pop!`, `intern!`, the
two-value `entries`, and the four set operations `union!`,
`intersection!`, `difference!` and `xor!`. Keys are a `(srfi 128)`
comparator rather than a loose pair of procedures, and the deprecated
form that takes a bare equality predicate is accepted too.

### Three things this SRFI does that surprise people

**A table can be immutable, and two constructors give you one.** A table
from `hash-table` or from `hash-table-copy` with no second argument is
immutable, and mutating it raises. `make-hash-table`,
`alist->hash-table`, `hash-table-unfold` and `hash-table-empty-copy` all
give mutable tables. The flag is named `mutable` because it is an opt-in.
Since mutability is fixed when the record is made, an immutable table
cannot be built and then frozen; the core grew a copy constructor that
builds the result immutable from birth, which is also what
`hash-table-copy` needs.

**`hash-table-walk` and `hash-table-for-each` are the same operation with
their arguments in opposite orders.** `for-each` takes the procedure
first, `walk` takes the table first because it is SRFI 69's name and keeps
SRFI 69's order. That is the entire reason both exist, and getting it
wrong is silent: the procedure is simply called with a table.

**`hash-table-fold` reads its own arguments two ways.** It is
`(hash-table-fold table proc seed)`, but when the first argument is a
procedure the arguments are taken in SRFI 69's order instead. The SRFI
asks for this by name, and dispatching on `procedure?` is unambiguous
because a table is never a procedure.

### Deleting while walking

`prune!`, and so `intersection!` and `difference!` with it, cannot delete
during the fold: the walk is standing on the bucket chain that a delete
rewrites. Each of them collects the doomed keys first and deletes
afterwards. `xor!` has the same shape for a different reason -- it
gathers the additions before changing anything, so that a key just added
from the second table is not then pruned as one the two had in common.

## Tranche five: the numeric vector operations

2026-10-14. `(srfi 160)`, which is ten libraries and a base.

### Six hundred procedures, written sixty times

SRFI 160 specifies about sixty operations for *each* of its element types:
`f64vector-map`, `s32vector-map`, `u8vector-map`, and so on down. Written
out literally that is six hundred procedures differing only in a name.

They do not have to be. **A numeric vector carries its own element type**,
which `numvector-tag` reports, so a result vector can be made the same type
as its input without anyone naming the type -- and then each operation is
written once in `(scm numvector)` and each per-type library is aliases onto
it. `f64vector-map` and `s32vector-map` are not two procedures that behave
alike; they are one procedure under two names, so there is one place for a
bug to be and one place to fix it.

Nine operations per type genuinely need telling, because they have no input
vector to ask: the constructors `@vector-unfold`, `-unfold-right`,
`-append`, `-concatenate` and `-append-subvectors`, the conversions
`reverse-list->@vector` and `vector->@vector`, the element predicate `@?`
and the comparator. `(f64vector-append)` with no arguments still has to
produce an f64vector, which is the whole reason that list is not shorter.

So a per-type library is a name list and nine one-line wrappers. That is
boring on purpose: the names are the only thing the ten files disagree
about.

### `u8vector` is the bytevector, and now the generic operations know it

`(srfi 4)` already had `u8vector` *be* R7RS's bytevector, which is right --
there should not be two representations of a vector of octets -- but it
meant `NumTag` had no `u8` and the generic `numvector-*` primitives
refused a bytevector. `(srfi 160 u8)` would then have been the one element
type with no operations, and it is not an obscure one: **an 8-bit image
buffer is exactly a vector of octets**, which is the shape ray cares about
most after `f64`.

So the generic primitives now accept either representation. The heap's
`store_ref`, `store_len` and `store_set` already read and write both, for
SRFI 63's arrays, so the primitives were rewritten over those three and one
helper decides which of the two a value is. `make-numvector` with `'u8`
builds a bytevector, `numvector-tag` of a bytevector answers `u8`, and
`(numvector? bv 'u8)` is true. Nothing in SRFI 4's interface changed.

### Two absences, and one range that is not the SRFI's

**`c64` and `c128` are not implemented.** They are vectors of complex
numbers and plan §2.3 declines the numeric tower, so there is no element to
store. Eight integer types and two float types are here.

**`u64`'s element predicate stops at 2^63-1**, not 2^64-1, because the top
half of the range has no fixnum to compare against. `u64?` is therefore
narrower than the SRFI's, and this is the numeric tower showing up again --
the third time in two tranches.

## Tranche six: the reproducible generator

2026-10-15. `(srfi 27)`, finished.

Half of it was already native, because the survey said so: ray's corpus
mentions `math/random` in six files but `:seed` in seventeen, and
`grow-branches` is replayable only because a seed is threaded through it.
So `random-integer`, `random-real` and `make-random-source` were built at
S5 with the generator as a *value* behind a parameter.

The half that was missing is the half that makes a run reproducible, and
for procedural modelling that is the whole point:

- **`random-source-state-ref` and `-state-set!`** save and restore the
  generator. SplitMix64's state is one word, so the external state is
  `#(splitmix64 n)` with that word as a signed integer, there being no
  bignum to hold it unsigned. It is a datum, so it writes and reads back.
  This is what lets a build regenerate one branch without regenerating
  everything before it.
- **`random-source-pseudo-randomize!`** gives independent substreams
  indexed by a pair, so a tile or a frame can own its own stream and stay
  replayable. The same pair always gives the same stream and it overrides
  whatever seed the source held.
- **`random-source-randomize!`** seeds from the clock, through the mixer
  first because consecutive clock readings differ only in the low bits.
  It is the one operation here that gives up reproducibility, which is why
  it has to be asked for by name.
- **`random-source-make-integers` and `-make-reals`**, the latter honouring
  a `unit` as the resolution.

Two deviations, both deliberate. **`random-integer` and `random-real` take
an optional source**, where the SRFI ties them to `default-random-source`;
scm puts the source in the `current-random-source` parameter so a seed can
be scoped to one build, and a bare call reads that parameter, so SRFI code
works unchanged. **`default-random-source` is the source that parameter
starts with**, so randomizing it does affect a later bare call, unless
something has parameterized the source, in which case the parameter wins.

One correction the SRFI forced: its reals are in the **open** interval and
the native `random-real` gives `[0,1)`. The library therefore builds its
own from a 53-bit integer draw and redraws zero. It cannot simply wrap the
native one, because a library body's definitions are mutually recursive, so
naming `random-real` inside the library that exports it calls itself.

## What is still absent

`(srfi 13)`/`(srfi 130)`
strings; `(srfi 14)` char sets; `(srfi 133)`, the operations on *boxed*
vectors, which is 160's counterpart for the case where the elements are not
numbers; and the control and testing SRFIs. `(srfi 47)` will not be added: it is withdrawn and
`(srfi 63)` supersedes it.
