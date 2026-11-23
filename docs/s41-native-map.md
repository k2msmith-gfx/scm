# S41: `map`, `for-each` and `fold` run by the machine

lux's flock profile measured `map` over 3 elements at 1.9 µs and `fold`
over 3 at 1.0 µs, against a plain call at 0.03–0.08 µs, and
[linalg.md](linalg.md) ended its vec3 stage by naming this as the next
one: the prelude's and `(srfi 1)`'s iterations were Scheme, and every scm
program uses them.

## The mechanism: a frame, as `dynamic-wind` has

A `PrimFn` gets the engine and its arguments and cannot call anything,
which is why `apply`, `call/cc`, `dynamic-wind` and `call-with-values`
are recognised by `Vm::apply` rather than run as primitives. The
iterations are the same kind of thing and use the same mechanism, not a
second one: six primitive slots whose ids `Vm::apply` recognises
(`iter_prims`, beside the five special ids, and like them never "plain"),
and one new frame, `Frame::Iter { kind, f, list, more, acc }`, in
`scm/src/vm/iter.rs`.

Each element is one call of the procedure from that frame. The frame is
pushed, the procedure entered, and its value comes back to the frame
through the trampoline like any other return. So everything a Scheme loop
had, this has, with nothing written for it:

- **The safepoint between elements**: fuel, the interrupt and collection.
  A `map` over a circular list stops for either, even when the procedure
  is a plain primitive that has no safepoint of its own (tested in
  `unwind.rs`).
- **Unwinding**: an error in the procedure, a `raise` caught by `guard`,
  an escape continuation, a `dynamic-wind` inside or around it -- the
  unwind truncates the stack through the frame as through any other.
  A continuable `raise` returns into it and the map carries on.
- **The collector** traces the frame's procedure, lists and accumulator
  (`Frame::trace`). Results are gathered as a reversed heap list and
  reversed at the end, which is the allocation the Scheme made, so the
  heap cap still bounds a runaway; nothing is written into an existing
  object, so the write barrier has nothing to see.
- **All three engines**: T1 and T2 reach a non-plain primitive through
  `do_call` -> `apply`, the same path as `dynamic-wind`. The procedure is
  entered as `do_call` enters one: a plain primitive straight off the
  value stack, a compiled flat-parameter closure with its arguments where
  they were pushed (no vector), anything else through `apply`.

`call_procedure`, the nested evaluation a host function uses, was the
other candidate and is the wrong one: an escape or an outer handler cannot
cross the Rust frame it puts between the iteration and its caller.

## What went native

| name | where it was | kind |
|---|---|---|
| `map` | prelude; `(srfi 1)` re-exports it now | N lists, stops at the shortest |
| `for-each` | prelude | N lists, value unspecified |
| `%srfi-1-for-each` | `(srfi 1)`'s `for-each` | the same, value `#t`, as the Scheme had |
| `%fold` | `(srfi 1)`'s `fold` | `(kons e1 … acc)`, N lists |
| `filter` | prelude; `(srfi 1)` re-exports it now | one list |
| `reduce` | prelude | `(f acc elem)` with an init, one list |

Built on them and faster with no change: `vector-map`, `vector-for-each`,
`string-map`, `fold-left`, `(srfi 1)`'s `reduce`, `append-map`,
`filter-map`, `remove`, `map-in-order`.

**Semantics are the Scheme's**, call for call. `map`, `fold` and
`reduce` take each tail *before* calling the procedure and `for-each` and
`filter` after, as the old loops did, so a procedure that `set-cdr!`s the
pair it was handed sees the same thing; `filter` reads the element again
after the predicate. **One thing changed, as asked: an error map itself
raises names map** -- `(map car 5)` was `cdr: not a pair: 5` and is
`map: not a list: 5`; arity errors read as a Scheme lambda list's did
(`map expects 2 required arguments, got 1`). The prelude's helpers
`any-null?`, `map-car` and `map-cdr`, undocumented, are gone with the
definitions that used them.

## Measured

Same machine, same run, heavily loaded (a render in progress), so the
ratios are the result; µs per element, loop overhead subtracted, T1,
the better of two runs. "Before" is the binary built from `db89611`;
`ref-` are the old Scheme definitions run by the new binary.

