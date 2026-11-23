# `(scm linalg)`: vectors, matrices and quaternions

2026-10-15. Source `scm/lib/scm/linalg.sld`, tests
`scm/tests/srfi/linalg.scm`, leaf-compilation proof
`scm-llvm/tests/leaf_shapes.rs`, benchmark
`scm-llvm/examples/linalg_bench.rs`.

**No SRFI covers linear algebra**, and procedural 3D cannot do without
it. That absence is why this is the first library in the project that is
not a standards implementation, and it means every convention in it is a
decision rather than a transcription.

## The conventions are ray's, not invented

ray is built on [glam], so matching it exactly means a value crossing the
bridge needs no reinterpretation. Each of these was read out of ray's
source rather than assumed:

| | convention | why it matters |
|---|---|---|
| vec3 | 3-element Scheme vector of reals | `[0 1 0]` already is one (plan §4.5), and `:at [0 1 0]` already passes one |
| quaternion | 4 elements, `x y z w`, **w last** | glam's storage order |
| mat4 | 16 elements, **column-major** | glam's layout, so the bridge is a copy |
| angles | radians everywhere | R7RS trigonometry is radians; **ray's API is degrees**, so `degrees->radians` is at the boundary |
| Euler | `xyz` means the product **Rx·Ry·Rz** | so **Z is applied to the vector first** |

That last row is load-bearing. ray's `:rotate [rx ry rz]` applies Z
innermost, and assuming the other order gives mirrored attitudes with no
error anywhere -- it cost ray two wrong renders before anyone noticed. The
order is therefore in the procedure's *name*, `mat4-rotation-xyz` and
`quat-from-euler-xyz`, and two tests pin it: one asserts the product is
`Rx·Ry·Rz`, and the other asserts it is **not** `Rz·Ry·Rx`, because a test
that passed under either convention would be worth nothing.

Nothing has to remember the column-major storage. `mat4-ref` takes a row
and a column, and both `mat4-from-rows` and `mat4-from-columns` exist, so
a matrix written out by hand reads the way it is drawn.

## Two layers, because of what the leaf compiler accepts

Every operation exists twice, and the reason is measured rather than
assumed. `scm-llvm/tests/leaf_shapes.rs` pins the boundary: the leaf
compiler (plan §6) compiles a procedure only when **its parameters are
scalars** and **its result is one or three `f64`s**. A sequence may be the
return shape, but never a parameter and never an intermediate value.

So a procedure taking two vec3s cannot be compiled, however pure it is.

- The **vector layer** -- `v3-dot`, `v3-cross`, `mat4*` -- takes and
  returns vectors. It is what scene code should write, and it runs on T1.
- The **component layer**, named with a trailing `*`, takes loose scalars:
  `(v3-dot* ax ay az bx by bz)`. It is leaf-compilable.

The vector layer is *defined in terms of* the component layer, so there is
one implementation of each piece of arithmetic and they cannot drift. Six
tests assert they agree on the same inputs.

### What that is worth, and the trap in it

A kernel written against the component layer, 200,000 iterations of a
squared distance and a dot product:

| | time | result |
|---|---|---|
| leaf-compiled | 0.40 ms | 37615851933.80022 |
| interpreted | 164.20 ms | 37615851933.80022 |

**406×**, with the two answers identical to the last digit.

The trap is that this comes from compiling **the whole kernel**, loop
included. Calling `v3-dot*` once from an interpreted loop buys nothing,
because the loop is still interpreted and each call pays the boundary. The
rule for an author is therefore: write the readable vector-layer form, and
when a loop turns out to be hot, rewrite **that whole loop** as one
component-layer procedure. The `*` layer is a performance escape hatch,
not a second interface, which is why it covers the dozen operations that
turn up in inner loops rather than all ninety.

## What the tests insist on

Range and algebra checks are the easy half. These are the ones that catch a
picture that is wrong without erroring:

