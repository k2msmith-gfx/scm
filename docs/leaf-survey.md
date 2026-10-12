# Survey: what do ray's authored fields and textures actually need?

2026-10-12, against ray @19bb6c5. This answers [plan.md](plan.md) §13's open
questions 1 and 2 — *is the pure-leaf subset big enough for the fields people
actually write*, and *could a Rust-authored field DSL deliver requirement (13)
instead* — and it changes the answer to both.

Measurement scenes in [`spikes/s0-leaf-survey`](../spikes/s0-leaf-survey).

---

## 1. The headline

**The survey moves the justification for the leaf compiler off fields and onto
textures, and it does so on a measured number rather than an argument.**

- Authored *fields* are one example in the whole corpus, and it is six lines of
  pure arithmetic.
- Authored *textures* are ~40 call sites, and one of them costs **17.2
  seconds**. A real scene's build was measured at **15.22 s**, essentially all
  of it authored texel functions.

ray's own plan says in its §1 that this project is **not** for making scripting
faster, citing the 2026-09-12 finding of no Janet hot spot and a ~3,200× gap.
That is correct for scene loading and **wrong for `make-texture`**, which is a
genuine interpreter-bound hot loop. It is the one place ray has one.

---

## 2. Fields: the capability has exactly one user

Every `add-implicit` call site in `examples/` — 16 of them — **composes
Rust-side combinators**. Not one passes an authored function:

```
(add-implicit :gyroid :cell 0.6 :thickness 0.05 :inside (sphere-field ...) :trace true)
(add-implicit (field-intersect (voronoi-net surface seeds ...) ...))
(add-implicit (strut-field nodes (net :edges) 0.018 :smooth 0.0072 :blend :round))
(add-implicit (mandelbulb-field :power 8 :iters 14) ...)
(add-implicit (field-union (capsules-field struts ...) ...))
(add-implicit (field-subtract blob (capsule-field ...) :smooth 0.07) ...)
```

The vocabulary they draw on is already large: `sphere-field`, `capsule-field`,
`capsules-field`, `bars-field`, `strut-field`, `tpms-field`,
`mandelbulb-field`, `voronoi-field`, `field-union`/`-intersect`/`-subtract`,
`field-shell`.

**There is exactly one authored field in the corpus**, in
`examples/geometry/baked-field.janet`, and it reaches the tracer through
`field-bake`:

```janet
(defn ripple-blob [x y z]
  (- (math/sqrt (+ (* x x) (* y y) (* z z)))
     (+ 0.78 (* 0.11 (math/sin (* 9 x)) (math/sin (* 7 y)) (math/sin (* 8 z))))))
```

`math/sqrt`, `math/sin`, `*`, `+`, `-`. **Trivially inside even the narrowest
possible leaf subset.**

The honest reading of n=1 is not "nobody wants this". That file's header is a
32-line essay explaining why an authored field *cannot* be traced and what the
workaround costs — it is a capability nobody uses because it does not exist.
But it is not a volume justification either, and the plan should stop implying
it is.

### Which makes question 2's answer: the DSL already shipped

`field-code` — *"flatten a field value to the flat postfix numbers
(`%add-implicit`) reads"* — means **ray already has a field IR**, and the
combinator fields already compile to a thread-safe postfix program the Rust
tracer evaluates. That *is* the Rust-authored field DSL that plan §13's
question 2 asks about, and it covers all 16 existing call sites.

So a DSL is not an alternative to the leaf compiler. It is the thing already
built for the composed case, and the only thing it cannot do is accept a
user's own `(fn [x y z] …)`. The leaf compiler's field story is therefore
narrower than advertised: it is a *second backend* for an IR that exists, plus
the ability to take authored functions into it.

### The real field argument is quality, not threads

`baked-field.janet` documents three costs of the bake workaround, and all
three are things a compiled field removes:

- **Resolution is the object.** A feature thinner than a cell is gone. `:res 96`
  is 912k calls (measured below: 0.167 s) and its ripples are ~3 cells wide,
  "as fine as this bake can honestly carry".
- **The box is a knife.** Outside `:bounds` the field reads as outside, so a
  solid poking through a face is sliced flat.
- **A visible cell pattern in the shading.** Trilinear interpolation is
  continuous across a cell face but its gradient — which *is* the normal — is
  not, so the shading steps very slightly at every cell. Raising `:res` shrinks
  the steps; **it never removes them.**

That is a stronger argument than thread-safety, and it is not the one either
plan was making.

---

## 3. Textures: ~40 call sites, and the cost is real

`(make-texture w h f &opt name)` runs an authored function once per texel.
Call sites across the corpus run from 128² to **2048²**, i.e. 4.2M calls for a
single texture.

### Measured, on this machine (`spikes/s0-leaf-survey/texcost.janet`)

The body is copied from `fruit-bowl.janet`'s lemon-skin texture so it is
representative, including its real `fbm` → `vnoise` → `hash2`/`fade` chain.

