;;; (scm noise) — procedural noise, written to be compiled.
;;;
;;; ray's scene corpus calls `noise` 191 times and `fbm` 69, all of it
;;; implemented in the scene language because the Rust side has none. And
;;; `orange.janet` records why that hurt, in its own comment: the noise
;;; fields are "computed ONCE into small tables rather than hashed per
;;; texel", because "fbm is four octaves of four hashes -- sixteen
;;; math/sin per call". Baking a texture into a small table and sampling
;;; it is a workaround for noise being too slow to evaluate.
;;;
;;; The point of this library is that the workaround is no longer needed.
;;; Every procedure here is **leaf-compilable** (plan §6), and
;;; `scm-llvm/tests/leaf_noise.rs` asserts that for every single one, so
;;; it cannot quietly stop being true.
;;;
;;; ## Everything is scalars, and that is not a style choice
;;;
;;; The leaf compiler takes a procedure only when its parameters are
;;; scalars and its result is one or three `f64`s. Three further limits
;;; shape this library, and all three were measured rather than assumed
;;; (`scm-llvm/tests/leaf_shapes.rs`):
;;;
;;;   * **No sequence may be a parameter or an intermediate.** So noise is
;;;     `(perlin-2 x y)`, never `(perlin-2 point)`.
;;;   * **A procedure cannot be a parameter.** So `fbm` cannot take the
;;;     basis to sum; there is one per basis instead, which is why
;;;     `perlin-2-fbm` and `value-noise-2-fbm` are separate names.
;;;   * **A leaf cannot see a library-private binding.** Only exported
;;;     names become globals on import, so every helper on the compile
;;;     path is exported -- `noise-fract`, `noise-fade`, the hashes. They
;;;     are useful in their own right, but that is not why they are here.
;;;
;;; There are no tables. The gradients are selected arithmetically, with
;;; comparisons and `even?`, rather than indexed out of a permutation
;;; array -- partly because a private table could not be seen, and partly
;;; because arithmetic beats a memory load once this is native code.
;;;
;;; ## The hash is the corpus's hash
;;;
;;; `noise-hash2` is `fract(43758.5453 · sin(127.1x + 311.7y))`, the exact
;;; constants `orange.janet` uses, so a converted scene produces the same
;;; field it did before rather than a different one that merely looks
;;; similar.
;;;
;;; Its limitation is worth stating plainly: it is `sin`-based, so it
;;; degrades at large coordinates where `sin`'s argument loses precision,
;;; and it is not a good general-purpose hash. For texture coordinates in
;;; the usual range it is fine, and the tests measure its uniformity and
;;; the decorrelation of neighbouring lattice cells rather than assuming
;;; either.
;;;
;;; ## Ranges
;;;
;;; `value-noise-*` and the hashes return `[0,1)`. `perlin-*` and
;;; `simplex-2` return roughly `[-1,1]` and are not normalised, as is
;;; conventional. The `-fbm` sums divide by the total amplitude, so they
;;; keep the range of their basis. `turbulence` and `ridged` are
;;; non-negative. `worley-*` return a distance, which is `0` at a feature
;;; point and grows from there.
(define-library (scm noise)
  (export
    ;; helpers, exported because a leaf must be able to see them
    noise-fract noise-fade noise-lerp noise-smoothstep
    noise-hash1 noise-hash2 noise-hash3
    ;; value noise
    value-noise-2 value-noise-3 value-noise-2-tiled
    ;; gradient noise
    perlin-2 perlin-3 perlin-grad-2 perlin-grad-3
    simplex-2
    ;; sums over octaves, one per basis
    value-noise-2-fbm value-noise-3-fbm
    perlin-2-fbm perlin-3-fbm
    perlin-2-turbulence perlin-3-turbulence
    perlin-2-ridged
    ;; cellular
    worley-2 worley-2-f2 worley-3)
  (import (scheme base) (scheme inexact))
  (begin

    ;; ── helpers ──────────────────────────────────────────────────────
    (define (noise-fract x) (- x (floor x)))

    ;; Perlin's quintic, which has zero first AND second derivative at
    ;; both ends -- so the lattice does not show as creases where cells
    ;; meet, which the cubic `t²(3-2t)` does under lighting.
    (define (noise-fade t) (* t t t (+ (* t (- (* t 6.0) 15.0)) 10.0)))

    (define (noise-lerp a b t) (+ a (* t (- b a))))

    (define (noise-smoothstep e0 e1 x)
      (let ((t (max 0.0 (min 1.0 (/ (- x e0) (- e1 e0))))))
        (* t t (- 3.0 (* 2.0 t)))))

    ;; ── the hashes ───────────────────────────────────────────────────
    ;; The corpus's constants exactly, so a converted scene keeps its
    ;; field. See the header on what this hash is and is not.
    (define (noise-hash1 x)
      (noise-fract (* 43758.5453 (sin (* x 127.1)))))

    (define (noise-hash2 x y)
      (noise-fract (* 43758.5453 (sin (+ (* x 127.1) (* y 311.7))))))

    (define (noise-hash3 x y z)
      (noise-fract (* 43758.5453 (sin (+ (* x 127.1) (* y 311.7) (* z 74.7))))))

    ;; ── value noise ──────────────────────────────────────────────────
    ;; One hashed value per lattice corner, faded and interpolated. The
    ;; cheapest noise there is, and what the corpus's `vnoise` was.
    (define (value-noise-2 x y)
      (let* ((ix (floor x)) (iy (floor y))
             (fx (noise-fade (- x ix))) (fy (noise-fade (- y iy)))
             (a (noise-hash2 ix iy))
             (b (noise-hash2 (+ ix 1.0) iy))
             (c (noise-hash2 ix (+ iy 1.0)))
             (d (noise-hash2 (+ ix 1.0) (+ iy 1.0))))
        (noise-lerp (noise-lerp a b fx) (noise-lerp c d fx) fy)))

    (define (value-noise-3 x y z)
      (let* ((ix (floor x)) (iy (floor y)) (iz (floor z))
             (fx (noise-fade (- x ix))) (fy (noise-fade (- y iy)))
             (fz (noise-fade (- z iz)))
             (x1 (+ ix 1.0)) (y1 (+ iy 1.0)) (z1 (+ iz 1.0))
             (c000 (noise-hash3 ix iy iz)) (c100 (noise-hash3 x1 iy iz))
             (c010 (noise-hash3 ix y1 iz)) (c110 (noise-hash3 x1 y1 iz))
             (c001 (noise-hash3 ix iy z1)) (c101 (noise-hash3 x1 iy z1))
             (c011 (noise-hash3 ix y1 z1)) (c111 (noise-hash3 x1 y1 z1)))
        (noise-lerp
         (noise-lerp (noise-lerp c000 c100 fx) (noise-lerp c010 c110 fx) fy)
         (noise-lerp (noise-lerp c001 c101 fx) (noise-lerp c011 c111 fx) fy)
         fz)))

    ;; Wraps exactly at `period`, so a texture made from it tiles without
    ;; a seam. The lattice coordinate is reduced before hashing, which is
    ;; the only way the two edges can agree.
    (define (value-noise-2-tiled x y period)
      (let* ((ix (floor x)) (iy (floor y))
             (fx (noise-fade (- x ix))) (fy (noise-fade (- y iy)))
             (wx0 (floor-remainder ix period))
             (wy0 (floor-remainder iy period))
             (wx1 (floor-remainder (+ ix 1.0) period))
             (wy1 (floor-remainder (+ iy 1.0) period))
             (a (noise-hash2 wx0 wy0)) (b (noise-hash2 wx1 wy0))
             (c (noise-hash2 wx0 wy1)) (d (noise-hash2 wx1 wy1)))
        (noise-lerp (noise-lerp a b fx) (noise-lerp c d fx) fy)))

    ;; ── gradient (Perlin) noise ──────────────────────────────────────
    ;; The dot product of a lattice gradient with the offset to the point.
    ;; The gradient is chosen from the corner's hash by arithmetic rather
    ;; than indexed out of a table, because a leaf cannot see a private
    ;; one -- and because eight comparisons beat a memory load in native
    ;; code.
    ;;
    ;; Eight directions: the four axes and the four diagonals.
    (define (perlin-grad-2 h x y)
      (let ((k (floor (* h 8.0))))
        (cond ((< k 1.0) (+ x y))
              ((< k 2.0) (- x y))
              ((< k 3.0) (+ (- x) y))
              ((< k 4.0) (- (- x) y))
              ((< k 5.0) x)
              ((< k 6.0) (- x))
              ((< k 7.0) y)
              (else (- y)))))

    (define (perlin-2 x y)
      (let* ((ix (floor x)) (iy (floor y))
             (dx (- x ix)) (dy (- y iy))
             (fx (noise-fade dx)) (fy (noise-fade dy))
             (x1 (+ ix 1.0)) (y1 (+ iy 1.0))
             (n00 (perlin-grad-2 (noise-hash2 ix iy) dx dy))
             (n10 (perlin-grad-2 (noise-hash2 x1 iy) (- dx 1.0) dy))
             (n01 (perlin-grad-2 (noise-hash2 ix y1) dx (- dy 1.0)))
             (n11 (perlin-grad-2 (noise-hash2 x1 y1) (- dx 1.0) (- dy 1.0))))
        ;; The 2D gradients reach √2, so the result is scaled to land in
        ;; roughly [-1,1] like every other basis here.
        (* 0.7071067811865476
           (noise-lerp (noise-lerp n00 n10 fx) (noise-lerp n01 n11 fx) fy))))

    ;; Perlin's own twelve gradients: the permutations of (±1,±1,0). The
    ;; selection is the classic one, with `even?` standing in for the bit
    ;; tests, there being no bitwise operation in the leaf subset.
    (define (perlin-grad-3 h x y z)
      (let* ((k (floor (* h 16.0)))
             (u (if (< k 8.0) x y))
             (v (cond ((< k 4.0) y)
                      ((or (= k 12.0) (= k 14.0)) x)
                      (else z)))
             (su (if (even? (exact (floor k))) u (- u)))
             (sv (if (even? (exact (floor (/ k 2.0)))) v (- v))))
        (+ su sv)))

    (define (perlin-3 x y z)
      (let* ((ix (floor x)) (iy (floor y)) (iz (floor z))
             (dx (- x ix)) (dy (- y iy)) (dz (- z iz))
             (fx (noise-fade dx)) (fy (noise-fade dy)) (fz (noise-fade dz))
             (x1 (+ ix 1.0)) (y1 (+ iy 1.0)) (z1 (+ iz 1.0))
             (dx1 (- dx 1.0)) (dy1 (- dy 1.0)) (dz1 (- dz 1.0))
             (n000 (perlin-grad-3 (noise-hash3 ix iy iz) dx dy dz))
             (n100 (perlin-grad-3 (noise-hash3 x1 iy iz) dx1 dy dz))
             (n010 (perlin-grad-3 (noise-hash3 ix y1 iz) dx dy1 dz))
             (n110 (perlin-grad-3 (noise-hash3 x1 y1 iz) dx1 dy1 dz))
             (n001 (perlin-grad-3 (noise-hash3 ix iy z1) dx dy dz1))
             (n101 (perlin-grad-3 (noise-hash3 x1 iy z1) dx1 dy dz1))
             (n011 (perlin-grad-3 (noise-hash3 ix y1 z1) dx dy1 dz1))
             (n111 (perlin-grad-3 (noise-hash3 x1 y1 z1) dx1 dy1 dz1)))
        (* 0.7071067811865476
           (noise-lerp
            (noise-lerp (noise-lerp n000 n100 fx) (noise-lerp n010 n110 fx) fy)
            (noise-lerp (noise-lerp n001 n101 fx) (noise-lerp n011 n111 fx) fy)
            fz))))

    ;; ── simplex noise, 2D ────────────────────────────────────────────
    ;; The point of simplex over Perlin is that its lattice is triangular,
    ;; so there are no axis-aligned directions for the gradient artefacts
    ;; to line up along -- Perlin's square lattice shows faint grid
    ;; structure in a smooth gradient, and this does not.
    ;;
    ;; The skew constants are the standard ones: (√3-1)/2 to go into the
    ;; simplex grid and (3-√3)/6 to come back.
    (define (simplex-2 x y)
      (let* ((f2 0.3660254037844386)
             (g2 0.21132486540518713)
             (s (* (+ x y) f2))
             (i (floor (+ x s))) (j (floor (+ y s)))
             (t (* (+ i j) g2))
             (x0 (- x (- i t))) (y0 (- y (- j t)))
             ;; which of the two triangles in the cell the point is in
             (i1 (if (> x0 y0) 1.0 0.0))
             (j1 (if (> x0 y0) 0.0 1.0))
             (x1 (+ (- x0 i1) g2)) (y1 (+ (- y0 j1) g2))
             (x2 (+ (- x0 1.0) (* 2.0 g2))) (y2 (+ (- y0 1.0) (* 2.0 g2)))
             ;; each corner's contribution falls off as (0.5 - r²)⁴, and
             ;; is simply dropped once r² passes 0.5
             (t0 (- 0.5 (* x0 x0) (* y0 y0)))
             (t1 (- 0.5 (* x1 x1) (* y1 y1)))
             (t2 (- 0.5 (* x2 x2) (* y2 y2)))
             (n0 (if (< t0 0.0) 0.0
                     (* t0 t0 t0 t0 (perlin-grad-2 (noise-hash2 i j) x0 y0))))
             (n1 (if (< t1 0.0) 0.0
                     (* t1 t1 t1 t1
                        (perlin-grad-2 (noise-hash2 (+ i i1) (+ j j1)) x1 y1))))
             (n2 (if (< t2 0.0) 0.0
                     (* t2 t2 t2 t2
                        (perlin-grad-2 (noise-hash2 (+ i 1.0) (+ j 1.0)) x2 y2)))))
        ;; 70 is the conventional scale that brings the sum to about [-1,1]
        (* 70.0 (+ n0 n1 n2))))

    ;; ── sums over octaves ────────────────────────────────────────────
    ;; One per basis, because a leaf cannot take the basis as an argument.
    ;; Each divides by the total amplitude, so the sum keeps the range of
    ;; what it is summing instead of growing with the octave count.
    (define (value-noise-2-fbm x y octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (value-noise-2 (* x freq) (* y freq))))
                  (+ total amp)))))

    (define (value-noise-3-fbm x y z octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (value-noise-3 (* x freq) (* y freq) (* z freq))))
                  (+ total amp)))))

    (define (perlin-2-fbm x y octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (perlin-2 (* x freq) (* y freq))))
                  (+ total amp)))))

    (define (perlin-3-fbm x y z octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (perlin-3 (* x freq) (* y freq) (* z freq))))
                  (+ total amp)))))

    ;; The absolute value of each octave, which creases the field at every
    ;; zero crossing: smoke and marble.
    (define (perlin-2-turbulence x y octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (abs (perlin-2 (* x freq) (* y freq)))))
                  (+ total amp)))))

    (define (perlin-3-turbulence x y z octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                  (+ sum (* amp (abs (perlin-3 (* x freq) (* y freq) (* z freq)))))
                  (+ total amp)))))

    ;; Turbulence turned inside out, so the creases become ridges: the
    ;; standard way to get a mountain silhouette out of noise.
    (define (perlin-2-ridged x y octaves)
      (let loop ((i 0) (freq 1.0) (amp 0.5) (sum 0.0) (total 0.0))
        (if (= i octaves)
            (if (= total 0.0) 0.0 (/ sum total))
            (let ((r (- 1.0 (abs (perlin-2 (* x freq) (* y freq))))))
              (loop (+ i 1) (* freq 2.0) (* amp 0.5)
                    (+ sum (* amp r r))
                    (+ total amp))))))

    ;; ── cellular (Worley) noise ──────────────────────────────────────
    ;; One feature point per lattice cell, placed by the cell's hash. The
    ;; value is the distance to the nearest one, so the field is zero at
    ;; each point and rises between them: scales, cracks, pebbles.
    ;;
    ;; The nine surrounding cells have to be searched, not just the one
    ;; the point is in -- a neighbour's feature point can be closer than
    ;; the home cell's own.
    ;;
    ;; That search is ONE loop over the nine cells rather than two nested
    ;; ones, with the offsets derived from the counter. Nesting reads
    ;; better until the accumulator has two parts, as `worley-2-f2`'s
    ;; does: an inner loop cannot then hand both back, and calling the
    ;; outer loop from inside the inner one is not a tail call, which the
    ;; leaf compiler refuses. One flat loop keeps every call in tail
    ;; position and is the same shape for all three.
    (define (worley-2 x y)
      (let ((ix (floor x)) (iy (floor y)))
        (let loop ((k 0) (best 8.0))
          (if (= k 9)
              (sqrt best)
              (let* ((cx (+ ix (- (exact->inexact (modulo k 3)) 1.0)))
                     (cy (+ iy (- (exact->inexact (quotient k 3)) 1.0)))
                     (fx (+ cx (noise-hash2 cx cy)))
                     (fy (+ cy (noise-hash2 (+ cx 41.0) (+ cy 7.0))))
                     (dx (- fx x)) (dy (- fy y))
                     (d (+ (* dx dx) (* dy dy))))
                (loop (+ k 1) (min best d)))))))

    ;; The distance to the SECOND nearest point. `(- f2 f1)` is the
    ;; classic crack pattern: zero exactly on the boundary between two
    ;; cells and rising away from it.
    (define (worley-2-f2 x y)
      (let ((ix (floor x)) (iy (floor y)))
        (let loop ((k 0) (f1 8.0) (f2 8.0))
          (if (= k 9)
              (sqrt f2)
              (let* ((cx (+ ix (- (exact->inexact (modulo k 3)) 1.0)))
                     (cy (+ iy (- (exact->inexact (quotient k 3)) 1.0)))
                     (fx (+ cx (noise-hash2 cx cy)))
                     (fy (+ cy (noise-hash2 (+ cx 41.0) (+ cy 7.0))))
                     (dx (- fx x)) (dy (- fy y))
                     (d (+ (* dx dx) (* dy dy))))
                (if (< d f1)
                    (loop (+ k 1) d f1)
                    (loop (+ k 1) f1 (min f2 d))))))))

    (define (worley-3 x y z)
      (let ((ix (floor x)) (iy (floor y)) (iz (floor z)))
        (let loop ((k 0) (best 8.0))
          (if (= k 27)
              (sqrt best)
              (let* ((cx (+ ix (- (exact->inexact (modulo k 3)) 1.0)))
                     (cy (+ iy (- (exact->inexact (modulo (quotient k 3) 3)) 1.0)))
                     (cz (+ iz (- (exact->inexact (quotient k 9)) 1.0)))
                     (fx (+ cx (noise-hash3 cx cy cz)))
                     (fy (+ cy (noise-hash3 (+ cx 41.0) cy cz)))
                     (fz (+ cz (noise-hash3 cx (+ cy 7.0) cz)))
                     (dx (- fx x)) (dy (- fy y)) (dz (- fz z))
                     (d (+ (* dx dx) (* dy dy) (* dz dz))))
                (loop (+ k 1) (min best d)))))))
))
