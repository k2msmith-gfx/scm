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
| `(srfi 42)` | eager comprehensions | Scheme, `syntax-rules` throughout |
| `(srfi 63)` | arrays and shared arrays | **native views**, Scheme prototypes |
| `(srfi 69)` | basic hash tables | Scheme, on **native hash functions** |
| `(srfi 111)` | boxes | Scheme, on records |
| `(srfi 125)` | intermediate hash tables, ~45 procedures | Scheme, same table as `(srfi 69)` |
| `(srfi 128)` | comparators | Scheme |
| `(srfi 132)` | the sort libraries | Scheme, merge sort |
| `(srfi 133)` | vector operations on boxed elements, 42 procedures | Scheme; **16 re-exported from the prelude** |
| `(srfi 14)` | char-sets, 62 bindings | Scheme, on a **native bytevector**; Latin-1 |
| `(srfi 13)` | the string library, ~87 procedures | Scheme, on `(srfi 14)` |
| `(srfi 130)` | the same, with cursors — 59 bindings | Scheme, on `(srfi 13)`; **cursors are indices** |
| `(srfi 151)` | bitwise operations | Scheme, on **native bit primitives** |
| `(srfi 158)` | generators and accumulators | Scheme |
| `(srfi 194)` | random data generators, 30 of 32 | Scheme, on `(srfi 27)` |
| `(srfi 204)` | `match` and its five derived forms — **withdrawn SRFI**, see the tranche below | Scheme, `syntax-rules` throughout |
| `(srfi 160 base)` | `(srfi 4)` under SRFI 160's name | the same bindings |
| `(srfi 160 <type>)` | ~57 operations for each of ten element types | Scheme, written once |
| `(scm test)` | `check`, `check-raises`, `test-failures` | Scheme |
| `(scm hash-table)` | the table `(srfi 69)` and `(srfi 125)` share | Scheme, on **native hash functions** |
| `(scm numvector)` | `(srfi 160)`'s operations, written once | Scheme, on **native unboxed storage** |
| `(scm linalg)` | vec3/vec4/mat4/quaternion — **not an SRFI**, see [linalg.md](linalg.md) | Scheme, two layers |
| `(scm noise)` | value/Perlin/simplex/Worley and fbm — **not an SRFI**, see [noise.md](noise.md) | Scheme, all leaf-compilable |

Written against each specification rather than copied from its reference
implementation, so every procedure is one this project can test and change.
Where a specification is genuinely silent the reference implementation is
read to settle the question rather than guessed at, and the three places
that happened in `(srfi 133)` are named below.
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

## Tranche seven: the samplers a renderer needs

2026-10-15. `(srfi 194)`, 30 of its 32 procedures.

This is the SRFI that earns its place in a renderer. Sampling a direction
on a sphere is the inner loop of a path tracer and scattering instances
over a surface is the inner loop of a modeller. Both are easy to get
subtly wrong, and a biased sampler is a faint wrong answer rather than a
crash.

**The specification was fetched rather than recalled.** Writing `(srfi
125)` from memory got the mutability defaults uncertain, and this library
has 32 entry points, so the surface and the semantics came from the
document. That caught `make-bernoulli-generator` yielding **1 and 0**
rather than booleans, which matters because a run of them has to sum to a
binomial variate, and `make-random-real-generator`'s interval being
**closed** where `(srfi 27)`'s is open.

### Two places where the obvious implementation is wrong

**A direction is normalised gaussians, not normalised uniforms.** Taking
three uniforms and dividing by the length concentrates points toward the
cube's corners. Every length check still passes, so the test asserts the
property that actually distinguishes them: for a sphere in 3-space the
z-component is uniform on [-1,1], by Archimedes, so ten equal bands each
hold a tenth of the points. The naive version fails that and nothing else.

**A point inside a ball scales its radius by u^(1/n), not by u.** Volume
grows as r^n, so a linear radius piles the points at the centre. The test
checks that an eighth of the points in three dimensions fall inside half
the radius rather than half of them.

### Poisson in log space, and three linear-time generators

`make-poisson-generator` sums exponential interarrivals rather than
multiplying uniforms. The product form needs `exp(-L)`, which underflows
to zero for a mean past about 745 and would then loop forever. The log
form is exact at any mean, at a cost linear in it, and the test takes 200
draws at a mean of 900 to prove the point.