| | before | after | after: `ref-` | before / after |
|---|---|---|---|---|
| a call of a closure `(f 1)` | 0.017 | 0.019 | | |
| `map` over 3 | 0.166 | 0.073 | 0.172 | 2.3× |
| `fold` over 3 (`(srfi 1)`) | 0.095 | 0.053 | 0.066 | 1.8× |
| `for-each` over 3 | 0.089 | 0.053 | | 1.7× |
| `map` over 1000 | 0.109 | 0.055 | 0.110 | 2.0× |
| `fold` over 1000 | 0.047 | 0.033 | 0.046 | 1.5× |
| `map car` over 1000 (a primitive) | 0.192 | 0.096 | | 2.0× |
| `map +` over two lists of 1000 | 1.28 | 0.047 | | 27× |

On T0 the loop was interpreted, and the gain is the loop: `map` over 3
4.27 -> 0.13 µs an element, `fold` 2.67 -> 0.055.

An element now costs about three calls (`map`) or two (`fold`), where it
cost six and five. What is left is the frame's round trip through the
trampoline and the callee's `Frame::Code`; see "Next".

The script is plain: `us-per` runs a thunk N times between two
`current-jiffy` readings, and the empty thunk's time is subtracted.

## Tests

- `scm/tests/srfi/native-map.scm`, run on T0 and T1 and again under GC
  stress (`srfi.rs`): the old definitions copied verbatim as `ref-…`,
  and every procedure logging its arguments, so native and ref must agree
  on the value AND the calls in order -- one list of each shape, two and
  three lists of every pair of lengths, improper lists and non-lists
  (same calls, then an error), procedures that mutate the list. Then
  errors propagating through `guard`, escapes, `dynamic-wind` on escape
  and on error, `raise-continuable`, the messages naming map, every kind
  of callee (primitive, variadic, `&opt`, continuation, parameter), and
  100,000-element lists. Run by the old binary it fails exactly the five
  message checks.
- `unwind.rs`: a circular list under `map`, `for-each`, `fold`,
  `filter`, `reduce` stops for fuel and for the interrupt, on both tiers,
  and the machine is usable after.
- The fuzzer generates them (`Gen::iteration`): folds over one and two
  lists with a non-commutative body, map over two lists, a for-each whose
  order shows in its value, filter into reduce, a raise or `error` inside
  the procedure, an improper list, an escape through a `dynamic-wind`.
  `tests/fuzz.rs` asserts each shape appears and that 80% of the programs
  using them run to a value (97% do).
- Old binary against new on 933 generated programs that use them, T1:
  identical output on all 933 once the one deliberate change -- the
  message's length, which some shapes return -- is masked.

## Gates

`cargo test` (734 passed), `cargo clippy --all-targets` (0 warnings),
`scm-fuzz --seed 42 --count 2000` (0 mismatches; also seed 7 x 3000, and
300 under GC stress), and with `--features t2 --t2` (0 mismatches, T2
included). Conformance unchanged at 1112 / 71. `scm-cli --test boot`
passes. `cargo test -p scm-llvm` passes (47): the leaf compiler
recognises `map` by name, syntactically, and specialises it over
literal sequences or refuses it, so what `map` is bound to does not
reach it.

## Next

- **`vector-map` and `vector-for-each` direct**, without `vector->list`:
  the same frame with an index in place of a tail. Their cost now is the
  two conversions, not the loop.
- **`fold-right` and `pair-fold`** are not native: the natural way is to
  collect the elements first, and collecting a circular list would loop
  in Rust with no safepoint. A frame that walks first and calls second
  would do it.
- **`any`, `every`, `find`, `count`, `filter-map`, `partition`, `delete`,
  `assoc`/`member` with a predicate**: the same frame with another kind,
  each a few lines in `iter.rs` and a block in the test program.
- **The per-element cost**: a procedure entered from the frame goes
  `Frame::Iter` -> `Frame::Code` -> trampoline -> back. Looping inside
  `iter_resume` for a plain primitive, counting ticks itself, would halve
  `map car`; letting `run_code`'s `ret!` resume an `Iter` frame without
  the trampoline would do the same for closures.
