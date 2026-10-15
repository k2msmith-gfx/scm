# `(scm noise)`: procedural noise, written to be compiled

2026-10-15. Source `scm/lib/scm/noise.sld`, properties
`scm/tests/srfi/noise.scm`, compilation and agreement
`scm-llvm/tests/leaf_noise.rs`, benchmark
`scm-llvm/examples/noise_bench.rs`.

## The workaround this removes

ray's scene corpus calls `noise` 191 times and `fbm` 69, all of it written
in the scene language because the Rust side has none. `orange.janet`
records what that cost, in its own comment: the noise fields are "computed
**ONCE into small tables** rather than hashed per texel", because "fbm is
four octaves of four hashes — sixteen `math/sin` per call".

Baking a field into a small table and sampling it is a workaround for
noise being too slow to evaluate. It costs resolution, it costs the
ability to vary the field with anything the table was not indexed by, and
it is a decision that has to be made per texture.

Measured on a 256×256 sheet, four octaves of fbm per texel, which is the
corpus's own shape:

| | total | per texel |
|---|---|---|
| leaf-compiled | 6.8 ms | 103 ns |
| interpreted | 932.4 ms | 14,228 ns |

**138×**, same answer both ways. One texel called from Rust, which is what
a renderer actually does per sample, costs **83.5 ns**.

At 14 µs a texel, a 1920×1080 field is half a minute and tables are the
only option. At 83 ns it is 170 ms on one thread, and ray renders on all
of them. That is the difference between baking and sampling.

## Everything is scalars, and that is not a style choice

Every one of the 25 exported procedures is leaf-compilable, and
`leaf_noise.rs` asserts it for each by name, so the property cannot
quietly lapse. Three measured limits of the leaf subset shaped the whole
library:

- **No sequence may be a parameter or an intermediate.** So noise is
  `(perlin-2 x y)`, never `(perlin-2 point)`.
- **A procedure cannot be a parameter.** So `fbm` cannot take the basis to
  sum over; there is one per basis instead, which is why `perlin-2-fbm`
  and `value-noise-2-fbm` are separate names rather than one higher-order
  procedure.
- **A leaf cannot see a library-private binding.** Only exported names
  become globals on import. So every helper on the compile path is
  exported — `noise-fract`, `noise-fade`, the three hashes. They are
  useful in their own right, but that is not why they are public.

A fourth limit showed up while writing it. **A loop may only be called in
tail position**, so the natural nested-loop search over a cell's nine
neighbours works for `worley-2`, whose inner loop can hand a single
accumulator back, and fails for `worley-2-f2`, whose accumulator has two
parts. All three cellular procedures therefore run **one flat loop** over
the nine or twenty-seven cells with the offsets derived from the counter.
That is the same shape for all three and keeps every call in tail
position.

There are no permutation tables. Gradients are selected arithmetically,
with comparisons and `even?` standing in for the bit tests, since the leaf
subset has no bitwise operation — and since a private table could not have
been seen anyway.

## The hash is the corpus's hash, and its limit is stated

`noise-hash2` is `fract(43758.5453 · sin(127.1x + 311.7y))`, the exact
constants `orange.janet` uses, so a converted scene produces the field it
already had rather than a different one that merely looks similar.

Its weakness is worth saying plainly: it is `sin`-based, so it degrades at
large coordinates where the argument loses precision, and it is not a good
general-purpose hash. The tests measure rather than assume what matters
for noise — the mean over two thousand lattice cells sits within 0.02 of a
half, and the mean absolute difference between adjacent cells sits within
0.05 of the 1/3 expected of two independent uniforms, which is the
property that stops the lattice showing through.

## What is in it

Helpers: `noise-fract`, `noise-fade` (Perlin's quintic, which has zero
first *and* second derivative at both ends, so cells do not crease under
lighting), `noise-lerp`, `noise-smoothstep`, and `noise-hash1/2/3`.

Bases: `value-noise-2`, `value-noise-3`, `value-noise-2-tiled`,
`perlin-2`, `perlin-3`, `simplex-2`, `worley-2`, `worley-2-f2`,
`worley-3`.

Sums: `value-noise-2-fbm`, `value-noise-3-fbm`, `perlin-2-fbm`,
`perlin-3-fbm`, `perlin-2-turbulence`, `perlin-3-turbulence`,
`perlin-2-ridged`.

Ranges: the hashes and `value-noise-*` give `[0,1)`; `perlin-*` and
`simplex-2` give roughly `[-1,1]` and are not normalised, as is
conventional; the `-fbm` sums divide by total amplitude so they keep their
basis's range however many octaves are added; `turbulence` and `ridged`
are non-negative; `worley-*` give a distance.

## What the properties pin

Noise has no right answer to compare against, so the tests assert
properties:

- **Gradient noise is exactly zero on the lattice**, to 1e-12. This is the
  sharpest available check that `perlin-2` is gradient noise at all rather
  than value noise under another name, since the value is a dot product
  with an offset that is zero there. A companion check asserts it is not
  constant, so the first cannot pass trivially.
- **Continuity across a cell boundary**, which is what the fade is for. A
  jump would show as a grid line. Simplex is additionally checked across
  the diagonal *inside* a cell, which is where its triangular lattice
  differs from a square one.
- **Value noise on the lattice equals the hash there**, since the fade
  weights are exactly 0 and 1.
- **The tiled variant wraps exactly**, to 1e-12 and not approximately,
  because a seam is visible at any tolerance. Checked in x, in y, and in
  both at once, plus a check that it still varies.
- **fbm keeps its range as octaves are added**, swept over one to eight,
  which is what dividing by the total amplitude buys. Zero octaves gives
  zero rather than dividing by zero, and one octave is the basis exactly.
- **F2 is never nearer than F1**, and `F2 - F1` is near zero somewhere and
  large elsewhere, which is the crack pattern that difference exists for.
- **Every gradient case is reachable**, all eight 2D directions and all
  sixteen 3D cases, so no hash value falls through the selector.

Separately, `leaf_noise.rs` sweeps forty points per basis and seven octave
counts, asserting the compiled field matches the interpreted one to 1e-12.
A texture that changed the moment it got fast would be worse than a slow
one.

## What is absent

**3D simplex.** 2D simplex is here because its triangular lattice removes
the axis-aligned directions along which Perlin's artefacts line up, which
is visible in a smooth gradient. In 3D the same argument applies but
`perlin-3` covers the need and simplex's advantage grows mainly at four
dimensions and beyond, so it was not worth the fifty lines yet.

**Analytic derivatives.** Curl noise and normal mapping from a height
field both want the gradient of the noise rather than its value. That is a
real gap, and the honest reason it is not here is that it doubles the
library and nothing has asked for it yet.