Three generators are linear where a cleverer method exists, and this is a
deliberate trade for exactness and for code anyone can check:
`make-binomial-generator` sums n Bernoulli draws, and
`make-categorical-generator` and `make-zipf-generator` build a cumulative
table once and then binary-search it, which makes Zipf exact for any `s`
and any `q` where the usual rejection methods assume `q` is zero.

### What is not here

`make-random-rectangular-generator` and `make-random-polar-generator`
yield **complex** numbers and there are none (plan §2.3), so 30 of 32.
`make-random-u64-generator` stops at 2^63-1 for the same reason the rest
of the numeric tower keeps surfacing. `make-random-s64-generator` does
span the full signed word, built in two halves because the unsigned
intermediate 2^63 has no exact integer to pass through and its literal
does not even read.

One judgement call recorded in the library: the specification says a
random string's length is bounded by the *string's* length, which would
leave the `k` argument with nothing to do, so `k` bounds it.

## Tranche eight: comprehensions, and two decisions that nearly collided

2026-10-15. `(srfi 42)`.

The standard answer to Janet's `seq`, which the scene-language survey
counted 253 times as the second most common non-trivial form. A grid of
instances, a lattice, a swept profile: all of them are
`(list-ec (: i n) …)`.

### The collision, and why nothing had to change

Every generator in this SRFI is spelled `:list`, `:range`, `:while`. Under
plan §4 those read as **keywords**, not identifiers. A keyword cannot be a
macro name, and `syntax-rules` rejects one as a literal. Two deliberate
decisions, pointing straight at each other.

Neither had to move. A generator is only ever a *qualifier* inside a
comprehension, and **a keyword in a pattern already matches as a
constant** — so the comprehension macros recognise the generator forms
directly and nothing named `:list` is bound anywhere. No reader change, no
`syntax-rules` change, and the test file asserts that `(keyword? ':list)`
is still true afterwards.

Three names could not survive, because they are used in *operator*
position where a keyword is an error. `:generator-proc` is **absent**:
turning a comprehension inside out into a pull-driven procedure also wants
a re-entrant continuation, and plan §3.1 makes continuations escape-only.
`:-dispatch-ref` and `:-dispatch-set!` are spelled **`dispatch-ref`** and
**`dispatch-set!`**, same procedures, a spelling that reads as an
identifier.

### One normal form, because of `:parallel`

Every generator normalises to

    (%loop (outer …) ((var init step) …) continue? (inner …))

which is a `do` loop in pieces. That indirection exists for one reason:
`:parallel` runs generators in **lockstep**, and lockstep is *appending*
the pieces of each generator, where nesting two loops could only give the
cross product. Generators hand their normal form to a continuation
*macro*, since a macro cannot return a value.

`:while` and `:until` wrap their generator's loop in an escape
continuation, because the guard has to sit inside the per-iteration
bindings — the test reads the variable the generator binds — and must end
only that loop, leaving outer generators running. The test asserts exactly
that, and that `:until` includes the iteration where its test first holds
while `:while` excludes it.

### Two bugs worth recording

**`:` is a plain symbol where every other generator head is a keyword.**
Undeclared, a symbol in a pattern is a pattern *variable*, so the rule for
`:` matched every generator form and quietly rewrote `:parallel` into a
dispatch. It has to be a declared literal; the keyword-headed rules do not.

**The index rule belongs in the normaliser, not the walker.** `(v (index
i))` is handled by one rule that serves every generator, but `:parallel`
normalises its own sub-generators without going back through the walker,
so an index inside a `:parallel` reached the `:list` rule intact and became
a `let` binding whose variable was a list.

Early exit is tested by counting iterations rather than by trusting the
answer: `any?-ec` over 100 values must call its test four times to find
`(> i 2)`, and `first-ec` once. An unbounded `:integers` generator proves
the point, since the comprehension could not terminate otherwise.

## Tranche nine: generators, and the consumer the earlier tranches lacked

2026-10-15. `(srfi 158)`.

A generator is a thunk that yields its next value and then the
end-of-file object for ever after. That is the whole protocol, which is
why two earlier libraries were already producing generators with nothing
to consume them: `(srfi 160)` ships a `make-@vector-generator` for each of
ten element types, and every one of `(srfi 194)`'s samplers is a
generator. The test file asserts those pipelines rather than only the
library in isolation.

For procedural work the point is that a pipeline costs no intermediate
lists. Scattering points, filtering them against a surface and keeping the
first few that pass should not build the discarded million first, and
pulling one value at a time means it does not. The test pulls from a
hundred thousand and keeps four.

