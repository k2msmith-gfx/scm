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
