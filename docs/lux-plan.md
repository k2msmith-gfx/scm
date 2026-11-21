# lux in scm — plan, 2026-11-21

**PROPOSED, nothing built.** lux is the live-coding 3D toolkit built in
Common Lisp in `~/devel/cl-gpu` (k2msmith-gfx/cl-wgpu): scenes as immutable
values built by functions, shown by `animate`, redefined while they run.
This plan moves it to scm, so that ray, `edit` and lux are one language.

Kevin, 2026-11-21: *"I am trying to learn using scheme rather than spread
myself across several lisps"* — and: *"scm is my code so if we need to
modify it to better support lux, we can do it."*

**This is a port of a design, not a transcription.** cl-wgpu is ~8,100
lines of Lisp, and about half of it is there because Common Lisp reaches
WebGPU through a C library: the generated C API, the plist builder, the
SDL3 bindings, the GPU setup package, and hand-written JPEG, HDR, TrueType
and glTF readers. In a Rust host all of that is a crate. What moves is the
part that makes lux lux: the scene language, the materials and their
shader language, the lights, the look, the knobs — and the WGSL, which is
already text and moves nearly verbatim.

## What it looks like

The same scene, in each:

```lisp
;; cl-wgpu, examples/lux-look.lisp
(group
 (camera :at '(0.6 1.6 7.5) :target '(-0.3 1.2 -8) :fov 55)
 (light :sun :toward (sun-toward) :color #xFFC890 :intensity *sun*)
 (move '(0 1.05 0)
       (shade (pbr :metallic 1 :roughness 0.05) (sphere 0.45))))
```

```scheme
;; lux in scm
(group
 (camera :at [0.6 1.6 7.5] :target [-0.3 1.2 -8] :fov 55)
 (light :sun :toward (sun-toward) :color #xFFC890 :intensity sun)
 (move [0 1.05 0]
       (shade (pbr :metallic 1 :roughness 0.05) (sphere 0.45))))
```

Keywords (`subset.md` §3), the lambda list's `&named` (§1) and `[x y z]`
vectors (§3 of the additions, glam's layout per `linalg.md`) are what make
this read the same. The names stay lux's (`docs/api.md` in cl-wgpu), so its
manual carries over with Scheme's habits: no `*earmuffs*`, `define` for
`defun`, `?` on predicates.

## Decisions

Each is a recommendation until Kevin says otherwise.

| | decision | why |
|---|---|---|
| **D1** | **A new repository, `lux`**, a Rust crate that takes `schemers` by git as ray does. This document moves there when it exists. | scm's gates must build with no toolchain and stay fast; `wgpu`, `winit` and the image crates are large. scm keeps "the `scm` crate has no dependencies". |
| **D2** | **Scheme owns the scene; Rust owns the GPU.** Scene constructors, `flatten`, materials' shader source, the UI's layout and behaviours are Scheme. Device, pipelines, buffers, passes, windows and loaders are Rust. | The live-coded part is the part a user redefines; it belongs in the language they redefine it in. Everything heavy is per pixel or per vertex and already on the GPU. |
| **D3** | **One crossing a frame.** The host calls the user's scene function, Scheme flattens it, and the host takes draws, lights and the look. Instance data and matrices cross as `f32vector`s, not lists of numbers. The exact shape is settled by P0's measurement (returned list vs. a `draw!` the flatten calls). | `Interp` is single-threaded (plan §3.3); one call keeps it on the render thread with no lock. ray measured 6.9 ms for a 40,000-element crossing, so boxed numbers per vertex are out from the start. |
| **D4** | **`wgpu` the crate, not wgpu-native.** | Same implementation, without an FFI. Metal on macOS and Vulkan on Linux are a feature flag, not a port. |
| **D5** | **`winit` for windows**, as ray's modeler does. | Pure Rust, already in the family; nothing in lux needs SDL. |
| **D6** | **Tests render offscreen, with no window.** A test builds a scene in Scheme, renders it into a texture and checks pixels, as cl-wgpu's tests do. | wgpu needs no surface to render. cl-wgpu's hidden-window machinery (`CL_WGPU_HIDDEN`, SDL hints) has no counterpart because it is not needed. |
| **D7** | **`animate` takes a symbol**, resolved every frame. Knobs are top-level variables a knob `set!`s. Live code is written at top level, not inside a `define-library`. | A redefinition at the REPL replaces the global; a `Proc` held by the host would keep the old one. `subset.md` §4: an import is a copy, so a redefinition inside a library does not reach its importers. |
| **D8** | **`edit` connects as it does to `scm --serve`.** lux owns a `Session`; a listener thread parses requests and passes them over a channel; the render thread runs `Session::handle` between frames. `interrupt` stays out of band through the session's flag. | `14-remote.scm` already runs a program with an address and speaks the protocol (`repl.md`). An `edit` pane inside lux, as in ray's modeler, can come later. |
| **D9** | **The shader language is emitted in Scheme; the WGSL preludes are `.wgsl` files** in the crate, taken from cl-wgpu's strings. | `define-material` and `define-shader-function` translate s-expressions to WGSL text, which is list processing. The preludes (lights, shadows, PCSS, fog, the film's passes) are already WGSL. |
| **D10** | **cl-wgpu is frozen as the reference.** Its Stage L (Linux) and Stage B (building without Nix) are dropped. | D4 makes Linux a feature flag. `cargo build` makes building without Nix the default. |