### `make-coroutine-generator` is eager, and capped

The SRFI's version suspends the producer at each `yield` and resumes it on
the next call, which needs a **re-entrant** continuation. Plan §3.1 makes
continuations escape-only, so it cannot be done.

What is here runs the producer to completion when the generator is *made*,
collects what it yields, and hands the values out. Two real consequences.
Side effects happen at construction rather than at consumption. And a
producer that never returns cannot work — so rather than hanging, it
**raises** once it has yielded `%coroutine-limit` values, with a message
saying why and what to use instead. A hang with no output is the worst
failure to debug, so it is converted into one that explains itself. The
limit is a parameter, so a program that genuinely needs more can say so.

`make-for-each-generator` is built on it and is unaffected, because a
`for-each` over a collection always terminates.

### What the tests insist on

Three properties that a working-looking implementation can miss.
**Exhaustion is permanent**, so a drained generator keeps returning the
end-of-file object rather than restarting. **`gdelete-neighbor-dups`
removes only consecutive duplicates**, which the test pins by keeping a
value that recurs after a gap. And **`generator-find` and `generator-any`
stop rather than drain**, checked by counting calls against an unbounded
range generator, which could not terminate otherwise.

## Tranche ten: the boxed vectors, and sixteen procedures not written

2026-10-15. `(srfi 133)`.

This is `(srfi 160)`'s counterpart for the case where the elements are not
numbers: the same surface, over ordinary boxed vectors. Forty-two
procedures, of which **twenty-six are written here and sixteen are not**.

### Re-exporting is the implementation

Sixteen of the forty-two are R7RS base procedures: `make-vector`,
`vector`, `vector?`, `vector-ref`, `vector-length`, `vector-set!`,
`vector-copy`, `vector-copy!`, `vector-fill!`, `vector-append`,
`vector->list`, `list->vector`, `string->vector`, `vector->string`,
`vector-map` and `vector-for-each`. The prelude already defines every one
of them to this SRFI's specification, optional `start`/`end` arguments
included, and `vector-map`/`vector-for-each` already take any number of
vectors and stop at the shortest.

So the library defines none of them. `(export vector-ref)` with no local
definition falls back to the global binding — the case
[libraries.md](libraries.md) built the fallback for — and importing
`(srfi 133)` supplies all forty-two. The alternative was sixteen
reimplementations whose only possible contribution is drifting from the
ones the rest of the system uses. What the test file checks instead is
that the import really supplies them, with the optional arguments
present: a fallback that silently did not fire would otherwise look
exactly like success.

### Three argument orders, each pinned by a test that would fail reversed

This SRFI is unusually easy to implement backwards, in three places, and
each one is silent — the code runs and returns a plausible wrong answer.

**`kons` takes the state first.** `(vector-fold kons knil vec)` applies
`(kons state elt)`, which is the opposite of SRFI 1's `fold`, where it is
`(f elem acc)`. Both libraries are shipped here, so the two orders are
live in the same system. The test folds with `cons` and asserts `(3 2 1)`
— the order that comes out the other way round is `(1 2 3)`, which is
exactly what a reader would expect of a left fold and exactly what this
SRFI does not do. `vector-fold-right` visits right to left but still
takes the state first.

**`cmp` takes the element first.** `vector-binary-search` applies
`(cmp element value)`, and with the arguments swapped every sign inverts,
so the search walks away from the target and returns `#f`. The tests
search for the first, middle and last elements of a sorted vector with
`(lambda (a b) (- a b))`; all three would be `#f` under the other order,
and a single "absent" case would not have caught it.

**`vector-unfold-right` fills from the end.** The first element `f`
yields lands in the *last* slot, so the obvious test
(`(values x (+ x 1))` from 0) gives `#(4 3 2 1 0)` and not
`#(0 1 2 3 4)`. The SRFI's own idiom for it is reversing a vector, which
is also asserted.

### Three things the prose does not say, settled from the reference

Unlike `(srfi 125)`, this SRFI has a reference implementation reachable
from here — `gh api repos/scheme-requests-for-implementation/srfi-133`
— and three questions the specification leaves open were settled by
reading it rather than by choosing.

**An empty vector gives `#f` for `vector-any` and `#t` for
`vector-every`.** The prose defines both in terms of "the first" and "the
last" set of elements and says nothing about there being none. The
reference's loops open with `(and (not (= i len)) …)` and
`(or (= i len) …)` respectively, which answers it. Both are asserted here,
because they are the cases a caller reaches by accident.