- **Handedness.** `x cross y = z`, the right-handed convention. Backwards
  mirrors every normal in a scene.
- **The Euler order**, as above, asserted both ways round.
- **A normal transforms by the inverse transpose.** Under a non-uniform
  scale, carrying a normal by the matrix itself tilts it off the surface.
  One test asserts the transformed normal stays perpendicular to the
  transformed tangent, and a second asserts that the naive version does
  **not** -- so the first test cannot pass by accident.
- **Point versus direction.** A point carries the translation and divides
  through by w; a direction does neither.
- **Shortest-path slerp.** `q` and `-q` are the same rotation, so
  interpolating without the sign flip takes the long way round. Tested by
  negating an endpoint and checking the midpoint is still the 15° rotation.
- **Gimbal lock.** At a pitch of ±90° the other two angles are not
  separable, so the Euler round trip is asserted to give back the same
  *rotation*, not the same angles.
- **180° quaternion extraction.** `mat4->quat` uses Shepperd's method,
  taking the largest of four candidate denominators, because one fixed
  formula loses all precision at a half turn. Tested on all three axes.
- **Singular and near-singular matrices.** `mat4-inverse` uses
  Gauss-Jordan with partial pivoting, so a transform with a 1e-8 scale on
  one axis still inverts, and a genuinely singular one raises instead of
  returning infinities.

## The hot procedures are native (M7, 2026-11-21)

lux, a live-coding renderer written in scm, measured its per-frame work in
its P0 and found the arithmetic was the cost: scm's VM spends ~37 ns on a
floating-point operation, so `mat4*` took 12.7 µs, `mat4-translation`
6.1 µs and `mat4-rotation-y` 3.2 µs, and flattening a scene of 1,000 shapes
took 34 ms. Kevin, 2026-11-21, on ray: *"we use glam for matrices"* -- and
this crate has no dependencies, so glam itself was not an option.

**The representation did not change.** A flonum is an immediate `Value`,
so a vector of sixteen is one allocation and reading one is a load; only
the arithmetic needed moving. `src/prim/linalg.rs` holds `%mat4*`,
`%mat4-transpose`, `%mat4-inverse`, `%mat4-translation`, `%mat4-scaling`,
`%mat4-rotation-x/y/z`, `%mat4-transform-point`,
`%mat4-transform-direction`, `%quat*` and `%quat->mat4`, and the library
binds its names to them. What calls them -- `mat4-compose`,
`mat4-rotation-axis-angle`, `mat4-from-quaternion`,
`mat4-transform-normal`, `mat4-rotation-xyz` -- got faster with no change.

**Bit-identical, not close.** Each is a port of the Scheme it replaced,
operation for operation: `(+ a b c d)` folds left to right, so every sum
does; `(- x)` inside a sum is IEEE negation, so `x + -y` is written as
such; Rust does not contract `a * b + c` into a fused multiply-add. The
Scheme originals are kept verbatim in `scm/tests/srfi/linalg-native.scm`,
which checks every procedure against them on 300 pseudo-random inputs
each -- integers, zeros and negative zeros among them -- on both tiers,
with `equal?`, which tells -0.0 from 0.0.

| | Scheme | native |
|---|---|---|
| `mat4*`, two | 12.7 µs | 0.20 µs |
| `mat4-translation` | 6.1 µs | 0.29 µs |
| `mat4-rotation-y` | 3.2 µs | 0.14 µs |
| lux P0: flatten 1,000 shapes | 34.6 ms | 2.56 ms |
| lux P0: the 1,000 draws to the GPU, finished | 38.2 ms | 4.57 ms |

**Two deliberate differences.** Arithmetic results are always inexact,
where the Scheme kept a hand-built all-exact matrix exact -- every
constructor already made flonums. `mat4-transpose`, which only moves
elements, keeps them as they are, as before (the test caught the first
draft converting them). And a wrong shape is refused naming the
procedure, where the Scheme failed inside `vector-ref`.