```
  128 x 128   =     16384 texels     0.068 s     4162 ns/texel
  256 x 256   =     65536 texels     0.272 s     4147 ns/texel
  512 x 512   =    262144 texels     1.078 s     4113 ns/texel
 1024 x 1024  =   1048576 texels     4.317 s     4117 ns/texel
 2048 x 2048  =   4194304 texels    17.249 s     4112 ns/texel

field-bake :res 48   =    110592 samples   0.023 s      209 ns/sample
field-bake :res 96   =    884736 samples   0.167 s      189 ns/sample
field-bake :res 128  =   2097152 samples   0.391 s      186 ns/sample
```

**4,112 ns/texel, flat across a 256× range in size.** The flatness matters: it
means this is interpreter time, not cache or allocator behaviour. One 2048²
texture is **17.2 seconds**.

`field-bake` is 20× cheaper per call only because `ripple-blob` is six
arithmetic ops, where the texture body is two `fbm` calls = eight `vnoise` =
32 `hash2`, plus nested `map` closures over colour lists.

### And in a real scene

| example | textures | texels | cost |
|---|---|---|---|
| `still-life/fruit-bowl.janet` | 5 (4× 768², 1× 2048×1024) | 4.46M | **15.22 s measured** |
| `geometry/arcology.janet` | 2 × 2048² | 8.39M | ~34 s estimated |
| `still-life/orange.janet` | 2 × 640² | 0.82M | ~3.4 s estimated |

fruit-bowl's *entire scene build* was timed at **15.22 s**
(`spikes/s0-leaf-survey/fb-build.janet`), against an 18.3 s extrapolation from
the per-texel rate — so the extrapolation is sound and slightly high, and the
textures are essentially the whole build. The geometry is free by comparison.

At even 20 ns/texel — generous for LLVM-compiled f64 math — that build becomes
**0.09 s**. A 150–200× class win on a user-visible wait, across ~40 call sites.

---

## 4. The subset in plan §6 is too narrow

The plan says a leaf function "may call only *pure registered primitives*".
**Every real texture body in the corpus violates that**, and none of them for
a reason that threatens thread-safety. What they actually use:

| needed | example | in §6 as written? |
|---|---|---|
| pure math, `max`/`min`/`mod` | all | yes |
| local `var` + `set` + `+=` | `fbm`'s accumulator | no |
| bounded loops (`repeat`) | `fbm`'s 4 octaves | no |
| conditionals (`when`) | fruit-bowl's specks | no |
| **transitive calls to authored pure helpers** | `fbm` → `vnoise` → `hash2`, `fade` | **no — and this is the big one** |
| higher-order `map` with inline `fn`, nested | every colour blend | no |
| **closures over precomputed immutable tables** | `heights`, `draws`, `ofx/ofy/ofh`, `h-at` | **no** |
| returning a 3-element colour vector | every texture | no |

Note especially that `fbm`, `vnoise`, `hash2`, `fade` and `mix` are **not Rust
verbs** — they are defined per-example in Janet. A primitives-only leaf
compiler would reject every single texture in the corpus.

But all of it is still *pure*: no scene queries, no store access, no I/O, no
mutation of anything that outlives the call. **So the thread-locality collision
genuinely does not arise** — the restriction that matters is *pure*, not
*primitives-only*.

### What the leaf compiler therefore has to support

1. **Transitive compilation** over authored pure functions, with a reachability
   analysis that reports *which* call made a function uncompilable.
2. **Captured immutable data** as a read-only snapshot — the precomputed
   lookup tables are built during the scene build and never written after.
3. **Local mutation, bounded loops, conditionals** — ordinary SSA work, free
   from LLVM.
4. **Monomorphised higher-order calls** — `map` over a literal 3-list with an
   inline `fn` must inline, not allocate a closure.
5. **A no-heap return convention** for small float vectors (a 3-tuple in
   registers), since one allocation per texel at 4.2M texels is the thing we
   are trying to delete.

(1) and (2) are the ones that need design. The rest is ordinary codegen.

---

## 5. What this changes in the plan

**The justification reorders**, and §3.2 should say so:

| | justification | evidence |
|---|---|---|
| 1 | **`make-texture` speed** | measured: 15.22 s in one real scene, ~40 call sites, 150–200× available |
| 2 | **field quality** — kills the three documented bake artifacts | 1 call site, but the artifacts are written up in the example itself |
| 3 | **field thread-safety** — the original framing | real, and the least load-bearing of the three |

**Target textures first.** They are both the larger win and the easier target:
2D, `(u,v)` in and a colour out, no Lipschitz bound to derive, no tracer
integration, no `:trace` path, and a bake that is already a bake — so there is
no semantic change at all, only a faster one.

**And it partly contradicts ray's §1.** "Not for making scripting faster" was
measured on scene loading and is right there. It is wrong about
`make-texture`, and that is now the strongest single number in favour of the
project. Worth correcting in ray's document rather than leaving the two plans
disagreeing.

**Risk 3 in the plan** ("the leaf compiler's subset may be too small to be
interesting") is **resolved, with a condition**: the subset is big enough *if*
it covers §4's list. If it ships as primitives-only, it covers one six-line
field and nothing else.