**`vector-unfold` accepts zero seeds**, and then `f` is applied to the
index alone and returns the element alone, rather than returning one
value by a different protocol. The signature's `initial-seed ...` admits
zero and the reference has a separate `tabulate!` case that does this.

**`vector-partition`'s second value** is described by the specification as
the number of elements satisfying the predicate and by the reference's
own comment as the index of the leftmost element that does not. These are
the same number, since the satisfying elements come first — the test
asserts both readings of it, including that indexing the result at the
count lands on the first failing element.

### Two places this deliberately differs

`vector-partition` makes **one** pass, applying the predicate exactly once
per element. The reference applies it twice, once through its internal
`vector-count`, which any predicate with a side effect would see and
which nothing in the specification asks for.

`vector-binary-search` accepts the optional `start`/`end` that the
reference implementation takes and the signature in the specification's
body omits — a superset, and the range form is tested.

`vector-reverse-copy!` copies from a snapshot of the source range, so
`to` and `from` may be the same vector with overlapping ranges. That is
what reversing a slice in place *is*, the SRFI permits it, and the
element-at-a-time version quietly corrupts the overlap. Two overlapping
cases are asserted, one of them shifted so that a naive implementation
gives a visibly wrong answer rather than a coincidentally right one.

## Tranche eleven: char-sets, and a bound chosen three times over

2026-10-15. `(srfi 14)`.

This one was not next in the queue. `(srfi 13)` was, and `(srfi 13)` turns
out to need char-sets for twelve of its procedures — including
`string-trim`, whose *default* criterion is `char-set:whitespace`, so it
reaches for SRFI 14 even when the caller passes nothing. So 14 came first.

### Latin-1, with three independent reasons

Char-sets here hold code points 0 to 255. That is a real restriction and
worth being able to defend, which it is, three times over:

- **The SRFI says so.** `char-set:full` and `char-set-complement` are
  defined relative to "the implementation's character range", and the
  specification warns that an implementation with a Unicode-wide
  character type will find those operations expensive.
- **scm already says so.** The `full-unicode` feature is unclaimed
  ([plan §2.2](plan.md)), so the declared character range is already
  narrower than the `char` type.
- **The reference implementation says so**, in its own words: "Is Latin-1
  specific. Would certainly have to be rewritten for Unicode."