**One addition, `mat4-store!`**: `(mat4-store! dst offset m)` writes M's
sixteen values in storage order into an `f32vector` or `f64vector` from
OFFSET -- the one copy a matrix makes on its way to a GPU buffer. Storage
order is column-major, glam's, so it is a straight copy.

**Still Scheme**, because nothing measured them hot: the vec3 layer
(measured hot since, 2026-11-22 -- see "Next" below), the vec4 layer,
`mat4-determinant`, `mat4-decompose`, `mat4->quat`, the camera matrices,
`quat-slerp`. Each can follow the same way when it is.

## The vec3 layer native (S39, built 2026-11-22)

**As built.** Sixteen procedures in `scm/src/prim/linalg.rs`, bound by
the library as M7's are: `v3`, `v3+`, `v3-`, `v3*`, `v3/`, `v3-negate`,
`v3-scale`, `v3-dot`, `v3-cross`, `v3-length`, `v3-length-squared`,
`v3-distance`, `v3-distance-squared`, `v3-normalize`, `v3-lerp`,
`v3-min`, `v3-max`. The open decision below was taken as recommended: a
flonum fast path, and otherwise the generic `+ - * / sqrt min max` the
Scheme called, in its order and grouping -- `(+ a b c)` as one three-way
fold, not two additions -- so results are bit-identical and exact vectors
stay exact (`(v3+ [0 1 0] [1 0 0])` is still `#(1 1 0)`, `(v3-length
#(3 4 0))` the exact 5). `v3-normalize`'s zero vector raises through the
same `error` with the same message. `v3-normalize-or`, `v3-clamp` and the
rest stay Scheme and call the native ones. `tests/srfi/linalg-native.scm`
checks each against the Scheme original on reals, exact integers (with
2^70 among them) and a mix, on both tiers; its comparison treats NaN as
the same as NaN, which `equal?` does not and `v3/` of 0 by 0 needs.

Measured on an Apple M4 Pro under heavy load (another render running), so
the ratios matter more than the times: µs a call, less the loop's own
0.046, the Scheme original against the native, in the same run.

| | Scheme | native | × |
|---|---|---|---|
| `v3` | 0.15 | 0.026 | 5.7 |
| `v3+`, two | 0.46 | 0.050 | 9.2 |
| `v3-scale` | 0.23 | 0.040 | 5.7 |
| `v3-dot` | 0.28 | 0.024 | 11.8 |
| `v3-length` | 0.21 | 0.012 | 18.5 |
| `v3-normalize` | 0.54-0.69 | 0.046 | 12-15 |

The design as it was handed over follows.


**Why.** lux profiled its flock (lux `docs/plan.md`, "The flock,
profiled"; the scripts are lux's `tools/flock-profile.scm` and
`tools/micro-costs.scm`). Once lux fixed its own layers, a 200-boid tick
was 16 ms, 12 of it steering, and steering is mostly the vec3 layer. On an
AMD Renoir laptop, less the loop's 0.13 µs:

| | µs a call |
|---|---|
| a call, a fixnum `+`, a float `*`, a record accessor | 0.03-0.08 |
| `v3` (three `exact->inexact`) | 0.44 |
| `v3-scale` | 0.9 |
| `v3+`, two arguments | 1.65 |
| `map` over 3, `fold` over 3 (not this library) | 1.9, 1.0 |

`v3+` is a loop over a rest list making seven calls (`v3-x` ×6 and
`v3-add*`) for three additions. Native, it should cost what `%mat4*`'s
call does, ~0.1-0.2 µs. Expected: steering 12 -> ~3 ms, the tick near
5 ms. ray gets the same, wherever its Scheme uses `(scm linalg)`.