## Where each part of cl-wgpu goes

| cl-wgpu | lines | goes to |
|---|---|---|
| `src/` (generated C API, builder), `sdl3/`, `gpu/` | — | **dropped**: `wgpu` and `winit` |
| `lux/jpeg.lisp`, `hdr.lisp`, `gltf.lisp`, `font.lisp` | 990 | **dropped**: the `image` crate (PNG, JPEG, Radiance HDR), `gltf`, a TrueType rasteriser (`fontdue` or `ab_glyph`) |
| `lux/scene.lisp` | 720 | **Scheme**: the node constructors and `flatten` |
| `lux/math.lisp` | 235 | **Scheme**: `(scm linalg)` covers most of it already |
| `lux/color.lisp`, `random.lisp`, `behaviour.lisp`, `grid.lisp` | 234 | **Scheme** |
| `lux/shader.lisp` | 202 | **Scheme**: the WGSL emitter; the macros as `syntax-rules` |
| `lux/material.lisp` | 645 | **split**: the frame and light structs, preludes and BRDFs to `.wgsl`; the built-in materials (`pbr`, `blinn-phong`, `emissive`, `screen`…) to Scheme |
| `lux/ui.lisp`, `pick.lisp` | 621 | **Scheme** for layout, knobs and hit tests; **Rust** for the overlay pass and reading the pick buffer |
| `lux/text.lisp` | 249 | **Rust** for the glyph atlas; **Scheme** for `text` as a node |
| `lux/mesh.lisp` | 183 | **Rust**: built-in meshes, cached by shape key |
| `lux/render.lisp`, `shadow.lisp`, `environment.lisp` | 1,765 | **Rust**: the renderer, shadow maps and cascades, IBL prefiltering |
| `live/` (window, film) | 1,199 | **Rust**: the loop, the float film (AO, bloom, develop) |
| `lux/image.lisp`, `model.lisp` | 274 | **Rust** loaders; **Scheme** for `image`, `model`, `render-image` as nodes |
| `lux/runtime.lisp`, `run.lisp` | 449 | **Scheme** for `animate`, knobs and the frame function; **Rust** for the loop that drives them |
| `examples/lux-*.lisp` | 1,277 | **Scheme**, one set for both platforms |
| `tests/` | 2,642 | **Rust** tests that evaluate Scheme scenes and read back pixels |

Estimated sizes: about 3,000 lines of Scheme, about 4,000 of Rust, and the
WGSL from cl-wgpu nearly unchanged.

## Changes to scm

scm is Kevin's, so where the boundary is slow or awkward the fix is in scm
rather than around it. Each is a stage in scm's own table (plan §9), under
its gates, recorded in `subset.md` if it changes the language, and keeps
the `scm` crate free of dependencies. **Only what P0 shows is needed gets
built.**

| | change | needed if | note |
|---|---|---|---|
| **M1** | **`Val::F32s(Vec<f32>)` / `Val::F64s(Vec<f64>)`** for SRFI 4 vectors, both ways | an `f32vector` crosses as `Opaque` today | SRFI 4 is already native and unboxed (`srfi.md`), so this is a copy of the payload. `Val` is `#[non_exhaustive]`, so it is a patch, not a break. |
| **M2** | **Borrowed access to an `f32vector`** from a host function: a `&[f32]` for the length of the call | M1's copy shows in P0's numbers | Instance transforms for a flock of 10,000 are 640 KB a frame; copying once into the GPU buffer is the floor. |
| **M3** | **Host objects**: a `Val::Host(id, type)` the host creates and Scheme holds, prints and compares, released when unreachable | meshes, images and render targets must be values in a scene | Today a host thing would be an integer id with no lifetime. lux can start with keys (D3) and move to this when it pays. |
| **M4** | **A cheaper call into a known procedure** than `Interp::call` with `Val` arguments | P0 shows per-call overhead in a per-frame or per-knob path | — |
| **M5** | **Docstrings on `define`** reaching `describe` | the editor should show lux's verbs as it shows scm's | Check what `describe` already reads first. |
| **M6** | **A lower-level macro** (`er-macro-transformer` or `syntax-case`) | `syntax-rules` cannot write `define-material`'s keyword constructor, or its errors are unreadable | P0 tries `syntax-rules` first: it should expand `((drift 0.05) …)` into an `&named` lambda list. |