A set is a 256-byte bytevector, one byte per code point — the reference's
own structure with the improvement its notes ask for ("Should be rewritten
to use bit strings or byte vecs"), and scm's bytevectors are native and
unboxed.

### The bound has three different answers

The interesting part of a bound is what happens at it, and the three cases
want three different answers rather than one rule:

| operation on a character past 255 | answer | why |
|---|---|---|
| `char-set-contains?` | `#f` | it is not a member; that is not an error |
| `char-set-adjoin`, `char-set`, `string->char-set` | **raises** | it cannot be stored, and dropping it silently is worse |
| `char-set-delete` | no-op | it was not there, and the result is right either way |

`ucs-range->char-set` is the fourth case and the SRFI already decided it:
a range running past the bound is clipped **silently**, unless the
optional `error?` argument asks to be told.

The cost is that scm's characters really are Unicode scalar values, so
`(char-set-contains? char-set:letter #\λ)` is `#f` while
`(char-alphabetic? #\λ)` is `#t`. The way out is in the library's own
surface: every `(srfi 13)` procedure that takes a char-set takes a
*predicate* instead, and scm's `char-alphabetic?` and friends are
Unicode-correct over the whole range. **Pass a predicate when Unicode
matters.**

### The `!` variants are non-destructive, which is what makes the constants safe

Sixteen of the 62 bindings are linear-update variants. They are all the
non-destructive versions, which is what `(srfi 1)` does here and for the
same reason: the SRFI permits an implementation to reuse its argument's
storage or not, and "not" cannot surprise a caller who kept a reference.

Here it buys something specific. The reference implementation warns about
a hazard it cannot fix — "it would be very, very bad if a client's buggy
code corrupted these constants" — because `(char-set-adjoin! char-set:digit
#\z)` would mutate a shared global for the rest of the program. With
non-destructive variants that cannot happen, and the test asserts it
directly: the set comes back with eleven characters and `char-set:digit`
still has ten.

### A third fold order

`char-set-fold` applies `(kons char state)` — the **character first**.
This project now ships three folds with three different orders, which is
worth writing down in one place:

| | order |
|---|---|
| `(srfi 1)` `fold` | `(f elem acc)` |
| `(srfi 133)` `vector-fold` | `(kons state elt)` |
| `(srfi 14)` `char-set-fold` | `(kons char state)` |

`char-set-every` is a second asymmetry: it answers `#t`, where
`(srfi 133)`'s `vector-every` answers the last value the predicate gave.
`char-set-any` does answer the predicate's value. Both agree with 133 on
the empty case — `#t` and `#f`.

`char-set-unfold`'s first two arguments are a third: their order was
reversed in the SRFI's original signature and **corrected by erratum**,
the examples and the reference having always used the corrected one. `p`
is the *stop* predicate, so reading it as "keep going" produces the empty
set rather than an error.

### Where the standard sets disagree with scm's own predicates

The seventeen standard sets are the reference implementation's Latin-1
data copied verbatim, which for Latin-1 *is* the specification of what
they hold. Deriving them from scm's native predicates instead would give
different sets — so the two disagree, inside the representable range, in
exactly three places, and the tests pin each:

| set | native predicate | disagrees on |
|---|---|---|
| `char-set:digit` | `char-numeric?` | `² ³ ¹ ¼ ½ ¾` |
| `char-set:lower-case` | `char-lower-case?` | `ª º` |
| `char-set:whitespace` | `char-whitespace?` | U+0085 `NEL` |

None is a bug in either party. `char-set:digit` is the *decimal* digits
where Rust's `is_numeric` is Unicode `Nd`+`Nl`+`No`. `char-lower-case?`
follows the Unicode Lowercase property, which picks up the two ordinal
indicators through `Other_Lowercase`, while the reference files them under
`char-set:letter` only. And `char-whitespace?` counts NEL, a C1 control
the reference's list omits.

That last one was tempting to "fix", and the reason not to is instructive:
`char-set:printing` is defined as whitespace union graphic, so adding NEL
to make the predicates agree would put a control character among the
printing characters. The verbatim data is kept and the divergence is
tested instead. `char-set:letter` and `char-set:upper-case` agree with
their predicates exactly, which the tests also assert — so a change on
either side shows up here rather than as a surprise in `(srfi 13)`.

## Tranche twelve: the string library, and a SRFI that disagrees with itself

2026-10-15. `(srfi 13)`.

The library `(srfi 14)` was brought forward for. It is the largest here —
about 87 procedures — and the oldest in spirit: it predates R7RS, which
shows up in three separate ways.

### It needs char-sets, and the way past them is a predicate

Twelve procedures take a "char/char-set/pred" criterion, and
`string-trim` *defaults* to `char-set:whitespace`, so the dependency is
not optional even for a caller who passes nothing. One internal helper,
`%pred`, turns any of the three into a predicate, and it is the whole of
the SRFI 14 dependency.

Because char-sets here are Latin-1, that helper is also the escape hatch:
a **predicate criterion never goes through SRFI 14 at all**, so
`(string-index "aλb" char-alphabetic? 1 2)` finds the λ where
`(string-index "aλb" char-set:letter 1 2)` cannot. Both are asserted, side
by side, so the trade is visible rather than folklore.

### Two names collide with R7RS, and narrowing them would be a regression

`string-map` and `string-for-each` exist in both standards with different
shapes: R7RS takes any number of strings and stops at the shortest, SRFI
13 takes one string and an optional `[start end]`. Defining the SRFI's
shape would mean that merely importing this library broke ordinary R7RS
code.

They are therefore **supersets of both shapes**, told apart by what
follows the first string — another string means R7RS, an integer means
SRFI 13. `(srfi 1)` does exactly this for `map` and `for-each`, and the
tests assert both shapes of both procedures.

### It disagrees with itself about where the criterion goes

This is the thing most likely to bite, and it is not a defect in the
implementation — both orders are as specified:

| criterion first | string first |
|---|---|
| `string-every` | `string-index` |
| `string-any` | `string-index-right` |
| `string-filter` | `string-skip` |
| `string-delete` | `string-skip-right` |
| | `string-count` |
| | `string-trim`, `-right`, `-both` |
| | `string-tokenize` |

So it is `(string-filter pred s)` but `(string-count s pred)`. The later
string SRFIs regularised this; here every one of them is pinned by a test,
and one test puts the two orders side by side.

### `string-pad` truncates on the side it pads

`string-pad` pads on the left to reach the target length — and when the
string is *already* longer, it keeps the **rightmost** characters.
`(string-pad "hello" 3)` is `"llo"`, not `"hel"`. `string-pad-right` is
the mirror image. Getting this backwards silently keeps the wrong half of
the string, which no type error will catch, so both directions are
asserted.

### The comparisons answer an index, and the spec's special case is not one

`string<`, `string>`, `string<>` and the rest answer the index where the
two strings diverge rather than `#t` — a true value that also tells you
where. Only `string=` answers `#t`, having nothing useful to point at.

The specification says `string-compare` applies its procedure to the
mismatch index, and that "in the case of equality, this is `end1`". It is
tempting to implement that as a separate rule, and the first version here
did; it is wrong. The index is **always** `start1 + match`, one past the
common prefix, and the equality case is the same rule seen from a
different angle: when the ranges are equal the common prefix is all of
range 1, so `start1 + match` already *is* `end1`.

Treating it as a special case gets one asymmetric case wrong — when range
2 is a proper prefix of range 1, as in `(string> "abc" "ab")`, it reports
`end1` (3) where the strings actually run out at 2. That was caught by a
test rather than by reading, which is the argument for writing the test
before trusting the code: both directions of it are now pinned.

### Three smaller things

`string-unfold` has six arguments and the specification states the
identity it must satisfy, which is worth quoting as the implementation's
comment does: `base` is the leftmost part of the result and `make-final`,
applied to the terminal seed, is the rightmost. `string-unfold-right` is
the mirror image in both respects — `base` on the *right* — and that is
what makes it the idiom for reversing a string.

`string-join`'s `infix` and `strict-infix` grammars differ in exactly one
place, the empty list, where `infix` answers `""` and `strict-infix`
raises. That is the whole reason the second one exists: `infix` is not
invertible there.

The `/shared` procedures — `substring/shared`, `string-append/shared`,
`string-concatenate/shared`, `string-concatenate-reverse/shared` — are
*permitted* to share storage, not required to. scm's strings are
`Vec<char>` with no shared-slice representation, so each copies, which
conforms; a caller relying on `eq?`-ness would be relying on something the
SRFI never promised.

And unlike `(srfi 1)` and `(srfi 14)`, **the `!` procedures here really do
mutate**. Those two libraries' `!` procedures are *linear-update*
variants, which the SRFI explicitly lets an implementation make pure.
`string-upcase!`, `string-reverse!`, `string-map!` and `string-xcopy!` are
specified as mutators returning an unspecified value — a caller has no
other way to get the result — so they mutate, and the tests check that
they do.

`string-lib-internals` — `string-parse-start+end`, `let-string-start+end`,
`make-kmp-restart-vector`, `kmp-step`, `string-kmp-partial-search` — is a
**separate library** in the SRFI and is not provided. Its purpose is to
let *other* libraries parse SRFI 13-style argument lists; nothing here
needs to export that, and `string-contains` uses a plain scan, KMP being
worth its restart vector only for long self-similar patterns.

## Tranche thirteen: cursors, and an abstraction that costs nothing here

2026-10-15. `(srfi 130)`.

SRFI 13 again, with three changes of direction: subranges are given by
*cursors* rather than indices, the searches answer a cursor instead of
`#f`, and the comparison, case-mapping and mutation operations are gone.

### Cursors are indices, and the SRFI says that is allowed

The specification requires only that cursors be "either disjoint from
indexes or identical to indexes", and permits an implementation to make
them identical where "there is no benefit in implementing them
separately". That is this case, and it is worth being clear about *why*
rather than treating it as a licence to do less work.

The abstraction exists for implementations that store strings as UTF-8 or
UTF-16, where the *n*th character is not at byte *n*, so `string-ref` by
index costs a scan from one end and a cursor is the thing you can keep in
hand and advance in constant time. scm's strings are `Vec<char>`
(plan §4.3) — four bytes per character, chosen so that `string-ref` is
already O(1). A cursor has nothing left to buy.

So a cursor is an exact non-negative integer, `string-cursor->index` and
`string-index->cursor` are the identity, `string-cursor-diff` is
subtraction, and the fifteen cursor procedures are one line each. The
reference implementation's own portable layer does exactly this. Two
consequences are tested rather than assumed: cursors must be disjoint
from `#f`, and integers are; and a `start`/`end` argument may be written
as either a cursor or an index, because here there is nothing to tell
apart.

### What the cursor convention actually changes: "not found"

This is the part that touches every caller, and it is not a free
translation of SRFI 13.

A cursor must be disjoint from `#f`, so a search cannot report failure
that way. Instead:

| | found | not found |
|---|---|---|
| `string-index` | the match's cursor | the **`end`** cursor |
| `string-index-right` | the **successor** of the match | the **`start`** cursor |
| `string-skip` | the non-match's cursor | the `end` cursor |
| `string-skip-right` | the successor | the `start` cursor |

The successor convention on the rightward searches is what makes them
usable: what comes back is a cursor you can cut a substring *at*, keeping
the match, which is why `(substring/cursors s 0 (string-index-right s …))`
is the natural idiom. SRFI 13 returns the matched character's own index,
one less.

It also has a cost worth stating, and the tests state it: **a match at the
very end of the range and no match at all give the same cursor.** A caller
who must distinguish them compares against `end` first. That is the price
of having no spare value.

The exception is `string-contains` and `string-contains-right`, which the
specification does let answer `#f`. The pair looks inconsistent beside
`string-index` and is not: a substring can be found *at* the end cursor,
so "found there" and "not found" need two different answers and there is
no cursor left over. The two also disagree about the empty pattern —
`string-contains` finds it at `start1`, `string-contains-right` at `end1`
— which is just leftmost and rightmost being different places.

### `string-trim` really does trim differently

`(srfi 130)`'s `string-trim` defaults to `char-whitespace?` where
`(srfi 13)`'s defaults to `(srfi 14)`'s `char-set:whitespace`. That is not
a restatement of the same thing in other words. Char-sets here are
Latin-1, and Latin-1's whitespace list omits U+0085 `NEL` — so
`(srfi 13)` leaves a leading NEL in place and `(srfi 130)` trims it. The
test imports both libraries, prefixed, and asserts each behaviour, so the
difference is recorded rather than discovered later.

This is the third library in a row where the Latin-1 bound of
[tranche eleven](#tranche-eleven-char-sets-and-a-bound-chosen-three-times-over)
has shown through, and the resolution is the same each time: a predicate
is Unicode-correct over scm's whole character range, a char-set is not.

### Built on SRFI 13, and one collision fewer

Twenty-eight of these procedures are SRFI 13's unchanged, imported by name
and re-exported rather than written a second time — the same reasoning as
`(srfi 133)`'s sixteen re-exports. Eight more are imported under private
names and adjusted, so there is still one implementation of each piece of
work: the trims pass an explicit predicate, the four searches change the
sentinel, and `string-replicate` is `xsubstring` with `to` made mandatory
and `from > to` made an error. `string-remove` is `string-delete` under a
new name, which `rename` does at the import.

A welcome side effect of the dropped operations: `(srfi 130)` exports
neither `string-map` nor `string-for-each`, so unlike `(srfi 13)` it
collides with R7RS nowhere, and the test asserts that R7RS's n-ary
`string-map` still works after importing it. `string-for-each`'s
replacement, `string-for-each-cursor`, hands the procedure the cursor
rather than the character.

### `string-split` is not `string-tokenize`

They look like the same operation and are opposites. `string-tokenize`
keeps the maximal runs that *match* a criterion, so it never returns an
empty piece. `string-split` cuts at a fixed delimiter *string* and keeps
what lies between, so adjacent delimiters give empty pieces — and
`'prefix` and `'suffix` exist precisely to drop the empty piece a leading
or trailing delimiter would otherwise leave. Its `limit` counts **splits**
rather than pieces, so the result holds at most `limit + 1` of them and
the last keeps whatever remains, delimiters included. An empty delimiter
splits into characters.

## Tranche fourteen: the pattern matcher, and two holes in the expander

`(srfi 204)` is `match` — the Wright-Cartwright-Shinn matcher, the
destructuring form a Scheme or Racket reader already knows, since Chibi,
Gauche, Guile's `(ice-9 match)` and Racket's own `match` are all this shape.
It is here because the ray port measured the need: **551 destructuring binds
in ray's non-example Janet and 466 in its 218 scene files**, about a thousand
sites, and R7RS has nothing for them — `define-values` and `receive`
destructure multiple *values*, not structure.

**It is a withdrawn SRFI**, and that is stated here rather than buried:
withdrawn 2022-02-02. What withdrawal means for this library is that its
document will not change again, not that the matcher is unusual. The number
is used because implementations do use it for this matcher; if that ever
looks like a claim too far, `(scm match)` is a one-line rename.

### What it supports, and what it does not

The grammar, less three groups. Supported: pattern variables, `_`, the
literals (including a **keyword**, which the SRFI's grammar does not list
because no standard Scheme has them), `'sexp` and quoted data, list patterns
with an improper tail, `...` and `___` ellipsis — one per list, with any
number of fixed patterns after it — vector patterns with ellipsis, `and`,
`or`, `not`, `?` with extra patterns, `=` accessors, quasipatterns with
`unquote`, the `(=> fail)` restart, and all five derived forms, with
`match-let` binding in **parallel** and in its named-loop form.

Absent, in three groups with different standing:

| | why |
|---|---|
| `$`, `struct`, `object` — record patterns | **the SRFI permits it**: *"This pattern may not be available (because an implementation has not made the necessary record introspection forms available)"*. scm's records expose no `is-a?`/`slot-ref` to a library |
| `***` — tree patterns | **the SRFI permits it**: *"tree patterns are considered experimental at this stage and so are an optional part of this SRFI"* |
| `**1`, `=.. k`, `*.. k j` — counted ellipses; `get!`/`set!`; `unquote-splicing` in a quasipattern; repeated (nonlinear) pattern variables | **mine.** Each is a real omission rather than a permitted one. The repetition case is the one to know about: the SRFI says an implementation that does not support it *"should cause a syntax error"*, and this one does not — a repeated variable rebinds instead of comparing, so `((a a) …)` silently matches two different values. Do not write one |

`match-letrec` is `match-let*`: sequential, not recursive, so a pattern
cannot refer to a binding made later in the same form.

### The two expander holes it found, which is the point of writing libraries

Both were found by a pattern that could not fire, not by reading.

**`#(…)` in a `syntax-rules` pattern matched nothing.** R7RS 4.3.2 lists
vector patterns and vector templates; scm had neither. A `#(p …)` pattern
fell through to the constant case and was compared with `equal?`, so it
matched only a vector equal to the pattern itself — which is to say never —
and a `#(t …)` template instantiated its elements one at a time, so it
reached a repeated binding with no ellipsis around it and raised *"pattern
variable a needs ellipsis"*. Both are now the same sequence machinery the
list cases use, which is three arms in `syntax.rs` and no new concepts.
**The conformance number did not move**: 1112/71 before and after, because
`r7rs-tests.scm` never exercises a vector pattern. A feature gained with no
number to show for it is the clearest case yet of this document's own claim
that the number is not the finding.

**`_` in a literals list is still a wildcard.** `(syntax-rules (_) ((_ v _ sk
fk) sk) …)` — declaring `_` a literal and expecting it to match only the
identifier `_` — instead matches *everything* and binds nothing, so the
matcher's leaf rule swallowed every pattern and bound no variables. That cost
this library its first version. It is the facet of
[conformance.md](conformance.md)'s *"`_` in a pattern matches without
binding; as an argument it is an ordinary identifier"* that the suite's seven
remaining hygiene corners do not cover — and that document says *"if a real
macro ever trips on one, that changes the arithmetic."* One just did. The
library asks the question the other way round instead, with the candidate as
the literal and `_` as the input, which answers correctly and needed no
expander change.

### Three techniques worth recording, because they are not obvious

- **An identifier can be told from a literal inside `syntax-rules`**: put the
  candidate in a nested rule's pattern, where an identifier becomes a pattern
  *variable* and matches a probe while a number or string becomes a literal
  and does not. Everything in the matcher's leaf case rests on this.
- **`(syntax-rules <ellipsis> (<literal> …) …)` renames the ellipsis**, which
  is what makes `...` itself available as a literal to match. Every macro here
  that has to recognise an ellipsis *inside* a pattern declares its own.
- **The ellipsis split needs no backtracking.** `(p … p1 … pN)` is decided by
  counting the fixed patterns after the ellipsis, at expansion time, and
  `(length (quote rest))` counts them at run time by quoting the pattern as
  data. The SRFI forbids two ellipses in one list, which is exactly what makes
  counting sufficient.

## What is still absent

The control and testing SRFIs: 34/35, 41, 45, 113, 117, 121, 64, 19, 48,
98, 115. `(srfi 47)` will not be added: it is withdrawn and `(srfi 63)`
supersedes it. `(srfi 152)` is the third string library in this family and
is the one that finally regularises SRFI 13's argument orders; it is not
here yet.