**What.** As M7: primitives in `scm/src/prim/linalg.rs` named `%v3+` and
so on, registered in its `install`, and the library binding its names to
them -- `(define v3+ %v3+)   ; native: src/prim/linalg.rs` -- in place of
the Scheme. The measured ones first: `v3`, `v3+`, `v3-` (one argument
negates), `v3*`, `v3-negate`, `v3-scale`, `v3-dot`, `v3-cross`,
`v3-length`, `v3-length-squared`, `v3-distance`, `v3-distance-squared`,
`v3-normalize` (raising on zero with the same message), `v3-lerp`,
`v3-min`, `v3-max`. The variadic ones fold left to right, as the Scheme
does. The component layer (`v3-add*` and the rest) stays Scheme: the leaf
compiler compiles it (`scm-llvm/tests/leaf_shapes.rs`), and
`v3-normalize*` and friends are what its loops call.

**The open decision: exact components.** M7 made every matrix result
inexact, which changed nothing because every mat4 constructor already made
flonums. **The vec3 layer is different**: `[0 1 0]` is a v3 (§4.5) and
ray's `:at [0 1 0]` passes one, so exact vectors are common, and today
`(v3+ [0 1 0] [1 0 0])` is the exact `#(1 1 0)`. Making that `#(1.0 1.0
0.0)` would be visible -- printed, and to `eqv?`. **Recommended: a fast
path when every component is a flonum, and otherwise the generic
arithmetic the Scheme used**, so results are bit-identical AND as exact as
before, everywhere. (`v3` itself, `v3-scale` and `v3-lerp` already convert
with `%f`, so they are inexact either way; `v3-dot` of exact vectors stays
exact under the recommendation.) Look in `prim.rs` for the generic `+`,
`-`, `*` the fallback can call rather than repeating them.

**Errors.** A wrong shape refused naming the procedure, as M7 did; the
Scheme failed inside `vector-ref`. Check the fuzzer and the conformance
suite for any expected message that changes.

**Tests.** As M7's: the Scheme originals copied verbatim into
`scm/tests/srfi/linalg-native.scm` as `ref-…`, every native procedure
checked against them with `equal?` (which tells -0.0 from 0.0) on a
pseudo-random stream with integers, zeros and negative zeros among the
reals, on both tiers -- plus all-exact inputs, checking exactness is kept.
Then the timings in a table here, as M7's.

**Gates**: all of `CLAUDE.md`'s, the soak included. On NixOS the LLVM
gate needs the prefix and `LIBRARY_PATH` lines `CLAUDE.md` gives.

**Then in lux**: `cargo update -p schemers`, `cargo test --release`
inside `nix develop`, and the flock profiled again.

**After it, not part of it**: native `map` and `fold` (the prelude's and
`(srfi 1)`'s, both Scheme, ~0.6 µs an element). A primitive that calls a
Scheme procedure back is a VM matter, not a `prim/linalg.rs` one, so it
is its own stage. **Done as S41** ([s41-native-map.md](s41-native-map.md)):
with both, lux's flock tick went 4.30 -> 2.33 (S39) -> 2.08 ms (S41) and
its steering 3.17 -> 0.98 ms, A/B interleaved on a loaded M4 Pro.

## Deliberate choices worth knowing

- **`v3-normalize` raises on the zero vector**, because silently returning
  zero makes a mis-set direction look like a legal one.
  `v3-normalize-or` takes a fallback, and the component-layer
  `v3-normalize*` returns zero without raising, which is what a branchless
  loop wants.
- **`v3-refract` returns `#f` on total internal reflection**, rather than a
  vector of NaNs, so the caller has to decide what to do about it.
- **The constants are procedures**: `(v3-zero)`, not `v3-zero`. A vector is
  mutable, and a shared constant that someone wrote into would corrupt
  every later use of it.
- **`mat4-perspective` uses the OpenGL depth range**, -1 to 1. An API
  wanting 0 to 1 needs a remap after it.

[glam]: https://docs.rs/glam
