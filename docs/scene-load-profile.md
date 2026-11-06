# Scene-load and prelude profile, 2026-11-06

Where the time goes when ray loads a scene and when the modeler boots,
measured after the scm speed campaign (S30–S34) closed. The question asked
was "are there compile-time bottlenecks?" The answer is **no**: scm's
compiler appeared in no profile. The two real costs are on ray's side and
are listed at the end as follow-ups **for whoever is working in ray**.

Binary: ray @09e534e6 built `--features scm,oidn` (no `jit`), ray's
`Cargo.lock` pinning scm at S32 (07bc735). Apple Silicon, release build.

## 1. The modeler's prelude: 71 ms of a 161 ms headless boot

    MODELER_HEADLESS=1 MODELER_BOOT_CLOCK=exit target/release/modeler

prints a mark per boot step (`modeler/src/perf.rs` `boot_mark`):

| step | ms |
|---|---|
| ray's verbs and prelude | 10.8 |
| the 23 modeler files, 00-data … 18-ui-config | 27 |
| ray's prelude again (shadowing order) | 8.6 |
| unify.scm (8,579 lines) | 18.6 |
| 99-scheme-only, init, replay, audit | 7 |
| window, event loop, first frame | 90 |

About 24,000 lines of Scheme in 71 ms; no single file over 19 ms. A
`define` compiles lazily on its first call, so load is reading plus
top-level evaluation, and nothing here is worth optimising.

## 2. Scene loads, per top-level form

Method: a scratch script `read`s a scene file form by form and `eval`s each
in `(interaction-environment)` between `(current-jiffy)` calls, printing the
forms over a threshold. An `(include "x.scm")` must be rewritten to an
absolute path, because `include` resolves relative to the *including* file,
which is the script. Run as `ray script.scm` (no rendering).

| scene | load | where it goes |
|---|---|---|
| ask21 | 27 ms | nothing notable |
| Fallingwater massing (4,000 lines, 14 includes) | 274 ms | ray geometry (levels.scm 112, seats-cut 49, stairs 43); scm 4% |
| shark45 | 1.4 s | one 512×512 `make-texture`, 1.25 s |
| stiletto | 3.3 s | two 704×832 wing `make-texture`s, 1.4 s each |
| fruit-bowl | 25.7 s | the skins, many textures |

Profiles (`sample`) of the Fallingwater load: `polygons::flips`,
`cut::Welder::id` and `build_sah`, with the standard-library SipHash on
`flips`'s `HashMap<(usize,usize),Vec<usize>>` about a fifth of the whole
load. scm's interpreter is 4% and its compiler absent.

## 3. Follow-ups — both in ray, neither started

1. **Build ray with `jit` for texel scenes.** Every `make-texture` lambda
   runs per texel through the interpreter at ~2.4 µs a texel; that is all of
   stiletto, shark and fruit-bowl. Under `jit` the texel leaf is compiled:
   fruit-bowl's skins went 104 s (10-24) → 25.7 s (interpreter work since)
   → 0.5 s (jit). The feature exists; the lever is using it, or making it
   the default build where LLVM 21 is available. `docs/leaf-survey.md` and
   the texel-style note say what a texel must look like to compile.

2. **FxHash in `src/geometry/ops/polygons.rs` `flips`.** The edge map is a
   std `HashMap` keyed by `(usize, usize)`; SipHash costs ~50 ms of every
   Fallingwater load. Replacing the hasher is a one-line change at the map's
   construction. ray may not depend on a hasher crate yet; an inline
   FxHasher is ~15 lines if adding one is unwelcome. Measure before/after
   with the per-form script, Fallingwater massing is the test case.

Neither needs anything from scm. The earlier idea of compiling derived
forms directly in scm (to speed the prelude) is withdrawn: the prelude it
would speed up costs 71 ms.