## Stages

Each leaves the suite green. Order follows cl-wgpu's history, which is
also the order of dependency.

| | what | exit criterion |
|---|---|---|
| **P0** | **Measure.** (a) the crossing: a 1,000-draw flat scene returned as a list vs. pushed through `draw!`, at 60 fps; (b) whether an `f32vector` crosses, and its cost; (c) `syntax-rules` expanding a parameter list into an `&named` lambda; (d) `Interp::call` per frame; (e) `wgpu` offscreen render and readback on macOS (Metal) and Linux (Vulkan). | a table here, like `sbcl-plan.md`'s L0, and the list of M-stages it turns on |
| **P1** | **The crate and a frame.** `lux` repo; `wgpu` device; a window that clears to a color; the same frame rendered offscreen to PNG. | a Rust test reads back the cleared color with no window open |
| **P2** | **The live loop.** `Session` on the render thread; the listener thread and channel (D8); `animate` with a symbol (D7); an error pauses on the last good frame and is answered to the send; interrupt. | from `edit`, redefine the scene function and see the next frame change; `(car 5)` in it pauses rather than closes |
| **P3** | **Scenes.** `group` `move` `turn` `scale` `paint` `shade` `camera` and the built-in shapes, flattened in Scheme; meshes cached in Rust; unlit and flat-lit pipelines; `instances`. | cl-wgpu's first scene tests as pixel checks |
| **P4** | **Materials and shaders.** The WGSL emitter, `define-shader-function`, `define-material`; the preludes as `.wgsl`; `pbr`, `blinn-phong`, `emissive`, `screen`. | `lux-materials` and `lux-mandelbulb` render as in cl-wgpu |
| **P5** | **Lights.** sun, point, area, spot, ambient; the 96-float light record. | cl-wgpu's lights tests |
| **P6** | **Text, knobs and picking.** Glyph atlas, `text`, the UI panels and their wrapping, `knob`, picking. | `lux-controls` works with the mouse |
| **P7** | **Images, models, environments.** PNG/JPEG/HDR, glTF, IBL. | `lux-models` and `lux-helmet` |
| **P8** | **Shadows.** Shadow maps, cascades, cubes, PCF, PCSS for area lights. | cl-wgpu's shadows tests |
| **P9** | **The float film.** `rgba16float`, AO, bloom, tone mapping, fog, `look`. | `lux-look` and `lux-ocean` |
| **P10** | **Render to texture.** `render-image`, `canvas`, feedback. | `lux-screens` |
| **P11** | **The examples and the manual.** All of cl-wgpu's examples; `docs/api.md` rewritten for Scheme. | every example runs on macOS and Linux from one file each |

**Later, not in this plan:** an `edit` pane inside lux; per-instance
behaviours compiled to native code through scm's leaf compiler (the flock's
update, `s11-leaf.md`); MIDI, OSC and audio input; compute.

## Risks

- **The per-frame crossing.** If P0 shows a 1,000-draw scene cannot cross
  in ~2 ms, lux must cache: keep the flattened scene and re-flatten only
  what a frame changed. cl-wgpu re-flattens every frame and never had to.
- **Live redefinition through libraries.** D7 works for top-level code. A
  lux library that a user redefines a function of at the REPL will not see
  it (`subset.md` §4). The answer is that lux's own internals are not what
  a user redefines; if that turns out wrong, the import rule is scm's to
  revisit.
- **No restarts.** cl-wgpu pauses on an error and shows the last frame, and
  so will this. What SBCL adds — continuing from the error with a value —
  scm does not have. That is the same as cl-wgpu's `live` in practice.

## Open questions for Kevin

1. The repository's name and owner (`k2msmith-gfx/lux`?).
2. D10: drop cl-wgpu's Linux port, or finish it as planned first?
3. Should lux one day be ray's GPU preview, or stay separate? Nothing here
   depends on the answer, but D2's boundary would suit either.
