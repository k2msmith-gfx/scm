;;; (scm noise) — procedural noise.
;;;
;;; Noise has no "right answer" to compare against, so these are
;;; PROPERTIES: the field must be deterministic, continuous across lattice
;;; boundaries, in the range it claims, and in the case of gradient noise
;;; exactly zero on the lattice. Leaf-compilability and agreement with the
;;; compiled form are asserted separately, in scm-llvm/tests/leaf_noise.rs.
(import (scm test) (scm noise) (scheme base) (scheme inexact))

(define (close? a b tol) (< (abs (- a b)) tol))
(define (in-range? x lo hi) (and (>= x lo) (<= x hi)))

;; Walk a grid and report whether every sample satisfies `ok?`.
(define (grid-all? ok? n step f)
  (let loop ((i 0) (good #t))
    (if (or (= i n) (not good))
        good
        (let* ((x (* step (exact->inexact i)))
               (y (* step (exact->inexact (modulo (* i 7) n)))))
          (loop (+ i 1) (and good (ok? (f x y))))))))

;; ── helpers ──────────────────────────────────────────────────────────
(check fract (list (noise-fract 2.25) (noise-fract -0.25)) '(0.25 0.75))
;; The quintic has zero first and second derivative at both ends, which is
;; what keeps the lattice from showing as creases under lighting.
(check fade-endpoints (list (noise-fade 0.0) (noise-fade 1.0)) '(0.0 1.0))
(check fade-midpoint (close? (noise-fade 0.5) 0.5 1e-12) #t)
(check fade-is-flat-at-the-ends
       (list (close? (noise-fade 0.001) 0.0 1e-7) (close? (noise-fade 0.999) 1.0 1e-7))
       '(#t #t))
(check lerp (list (noise-lerp 10.0 20.0 0.0) (noise-lerp 10.0 20.0 1.0)
                  (noise-lerp 10.0 20.0 0.5))
       '(10.0 20.0 15.0))
(check smoothstep-clamps
       (list (noise-smoothstep 0.0 1.0 -5.0) (noise-smoothstep 0.0 1.0 5.0)
             (noise-smoothstep 0.0 1.0 0.5))
       '(0.0 1.0 0.5))

;; ── the hashes ───────────────────────────────────────────────────────
(check hash-is-deterministic (= (noise-hash2 3.0 7.0) (noise-hash2 3.0 7.0)) #t)
(check hash-range
       (grid-all? (lambda (h) (in-range? h 0.0 1.0)) 400 1.0 noise-hash2) #t)
;; Uniform enough to be a noise source: the mean over a lattice should sit
;; near a half, which a badly chosen constant would not.
(check hash-mean-is-near-a-half
       (let loop ((i 0) (sum 0.0))
         (if (= i 2000)
             (close? (/ sum 2000.0) 0.5 0.02)
             (loop (+ i 1) (+ sum (noise-hash2 (exact->inexact i)
                                               (exact->inexact (modulo (* i 13) 97)))))))
       #t)
;; And neighbouring cells must NOT be correlated, or the noise shows the
;; lattice. This is the property the sin-based hash is weakest on, so it
;; is measured rather than assumed.
(check adjacent-cells-decorrelate
       (let loop ((i 0) (sum 0.0))
         (if (= i 1000)
             ;; mean |difference| between neighbours should be near the 1/3
             ;; expected of two independent uniforms
             (close? (/ sum 1000.0) 0.3333 0.05)
             (let ((x (exact->inexact i)) (y (exact->inexact (modulo (* i 5) 71))))
               (loop (+ i 1)
                     (+ sum (abs (- (noise-hash2 x y) (noise-hash2 (+ x 1.0) y))))))))
       #t)
(check hash1-and-hash3-have-the-same-range
       (list (in-range? (noise-hash1 3.5) 0.0 1.0)
             (in-range? (noise-hash3 3.5 1.25 -7.0) 0.0 1.0))
       '(#t #t))

;; ── value noise ──────────────────────────────────────────────────────
(check value-is-deterministic (= (value-noise-2 0.3 0.7) (value-noise-2 0.3 0.7)) #t)
(check value-range
       (grid-all? (lambda (v) (in-range? v 0.0 1.0)) 400 0.37 value-noise-2) #t)
;; On the lattice, value noise IS the hash: the fade weights are exactly
;; 0 and 1 there, so no interpolation happens.
(check value-at-a-lattice-point
       (close? (value-noise-2 3.0 7.0) (noise-hash2 3.0 7.0) 1e-12) #t)
;; Continuity across a cell boundary is the whole reason for the fade. A
;; jump here would show as a visible grid line.
(check value-is-continuous-across-a-boundary
       (close? (value-noise-2 0.9999999 0.3) (value-noise-2 1.0000001 0.3) 1e-5) #t)
(check value-3-range
       (let loop ((i 0) (good #t))
         (if (or (= i 200) (not good))
             good
             (let ((x (* 0.31 (exact->inexact i))))
               (loop (+ i 1) (and good (in-range? (value-noise-3 x (* 0.7 x) (* 1.3 x))
                                                  0.0 1.0))))))
       #t)
(check value-3-is-continuous
       (close? (value-noise-3 0.9999999 0.3 0.5) (value-noise-3 1.0000001 0.3 0.5) 1e-5)
       #t)

;; ── tiling ───────────────────────────────────────────────────────────
;; The point of the tiled variant: the two edges of a texture must agree
;; exactly, not approximately, or the seam shows.
(check tiled-wraps-in-x
       (close? (value-noise-2-tiled 0.3 0.7 8.0) (value-noise-2-tiled 8.3 0.7 8.0) 1e-12)
       #t)
(check tiled-wraps-in-y
       (close? (value-noise-2-tiled 0.3 0.7 8.0) (value-noise-2-tiled 0.3 8.7 8.0) 1e-12)
       #t)
(check tiled-wraps-in-both
       (close? (value-noise-2-tiled 0.3 0.7 8.0) (value-noise-2-tiled 16.3 24.7 8.0) 1e-12)
       #t)
;; And it is still noise, not a constant.
(check tiled-still-varies
       (> (abs (- (value-noise-2-tiled 0.3 0.7 8.0) (value-noise-2-tiled 3.3 5.7 8.0)))
          0.01)
       #t)

;; ── gradient noise ───────────────────────────────────────────────────
;; Gradient noise is EXACTLY zero on the lattice, because the offset to
;; the nearest corner is zero and the value is a dot product with it.
;; This is the sharpest available check that it is gradient noise at all
;; rather than value noise under another name.
(check perlin-is-zero-on-the-lattice
       (list (close? (perlin-2 3.0 7.0) 0.0 1e-12)
             (close? (perlin-2 -2.0 5.0) 0.0 1e-12)
             (close? (perlin-3 1.0 2.0 3.0) 0.0 1e-12))
       '(#t #t #t))
(check perlin-is-deterministic (= (perlin-2 0.3 0.7) (perlin-2 0.3 0.7)) #t)
(check perlin-range
       (grid-all? (lambda (v) (in-range? v -1.05 1.05)) 600 0.23 perlin-2) #t)
(check perlin-is-continuous-across-a-boundary
       (close? (perlin-2 0.9999999 0.3) (perlin-2 1.0000001 0.3) 1e-5) #t)
(check perlin-3-range
       (let loop ((i 0) (good #t))
         (if (or (= i 300) (not good))
             good
             (let ((x (* 0.19 (exact->inexact i))))
               (loop (+ i 1) (and good (in-range? (perlin-3 x (* 0.6 x) (* 1.7 x))
                                                  -1.05 1.05))))))
       #t)
(check perlin-3-is-continuous
       (close? (perlin-3 0.9999999 0.3 0.5) (perlin-3 1.0000001 0.3 0.5) 1e-5) #t)
;; It must actually vary -- a gradient basis that returned zero everywhere
;; would pass the lattice check above.
(check perlin-varies
       (> (- (perlin-2 0.37 0.61) (perlin-2 2.11 4.73)) -2.0) #t)
(check perlin-is-not-constant
       (> (abs (- (perlin-2 0.37 0.61) (perlin-2 2.11 4.73))) 0.01) #t)
;; The gradient selector covers all eight directions and all sixteen
;; cases, so no hash value falls through.
(check grad-2-covers-every-case
       (let loop ((i 0) (good #t))
         (if (or (= i 8) (not good))
             good
             (loop (+ i 1)
                   (and good (real? (perlin-grad-2 (+ 0.0625 (* 0.125 (exact->inexact i)))
                                                   0.5 -0.25))))))
       #t)
(check grad-3-covers-every-case
       (let loop ((i 0) (good #t))
         (if (or (= i 16) (not good))
             good
             (loop (+ i 1)
                   (and good (real? (perlin-grad-3 (+ 0.03125 (* 0.0625 (exact->inexact i)))
                                                   0.5 -0.25 0.75))))))
       #t)

;; ── simplex ──────────────────────────────────────────────────────────
(check simplex-is-deterministic (= (simplex-2 0.3 0.7) (simplex-2 0.3 0.7)) #t)
(check simplex-range
       (grid-all? (lambda (v) (in-range? v -1.4 1.4)) 600 0.23 simplex-2) #t)
(check simplex-is-continuous
       (close? (simplex-2 0.9999999 0.3) (simplex-2 1.0000001 0.3) 1e-5) #t)
;; Continuity has to hold across the triangle boundary inside a cell too,
;; which is where simplex differs from a square lattice.
(check simplex-is-continuous-across-the-diagonal
       (close? (simplex-2 0.4999999 0.5000001) (simplex-2 0.5000001 0.4999999) 1e-5) #t)
(check simplex-is-not-constant
       (> (abs (- (simplex-2 0.37 0.61) (simplex-2 2.11 4.73))) 0.01) #t)

;; ── sums over octaves ────────────────────────────────────────────────
;; No octaves is no signal, rather than a division by zero.
(check fbm-zero-octaves (perlin-2-fbm 0.3 0.7 0) 0.0)
;; One octave is the basis itself, since the sum is divided by the total
;; amplitude.
(check fbm-one-octave-is-the-basis
       (close? (perlin-2-fbm 0.3 0.7 1) (perlin-2 0.3 0.7) 1e-12) #t)
(check value-fbm-one-octave-is-the-basis
       (close? (value-noise-2-fbm 0.3 0.7 1) (value-noise-2 0.3 0.7) 1e-12) #t)
;; Dividing by the total amplitude is what keeps the range fixed as
;; octaves are added, instead of growing with them.
(check fbm-keeps-its-range
       (let loop ((oct 1) (good #t))
         (if (or (> oct 8) (not good))
             good
             (loop (+ oct 1)
                   (and good
                        (grid-all? (lambda (v) (in-range? v -1.05 1.05)) 60 0.31
                                   (lambda (x y) (perlin-2-fbm x y oct)))))))
       #t)
(check value-fbm-keeps-its-range
       (grid-all? (lambda (v) (in-range? v 0.0 1.0)) 200 0.31
                  (lambda (x y) (value-noise-2-fbm x y 5)))
       #t)
(check fbm-adds-detail
       (> (abs (- (perlin-2-fbm 0.3 0.7 1) (perlin-2-fbm 0.3 0.7 5))) 1e-6) #t)
(check value-3-fbm-range
       (let loop ((i 0) (good #t))
         (if (or (= i 100) (not good))
             good
             (let ((x (* 0.29 (exact->inexact i))))
               (loop (+ i 1) (and good (in-range? (value-noise-3-fbm x (* 0.6 x) (* 1.7 x) 4)
                                                  0.0 1.0))))))
       #t)
(check perlin-3-fbm-range
       (let loop ((i 0) (good #t))
         (if (or (= i 100) (not good))
             good
             (let ((x (* 0.29 (exact->inexact i))))
               (loop (+ i 1) (and good (in-range? (perlin-3-fbm x (* 0.6 x) (* 1.7 x) 4)
                                                  -1.05 1.05))))))
       #t)

;; Turbulence and ridged are non-negative by construction.
(check turbulence-is-non-negative
       (grid-all? (lambda (v) (>= v 0.0)) 300 0.23
                  (lambda (x y) (perlin-2-turbulence x y 4)))
       #t)
(check turbulence-3-is-non-negative
       (let loop ((i 0) (good #t))
         (if (or (= i 100) (not good))
             good
             (let ((x (* 0.29 (exact->inexact i))))
               (loop (+ i 1) (and good (>= (perlin-3-turbulence x (* 0.6 x) (* 1.7 x) 4)
                                           0.0))))))
       #t)
(check ridged-is-non-negative
       (grid-all? (lambda (v) (>= v 0.0)) 300 0.23
                  (lambda (x y) (perlin-2-ridged x y 4)))
       #t)
;; Ridged peaks where turbulence creases, so the two differ.
(check ridged-differs-from-turbulence
       (> (abs (- (perlin-2-ridged 0.3 0.7 4) (perlin-2-turbulence 0.3 0.7 4))) 1e-6) #t)

;; ── cellular ─────────────────────────────────────────────────────────
(check worley-is-deterministic (= (worley-2 0.3 0.7) (worley-2 0.3 0.7)) #t)
(check worley-is-non-negative
       (grid-all? (lambda (v) (>= v 0.0)) 300 0.23 worley-2) #t)
;; Searching nine cells bounds the distance: a feature point is always
;; within the home cell or one of its neighbours.
(check worley-is-bounded
       (grid-all? (lambda (v) (< v 3.0)) 300 0.23 worley-2) #t)
;; F2 is the second nearest, so it can never be nearer than F1.
(check f2-is-at-least-f1
       (grid-all? (lambda (p) p) 300 0.23
                  (lambda (x y) (>= (worley-2-f2 x y) (worley-2 x y))))
       #t)
(check worley-is-continuous
       (close? (worley-2 0.9999999 0.3) (worley-2 1.0000001 0.3) 1e-5) #t)
(check worley-3-is-non-negative
       (let loop ((i 0) (good #t))
         (if (or (= i 150) (not good))
             good
             (let ((x (* 0.29 (exact->inexact i))))
               (loop (+ i 1) (and good (>= (worley-3 x (* 0.6 x) (* 1.7 x)) 0.0))))))
       #t)
(check worley-3-is-continuous
       (close? (worley-3 0.9999999 0.3 0.5) (worley-3 1.0000001 0.3 0.5) 1e-5) #t)
;; The crack pattern, which is what F2 minus F1 is for.
(check f2-minus-f1-is-small-somewhere-and-large-elsewhere
       (let loop ((i 0) (lo 99.0) (hi 0.0))
         (if (= i 400)
             (and (< lo 0.05) (> hi 0.3))
             (let* ((x (* 0.137 (exact->inexact i)))
                    (y (* 0.211 (exact->inexact (modulo (* i 7) 400))))
                    (d (- (worley-2-f2 x y) (worley-2 x y))))
               (loop (+ i 1) (min lo d) (max hi d)))))
       #t)

(test-failures)
