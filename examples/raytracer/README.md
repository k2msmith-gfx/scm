# A ray tracer in scm

`raytracer.scm` is a Whitted ray tracer in plain R7RS: a port of
[raylisp](https://github.com/k2msmith-gfx/raylisp) (pure SBCL), which is
itself a port of the Rust `ray` renderer's original core. All three render the
same demo scene — six spheres (Lambertian, Phong, dispersive glass, mirror,
Blinn-Phong, normals-as-colour), a ground plane, a backdrop rectangle, a turned
box, two point lights and a spot — at one sample per pixel. That makes it a
measure of what scm costs against compiled Common Lisp and against Rust on the
same work.

    scm examples/raytracer/raytracer.scm [--size WxH] [--out FILE]
    examples/raytracer/bench.sh [N] [WxH] [OUT.ppm]

scm has no threads in the language (`docs/plan.md` §1), so `bench.sh` uses
processes: worker K renders rows K, K+N, … and `paste` interleaves the
stripes. The workers share nothing, so there is no collector to contend on.

## Results

1280×720, Apple M4 Pro (8 performance + 4 efficiency cores), 2026-11-06,
scm `421cbe4`, render loop only, best of three for ray and raylisp:

| | 1 core | 12 cores | scaling |
|---|---|---|---|
| ray (Rust, rayon; `RAYON_NUM_THREADS=1` for one core) | 0.267 s · 3.45 Mpix/s | 0.032 s · 29 Mpix/s | 8.4× |
| raylisp (SBCL, `(speed 3)`; `sb-thread` row stripes) | 0.866 s · 1.06 Mpix/s | 0.326 s · 2.83 Mpix/s | 2.7× |
| scm (T1; one process per core) | 42.9 s · 0.021 Mpix/s | 5.33 s · 0.17 Mpix/s | 8.1× |

- **Per core, scm is ~50× raylisp and ~160× ray.** That is the gap between a
  bytecode interpreter and native code doing f64 arithmetic in a tight loop,
  with every `vec3` a heap vector.
- **On the whole machine scm closes to ~16× raylisp**, because its processes
  scale 8.1× where SBCL's threads get 2.7×: raylisp consed 966 MB a frame when
  last profiled (August), and SBCL's stop-the-world collector serialises the
  threads.
- **T2 does not help here** (5.57 s, 43.1 s): its gains are elsewhere.
- One process and twelve write byte-identical images, as do T1 and T2.

## Agreement

- **raylisp: matches.** 99.996 % of channels are identical, 103 differ by one
  level (f64 against raylisp's single-float), and one pixel, (707, 398),
  differs by up to 31 levels — a ray grazing an edge that the two precisions
  send different ways. `python3 ppmdiff.py raylisp.ppm demo.ppm` measures it.
- **ray: the same scene, not the same picture.** `ray-demo.scm` is ray's own
  copy of the scene; rendered today, 97 % of channels match raylisp. ray has
  moved on since the fork — its glass casts no hard shadow and has no
  dispersion fringe, and the spot's pool is brighter — so the timing is the
  same work, but the image is not a bit-for-bit reference.

## Port notes

- `vec3`, rays, hits, matrices, shapes, materials and lights are vectors with
  `syntax-rules` accessors — the counterpart of raylisp's inlined structs.
  Measured on T1, a million `vec3` adds cost 180 ms with vectors and macro
  accessors, 320 ms with accessor procedures, and **570 ms with
  `define-record-type`**, whose constructor builds an association list of its
  fields on every call. That last is a finding about scm, not the port.
- The three lit shaders share one loop parameterised on the specular model;
  the arithmetic is raylisp's.
- Output is ASCII PPM (P3): scm's file ports are textual. It is not timed.
- raylisp computes AABBs its scene loop never reads; they are left out.
