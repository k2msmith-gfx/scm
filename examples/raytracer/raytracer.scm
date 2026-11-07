;;;; examples/raytracer/raytracer.scm
;;;;
;;;; A Whitted ray tracer in plain R7RS Scheme: a port of raylisp
;;;; (~/devel/raylisp, pure SBCL), which is itself a port of the Rust `ray`
;;;; renderer's original core.  It renders the same DEMO-SCENE so the three can
;;;; be compared on time and, within float tolerance, on pixels.
;;;;
;;;;     scm examples/raytracer/raytracer.scm [--size WxH] [--stripe K/N] [--out FILE]
;;;;
;;;; With no --stripe it renders the whole image and writes an ASCII PPM.  With
;;;; --stripe K/N it renders only rows K, K+N, K+2N, ... and writes them one row
;;;; per line with no header, so N processes can render one image between them
;;;; and `paste` can interleave the rows again (see bench.sh).  scm has no
;;;; threads in the language, so processes are how it uses more than one core.
;;;;
;;;; Differences from raylisp, each deliberate:
;;;;   * Numbers are f64 (scm's only flonum); raylisp uses single-float to match
;;;;     glam's f32.  Pixels agree to within a few levels, not bit for bit.
;;;;   * Small fixed-shape values (vec3, ray, hit, mat4, ...) are vectors with
;;;;     macro accessors -- the counterpart of raylisp's inlined structs.  A
;;;;     `define-record-type` constructor costs about 3x a vector in scm today.
;;;;   * The three lit shaders share one loop, parameterised by the specular
;;;;     model, instead of three copies.  The arithmetic is the same.
;;;;   * Output is ASCII PPM (P3): scm's file ports are textual.  Output is not
;;;;     timed.
;;;;   * No AABBs: raylisp builds them but its scene loop never reads them.

(import (scheme base) (scheme write) (scheme file) (scheme inexact)
        (scheme time) (scheme process-context))

(define pi (* 4.0 (atan 1.0)))
(define (deg d) (* d (/ pi 180.0)))

;;; --- vec3 -------------------------------------------------------------------

(define-syntax vec3   (syntax-rules () ((_ x y z) (vector x y z))))
(define-syntax vec3-x (syntax-rules () ((_ v) (vector-ref v 0))))
(define-syntax vec3-y (syntax-rules () ((_ v) (vector-ref v 1))))
(define-syntax vec3-z (syntax-rules () ((_ v) (vector-ref v 2))))

(define (v+ a b)
  (vec3 (+ (vec3-x a) (vec3-x b)) (+ (vec3-y a) (vec3-y b)) (+ (vec3-z a) (vec3-z b))))
(define (v- a b)
  (vec3 (- (vec3-x a) (vec3-x b)) (- (vec3-y a) (vec3-y b)) (- (vec3-z a) (vec3-z b))))
(define (v* a s)
  (vec3 (* (vec3-x a) s) (* (vec3-y a) s) (* (vec3-z a) s)))
(define (dot a b)
  (+ (* (vec3-x a) (vec3-x b)) (* (vec3-y a) (vec3-y b)) (* (vec3-z a) (vec3-z b))))
(define (cross a b)
  (vec3 (- (* (vec3-y a) (vec3-z b)) (* (vec3-z a) (vec3-y b)))
        (- (* (vec3-z a) (vec3-x b)) (* (vec3-x a) (vec3-z b)))
        (- (* (vec3-x a) (vec3-y b)) (* (vec3-y a) (vec3-x b)))))
(define (normalize a) (v* a (/ 1.0 (sqrt (dot a a)))))
(define (normalize-or-zero a)
  (let ((len (sqrt (dot a a))))
    (if (< len 1e-30) (vec3 0.0 0.0 0.0) (v* a (/ 1.0 len)))))

;; I - 2 (I.N) N, for unit I and N.
(define (reflect i n) (v- i (v* n (* 2.0 (dot i n)))))

;; glam's Vec3::refract: the zero vector on total internal reflection.
(define (refract i n eta)
  (let* ((n-dot-i (dot n i))
         (k (- 1.0 (* eta eta (- 1.0 (* n-dot-i n-dot-i))))))
    (if (>= k 0.0)
        (v- (v* i eta) (v* n (+ (* eta n-dot-i) (sqrt k))))
        (vec3 0.0 0.0 0.0))))

(define black (vec3 0.0 0.0 0.0))
(define (color-zero? c)
  (and (zero? (vec3-x c)) (zero? (vec3-y c)) (zero? (vec3-z c))))

;;; --- ray, hits ------------------------------------------------------------

;; The direction is NOT normalised: a hit found in a shape's local frame at
;; parameter t is the same point as t along the world ray.
(define-syntax make-ray      (syntax-rules () ((_ o d) (vector o d))))
(define-syntax ray-origin    (syntax-rules () ((_ r) (vector-ref r 0))))
(define-syntax ray-direction (syntax-rules () ((_ r) (vector-ref r 1))))

(define-syntax local-hit        (syntax-rules () ((_ t p n) (vector t p n))))
(define-syntax local-hit-t      (syntax-rules () ((_ h) (vector-ref h 0))))
(define-syntax local-hit-point  (syntax-rules () ((_ h) (vector-ref h 1))))
(define-syntax local-hit-normal (syntax-rules () ((_ h) (vector-ref h 2))))

(define-syntax hit          (syntax-rules () ((_ t p n m) (vector t p n m))))
(define-syntax hit-point    (syntax-rules () ((_ h) (vector-ref h 1))))
(define-syntax hit-normal   (syntax-rules () ((_ h) (vector-ref h 2))))
(define-syntax hit-material (syntax-rules () ((_ h) (vector-ref h 3))))

;;; --- mat4: 16 flonums, column-major (row i, column j at i + 4j) -----------

;; Sixteen values in row-major reading order, stored column-major.
(define (mat4 . rows)
  (let ((rv (list->vector rows)) (m (make-vector 16 0.0)))
    (do ((i 0 (+ i 1))) ((= i 4) m)
      (do ((j 0 (+ j 1))) ((= j 4))
        (vector-set! m (+ i (* 4 j)) (inexact (vector-ref rv (+ (* 4 i) j))))))))

(define (mref m i j) (vector-ref m (+ i (* 4 j))))

(define (mat4-from-translation v)
  (mat4 1 0 0 (vec3-x v)  0 1 0 (vec3-y v)  0 0 1 (vec3-z v)  0 0 0 1))
(define (mat4-from-scale v)
  (mat4 (vec3-x v) 0 0 0  0 (vec3-y v) 0 0  0 0 (vec3-z v) 0  0 0 0 1))

;; A unit quaternion (x y z w) as a 4-vector, and its rotation matrix.
(define (quat-from-axis-angle axis angle)
  (let ((n (normalize axis)) (s (sin (* 0.5 angle))))
    (vector (* (vec3-x n) s) (* (vec3-y n) s) (* (vec3-z n) s) (cos (* 0.5 angle)))))

(define (mat4-from-quat q)
  (let ((x (vector-ref q 0)) (y (vector-ref q 1)) (z (vector-ref q 2)) (w (vector-ref q 3)))
    (let ((xx (* x x)) (yy (* y y)) (zz (* z z))
          (xy (* x y)) (xz (* x z)) (yz (* y z))
          (wx (* w x)) (wy (* w y)) (wz (* w z)))
      (mat4 (- 1.0 (* 2.0 (+ yy zz))) (* 2.0 (- xy wz)) (* 2.0 (+ xz wy)) 0.0
            (* 2.0 (+ xy wz)) (- 1.0 (* 2.0 (+ xx zz))) (* 2.0 (- yz wx)) 0.0
            (* 2.0 (- xz wy)) (* 2.0 (+ yz wx)) (- 1.0 (* 2.0 (+ xx yy))) 0.0
            0.0 0.0 0.0 1.0))))

(define (mat4-mul a b)
  (let ((out (make-vector 16 0.0)))
    (do ((i 0 (+ i 1))) ((= i 4) out)
      (do ((j 0 (+ j 1))) ((= j 4))
        (vector-set! out (+ i (* 4 j))
                     (+ (* (mref a i 0) (mref b 0 j)) (* (mref a i 1) (mref b 1 j))
                        (* (mref a i 2) (mref b 2 j)) (* (mref a i 3) (mref b 3 j))))))))

;; M = T * R * S: scale first, then rotate, then translate.
(define (mat4-from-scale-rotation-translation scale rotation translation)
  (mat4-mul (mat4-from-translation translation)
            (mat4-mul (mat4-from-quat rotation) (mat4-from-scale scale))))

;; Gauss-Jordan with partial pivoting on a 4x8 augmented matrix.
(define (mat4-inverse m)
  (let ((a (make-vector 4)))
    (do ((i 0 (+ i 1))) ((= i 4))
      (let ((row (make-vector 8 0.0)))
        (do ((j 0 (+ j 1))) ((= j 4)) (vector-set! row j (mref m i j)))
        (vector-set! row (+ 4 i) 1.0)
        (vector-set! a i row)))
    (do ((col 0 (+ col 1))) ((= col 4))
      (let ((pivot col))
        (do ((r (+ col 1) (+ r 1))) ((= r 4))
          (when (> (abs (vector-ref (vector-ref a r) col))
                   (abs (vector-ref (vector-ref a pivot) col)))
            (set! pivot r)))
        (when (< (abs (vector-ref (vector-ref a pivot) col)) 1e-12)
          (error "mat4-inverse: matrix is singular"))
        (let ((tmp (vector-ref a col)))
          (vector-set! a col (vector-ref a pivot))
          (vector-set! a pivot tmp))
        (let* ((prow (vector-ref a col)) (d (vector-ref prow col)))
          (do ((j 0 (+ j 1))) ((= j 8)) (vector-set! prow j (/ (vector-ref prow j) d)))
          (do ((r 0 (+ r 1))) ((= r 4))
            (unless (= r col)
              (let* ((row (vector-ref a r)) (f (vector-ref row col)))
                (do ((j 0 (+ j 1))) ((= j 8))
                  (vector-set! row j (- (vector-ref row j) (* f (vector-ref prow j)))))))))))
    (let ((out (make-vector 16 0.0)))
      (do ((i 0 (+ i 1))) ((= i 4) out)
        (do ((j 0 (+ j 1))) ((= j 4))
          (vector-set! out (+ i (* 4 j)) (vector-ref (vector-ref a i) (+ 4 j))))))))

;; The hot transforms index the column-major store directly.
(define (transform-point m v)
  (let ((x (vec3-x v)) (y (vec3-y v)) (z (vec3-z v)))
    (vec3 (+ (* (vector-ref m 0) x) (* (vector-ref m 4) y) (* (vector-ref m 8) z) (vector-ref m 12))
          (+ (* (vector-ref m 1) x) (* (vector-ref m 5) y) (* (vector-ref m 9) z) (vector-ref m 13))
          (+ (* (vector-ref m 2) x) (* (vector-ref m 6) y) (* (vector-ref m 10) z) (vector-ref m 14)))))

(define (transform-vector m v)
  (let ((x (vec3-x v)) (y (vec3-y v)) (z (vec3-z v)))
    (vec3 (+ (* (vector-ref m 0) x) (* (vector-ref m 4) y) (* (vector-ref m 8) z))
          (+ (* (vector-ref m 1) x) (* (vector-ref m 5) y) (* (vector-ref m 9) z))
          (+ (* (vector-ref m 2) x) (* (vector-ref m 6) y) (* (vector-ref m 10) z)))))

;; A normal goes through the inverse-transpose; LOCAL-FROM-WORLD is already the
;; inverse, so this is its upper-left 3x3 transposed.  Not normalised.
(define (transform-normal local-from-world v)
  (let ((m local-from-world) (x (vec3-x v)) (y (vec3-y v)) (z (vec3-z v)))
    (vec3 (+ (* (vector-ref m 0) x) (* (vector-ref m 1) y) (* (vector-ref m 2) z))
          (+ (* (vector-ref m 4) x) (* (vector-ref m 5) y) (* (vector-ref m 6) z))
          (+ (* (vector-ref m 8) x) (* (vector-ref m 9) y) (* (vector-ref m 10) z)))))

;;; --- geometry: canonical primitives in their own local frame --------------

;; sphere: radius 1 at the origin.  plane: y = 0, normal +Y.  rect: the unit
;; square x, z in [-1, 1] on that plane.  cuboid: x, y, z in [-1, 1].

(define up (vec3 0.0 1.0 0.0))

(define (sphere-intersect o d)
  (let* ((a (dot d d)) (h (dot o d)) (c (- (dot o o) 1.0))
         (disc (- (* h h) (* a c))))
    (and (>= disc 0.0)
         (let* ((sq (sqrt disc))
                (t0 (/ (- (- h) sq) a))
                (t (if (<= t0 0.0) (/ (+ (- h) sq) a) t0)))
           (and (> t 0.0)
                (let ((p (v+ o (v* d t)))) (local-hit t p p)))))))

(define (sphere-occluded o d t-min t-max)
  (let* ((a (dot d d)) (h (dot o d)) (c (- (dot o o) 1.0))
         (disc (- (* h h) (* a c))))
    (and (>= disc 0.0)
         (let* ((sq (sqrt disc)) (t-near (/ (- (- h) sq) a)))
           (or (and (> t-near t-min) (< t-near t-max))
               (let ((t-far (/ (+ (- h) sq) a)))
                 (and (> t-far t-min) (< t-far t-max))))))))

(define (plane-intersect o d)
  (let ((dy (vec3-y d)))
    (and (>= (abs dy) 1e-6)
         (let ((t (/ (- (vec3-y o)) dy)))
           (and (> t 0.0) (local-hit t (v+ o (v* d t)) up))))))

(define (plane-occluded o d t-min t-max)
  (let ((dy (vec3-y d)))
    (and (>= (abs dy) 1e-6)
         (let ((t (/ (- (vec3-y o)) dy)))
           (and (> t t-min) (< t t-max))))))

(define (rect-intersect o d)
  (let ((dy (vec3-y d)))
    (and (>= (abs dy) 1e-6)
         (let ((t (/ (- (vec3-y o)) dy)))
           (and (> t 0.0)
                (let ((p (v+ o (v* d t))))
                  (and (<= (abs (vec3-x p)) 1.0) (<= (abs (vec3-z p)) 1.0)
                       (local-hit t p up))))))))

(define (rect-occluded o d t-min t-max)
  (let ((dy (vec3-y d)))
    (and (>= (abs dy) 1e-6)
         (let ((t (/ (- (vec3-y o)) dy)))
           (and (> t t-min) (< t t-max)
                (let ((p (v+ o (v* d t))))
                  (and (<= (abs (vec3-x p)) 1.0) (<= (abs (vec3-z p)) 1.0))))))))

(define (axis-normal axis sign)
  (case axis
    ((0) (vec3 sign 0.0 0.0))
    ((1) (vec3 0.0 sign 0.0))
    (else (vec3 0.0 0.0 sign))))

;; The slab test.  An axis-parallel ray makes 1/d infinite and a grazing one
;; 0*inf = NaN; flonum division gives both without a trap, and the strict
;; comparisons discard the NaN, as raylisp's masked traps do.
(define (cuboid-intersect o d)
  (let loop ((axis 0) (t-enter -inf.0) (t-exit +inf.0) (enter-axis 0) (exit-axis 0))
    (if (< axis 3)
        (let* ((oc (vector-ref o axis))
               (inv-d (/ 1.0 (vector-ref d axis)))
               (a (* (- -1.0 oc) inv-d))
               (b (* (- 1.0 oc) inv-d))
               (t0 (if (< inv-d 0.0) b a))
               (t1 (if (< inv-d 0.0) a b)))
          (let ((enter? (> t0 t-enter)) (exit? (< t1 t-exit)))
            (loop (+ axis 1)
                  (if enter? t0 t-enter) (if exit? t1 t-exit)
                  (if enter? axis enter-axis) (if exit? axis exit-axis))))
        (and (> t-exit t-enter) (> t-exit 0.0)
             (let* ((inside? (<= t-enter 0.0))
                    (t (if inside? t-exit t-enter))
                    (axis (if inside? exit-axis enter-axis))
                    (travel (if (negative? (vector-ref d axis)) -1.0 1.0))
                    (sign (if inside? travel (- travel))))
               (local-hit t (v+ o (v* d t)) (axis-normal axis sign)))))))

(define (cuboid-occluded o d t-min t-max)
  (let loop ((axis 0) (t-enter -inf.0) (t-exit +inf.0))
    (if (< axis 3)
        (let* ((oc (vector-ref o axis))
               (inv-d (/ 1.0 (vector-ref d axis)))
               (a (* (- -1.0 oc) inv-d))
               (b (* (- 1.0 oc) inv-d))
               (t0 (if (< inv-d 0.0) b a))
               (t1 (if (< inv-d 0.0) a b)))
          (loop (+ axis 1)
                (if (> t0 t-enter) t0 t-enter)
                (if (< t1 t-exit) t1 t-exit)))
        (and (> t-exit t-enter) (< t-enter t-max) (> t-exit t-min)))))

(define (intersect-local geometry o d)
  (case geometry
    ((sphere) (sphere-intersect o d))
    ((plane)  (plane-intersect o d))
    ((rect)   (rect-intersect o d))
    ((cuboid) (cuboid-intersect o d))))

(define (occluded-local geometry o d t-min t-max)
  (case geometry
    ((sphere) (sphere-occluded o d t-min t-max))
    ((plane)  (plane-occluded o d t-min t-max))
    ((rect)   (rect-occluded o d t-min t-max))
    ((cuboid) (cuboid-occluded o d t-min t-max))))

;;; --- shapes: a geometry placed in the world -------------------------------

(define (make-shape world-from-local geometry material)
  (vector world-from-local (mat4-inverse world-from-local) geometry material))
(define-syntax shape-world-from-local (syntax-rules () ((_ s) (vector-ref s 0))))
(define-syntax shape-local-from-world (syntax-rules () ((_ s) (vector-ref s 1))))
(define-syntax shape-geometry         (syntax-rules () ((_ s) (vector-ref s 2))))
(define-syntax shape-material         (syntax-rules () ((_ s) (vector-ref s 3))))

(define (shape-sphere center radius material)
  (make-shape (mat4-mul (mat4-from-translation center)
                        (mat4-from-scale (vec3 radius radius radius)))
              'sphere material))
(define (shape-plane y material)
  (make-shape (mat4-from-translation (vec3 0.0 y 0.0)) 'plane material))

;; Intersect in the shape's local frame; the world-space lift is deferred so a
;; scan over many shapes pays for it only on the closest.
(define (shape-hit-local s ray)
  (let ((lfw (shape-local-from-world s)))
    (intersect-local (shape-geometry s)
                     (transform-point lfw (ray-origin ray))
                     (transform-vector lfw (ray-direction ray)))))

(define (shape-finish-hit s lh)
  (hit (local-hit-t lh)
       (transform-point (shape-world-from-local s) (local-hit-point lh))
       (normalize (transform-normal (shape-local-from-world s) (local-hit-normal lh)))
       (shape-material s)))

(define (shape-occluded? s ray t-min t-max)
  (let ((lfw (shape-local-from-world s)))
    (occluded-local (shape-geometry s)
                    (transform-point lfw (ray-origin ray))
                    (transform-vector lfw (ray-direction ray))
                    t-min t-max)))

;;; --- materials ------------------------------------------------------------

;; (material albedo :specular c :shininess n :reflectivity c :transparency c
;;           :ior n :abbe-number n :shader s); a missing shader is lambertian.
(define (material albedo . options)
  (define (option key default)
    (let loop ((o options))
      (cond ((null? o) default)
            ((eq? (car o) key) (cadr o))
            (else (loop (cddr o))))))
  (vector albedo
          (option :specular (vec3 0.5 0.5 0.5))
          (option :shininess 32.0)
          (option :reflectivity black)
          (option :transparency black)
          (option :ior 1.5)
          (option :abbe-number #f)
          (option :shader 'lambertian)))
(define-syntax material-albedo       (syntax-rules () ((_ m) (vector-ref m 0))))
(define-syntax material-specular     (syntax-rules () ((_ m) (vector-ref m 1))))
(define-syntax material-shininess    (syntax-rules () ((_ m) (vector-ref m 2))))
(define-syntax material-reflectivity (syntax-rules () ((_ m) (vector-ref m 3))))
(define-syntax material-transparency (syntax-rules () ((_ m) (vector-ref m 4))))
(define-syntax material-ior          (syntax-rules () ((_ m) (vector-ref m 5))))
(define-syntax material-abbe-number  (syntax-rules () ((_ m) (vector-ref m 6))))
(define-syntax material-shader       (syntax-rules () ((_ m) (vector-ref m 7))))

;;; --- lights ---------------------------------------------------------------

(define (point-light position intensity) (vector 'point position intensity))

;; Full INTENSITY inside CONE - PENUMBRA degrees of the axis, easing to zero at
;; CONE.  The cosines are derived once here, not per shading sample.
(define (spot-light position target intensity cone-deg penumbra-deg)
  (vector 'spot position intensity
          (normalize (v- target position))
          (cos (deg (max 0.0 (- cone-deg penumbra-deg))))
          (cos (deg cone-deg))))

(define-syntax light-position (syntax-rules () ((_ l) (vector-ref l 1))))

(define (light-intensity l point)
  (if (eq? (vector-ref l 0) 'point)
      (vector-ref l 2)
      (let* ((cos-theta (dot (vector-ref l 3) (normalize (v- point (light-position l)))))
             (cos-inner (vector-ref l 4))
             (cos-outer (vector-ref l 5))
             (band (- cos-inner cos-outer))
             ;; raylisp compares against single-float-epsilon.
             (t (cond ((> band 1.1920929e-7)
                       (max 0.0 (min 1.0 (/ (- cos-theta cos-outer) band))))
                      ((>= cos-theta cos-outer) 1.0)
                      (else 0.0))))
        (v* (vector-ref l 2) (* t t (- 3.0 (* 2.0 t)))))))

;;; --- scene ----------------------------------------------------------------

(define (make-scene camera shapes lights ambient)
  (vector camera shapes lights ambient
          (vec3 0.4 0.5 0.7)       ; sky at the horizon
          (vec3 0.08 0.18 0.5)     ; sky at the zenith
          8))                      ; recursion cap
(define-syntax scene-camera     (syntax-rules () ((_ s) (vector-ref s 0))))
(define-syntax scene-shapes     (syntax-rules () ((_ s) (vector-ref s 1))))
(define-syntax scene-lights     (syntax-rules () ((_ s) (vector-ref s 2))))
(define-syntax scene-ambient    (syntax-rules () ((_ s) (vector-ref s 3))))
(define-syntax scene-sky-bottom (syntax-rules () ((_ s) (vector-ref s 4))))
(define-syntax scene-sky-top    (syntax-rules () ((_ s) (vector-ref s 5))))
(define-syntax scene-max-depth  (syntax-rules () ((_ s) (vector-ref s 6))))

;; The closest hit over every shape, or #f.  No acceleration structure.
(define (scene-intersect scene ray)
  (let loop ((shapes (scene-shapes scene)) (best #f) (best-lh #f))
    (if (null? shapes)
        (and best (shape-finish-hit best best-lh))
        (let ((lh (shape-hit-local (car shapes) ray)))
          (if (and lh (or (not best-lh) (< (local-hit-t lh) (local-hit-t best-lh))))
              (loop (cdr shapes) (car shapes) lh)
              (loop (cdr shapes) best best-lh))))))

(define (scene-occluded? scene ray t-min t-max)
  (let loop ((shapes (scene-shapes scene)))
    (and (pair? shapes)
         (or (shape-occluded? (car shapes) ray t-min t-max)
             (loop (cdr shapes))))))

;;; --- camera ---------------------------------------------------------------

;; A pinhole camera at EYE looking at TARGET: (origin lower-left horizontal vertical).
(define (camera eye target up vfov-deg aspect)
  (let* ((h (tan (* 0.5 (deg vfov-deg))))
         (viewport-h (* 2.0 h))
         (viewport-w (* aspect viewport-h))
         (w (normalize-or-zero (v- eye target)))
         (u (normalize-or-zero (cross up w)))
         (v (cross w u))
         (horizontal (v* u viewport-w))
         (vertical (v* v viewport-h)))
    (vector eye
            (v- (v- (v- eye (v* horizontal 0.5)) (v* vertical 0.5)) w)
            horizontal vertical)))

;; The ray through (S, T) in [0, 1]^2, (0, 0) at the lower left.
(define (camera-ray cam s t)
  (let ((origin (vector-ref cam 0)))
    (make-ray origin
              (v- (v+ (v+ (vector-ref cam 1) (v* (vector-ref cam 2) s))
                      (v* (vector-ref cam 3) t))
                  origin))))

;;; --- shading --------------------------------------------------------------

(define shadow-t-min 1e-3)

;; Ambient floor plus, for each light that faces the surface and is not
;; blocked, albedo * intensity * N.L -- and with SPECULAR-MODEL phong or
;; blinn-phong, a specular highlight untinted by albedo.  The shadow ray uses
;; the un-normalised direction to the light, so the light sits at t = 1.
(define (shade-lit scene ray h specular-model)
  (let* ((mat (hit-material h))
         (albedo (material-albedo mat))
         (specular (material-specular mat))
         (shininess (material-shininess mat))
         (point (hit-point h))
         (normal (hit-normal h))
         (view-dir (normalize (v* (ray-direction ray) -1.0)))
         (ambient (scene-ambient scene))
         (ar (vec3-x albedo)) (ag (vec3-y albedo)) (ab (vec3-z albedo)))
    (let loop ((lights (scene-lights scene))
               (r (* (vec3-x ambient) ar))
               (g (* (vec3-y ambient) ag))
               (b (* (vec3-z ambient) ab)))
      (if (null? lights)
          (vec3 r g b)
          (let* ((light (car lights))
                 (to-light (v- (light-position light) point))
                 (to-light-dir (normalize to-light))
                 (cos-theta (dot normal to-light-dir))
                 (intensity (and (> cos-theta 0.0) (light-intensity light point))))
            (if (or (not intensity)
                    (color-zero? intensity)
                    (scene-occluded? scene (make-ray point to-light) shadow-t-min 1.0))
                (loop (cdr lights) r g b)
                (let* ((ir (vec3-x intensity)) (ig (vec3-y intensity)) (ib (vec3-z intensity))
                       (spec (case specular-model
                               ((phong)
                                (expt (max 0.0 (dot (reflect (v* to-light-dir -1.0) normal) view-dir))
                                      shininess))
                               ((blinn-phong)
                                (expt (max 0.0 (dot normal (normalize (v+ view-dir to-light-dir))))
                                      shininess))
                               (else 0.0))))
                  (loop (cdr lights)
                        (+ r (* ar ir cos-theta) (* (vec3-x specular) ir spec))
                        (+ g (* ag ig cos-theta) (* (vec3-y specular) ig spec))
                        (+ b (* ab ib cos-theta) (* (vec3-z specular) ib spec))))))))))

(define (shade scene ray h)
  (case (material-shader (hit-material h))
    ((normal-as-color) (v* (v+ (hit-normal h) (vec3 1.0 1.0 1.0)) 0.5))
    ((phong) (shade-lit scene ray h 'phong))
    ((blinn-phong) (shade-lit scene ray h 'blinn-phong))
    (else (shade-lit scene ray h 'none))))

;;; --- the recursive integrator ---------------------------------------------
;;;
;;;   L(P) = local(P) blended with reflectivity . L(reflected)
;;;                   then with transparency . L(refracted)

(define bounce-eps 1e-3)

(define (sky-color scene ray)
  (let ((a (* 0.5 (+ (vec3-y (normalize (ray-direction ray))) 1.0))))
    (v+ (v* (scene-sky-bottom scene) (- 1.0 a)) (v* (scene-sky-top scene) a))))

;; color . (1 - w) + w . incoming, per channel.
(define (blend c w in)
  (vec3 (+ (* (vec3-x c) (- 1.0 (vec3-x w))) (* (vec3-x w) (vec3-x in)))
        (+ (* (vec3-y c) (- 1.0 (vec3-y w))) (* (vec3-y w) (vec3-y in)))
        (+ (* (vec3-z c) (- 1.0 (vec3-z w))) (* (vec3-z w) (vec3-z in)))))

(define (radiance scene ray depth)
  (if (zero? depth)
      black
      (let ((h (scene-intersect scene ray)))
        (if (not h)
            (sky-color scene ray)
            (let* ((mat (hit-material h))
                   (local (shade scene ray h))
                   (k (material-reflectivity mat))
                   (tr (material-transparency mat)))
              (if (and (color-zero? k) (color-zero? tr))
                  local
                  (let* ((incident (normalize (ray-direction ray)))
                         (normal (hit-normal h))
                         (point (hit-point h))
                         (reflected
                          (if (color-zero? k)
                              local
                              (blend local k
                                     (radiance scene
                                               (make-ray (v+ point (v* normal bounce-eps))
                                                         (reflect incident normal))
                                               (- depth 1))))))
                    (if (color-zero? tr)
                        reflected
                        (blend reflected tr
                               (refracted-radiance scene mat incident normal point depth))))))))))

;; Entering or leaving is read off the sign of N.I: no ray state, so solid,
;; non-overlapping dielectrics only.  #f on total internal reflection.
(define (refract-dir incident normal ior)
  (let ((refracted (if (< (dot normal incident) 0.0)
                       (refract incident normal (/ 1.0 ior))
                       (refract incident (v* normal -1.0) ior))))
    (and (not (color-zero? refracted)) refracted)))

;; Per-channel IORs from Cauchy's n = A + B / lambda^2, fitted to IOR at the
;; d line and the Abbe number, evaluated at the C, d and F lines.
(define (dispersion-iors ior abbe)
  (let* ((lc 0.6563) (ld 0.5876) (lf 0.4861)
         (b (/ (- ior 1.0) (* abbe (- (/ 1.0 (* lf lf)) (/ 1.0 (* lc lc))))))
         (a (- ior (/ b (* ld ld)))))
    (define (n lambda) (+ a (/ b (* lambda lambda))))
    (values (n lc) (n ld) (n lf))))

;; The refracted ray, offset along its own direction since it crosses the
;; surface.  On TIR all of it reflects instead -- computed once and shared, as
;; the reflected direction does not depend on wavelength.  With an Abbe
;; number, one ray per channel, each taking its own channel.
(define (refracted-radiance scene mat incident normal point depth)
  (define tir #f)
  (define (tir-reflect)
    (unless tir
      (let ((rdir (reflect incident normal)))
        (set! tir (radiance scene (make-ray (v+ point (v* rdir bounce-eps)) rdir) (- depth 1)))))
    tir)
  (define (trace-channel ior)
    (let ((dir (refract-dir incident normal ior)))
      (and dir (radiance scene (make-ray (v+ point (v* dir bounce-eps)) dir) (- depth 1)))))
  (let ((abbe (material-abbe-number mat))
        (ior (material-ior mat)))
    (if abbe
        (let-values (((ior-r ior-g ior-b) (dispersion-iors ior abbe)))
          (vec3 (vec3-x (or (trace-channel ior-r) (tir-reflect)))
                (vec3-y (or (trace-channel ior-g) (tir-reflect)))
                (vec3-z (or (trace-channel ior-b) (tir-reflect)))))
        (or (trace-channel ior) (tir-reflect)))))

;;; --- the demo scene: raylisp's src/scenes.lisp, value for value -----------

(define (demo-scene aspect)
  (let* ((eye (vec3 0.0 0.0 1.6))
         (cam (camera eye (v+ eye (vec3 0.0 0.0 -1.0)) (vec3 0.0 1.0 0.0) 60.0 aspect))
         (white (vec3 1.0 1.0 1.0))
         (shapes
          (list
           ;; lambertian, cool blue, leftmost
           (shape-sphere (vec3 -2.45 -0.15 -2.6) 0.35
                         (material (vec3 0.3 0.5 0.9) :specular white :shininess 16.0
                                   :shader 'lambertian))
           ;; phong, warm red, and mostly-transparent dispersive glass
           (shape-sphere (vec3 -1.3 0.0 -2.1) 0.5
                         (material (vec3 0.9 0.3 0.3) :specular white :shininess 128.0
                                   :shader 'phong :transparency (vec3 0.9 0.9 0.9)
                                   :ior 1.5 :abbe-number 15.0))
           ;; bright orange, seen through the glass
           (shape-sphere (vec3 -1.72 0.0 -3.31) 0.5 (material (vec3 1.0 0.65 0.1)))
           ;; blinn-phong, green
           (shape-sphere (vec3 1.2 -0.1 -2.3) 0.4
                         (material (vec3 0.4 0.8 0.4) :specular white :shininess 180.0
                                   :shader 'blinn-phong))
           ;; mirror, dead centre
           (shape-sphere (vec3 0.0 0.3 -2.8) 0.8
                         (material (vec3 0.05 0.05 0.05) :reflectivity (vec3 0.9 0.9 0.9)))
           ;; normals as colour, rightmost
           (shape-sphere (vec3 2.5 -0.1 -2.4) 0.4
                         (material white :shader 'normal-as-color))
           (shape-plane -0.5 (material (vec3 0.6 0.6 0.6)))
           ;; backdrop: the unit rect stood upright, 6 x 3 at z = -5
           (make-shape (mat4-from-scale-rotation-translation
                        (vec3 3.0 1.0 1.5)
                        (quat-from-axis-angle (vec3 1.0 0.0 0.0) (deg 90.0))
                        (vec3 0.0 1.3 -5.0))
                       'rect (material (vec3 0.45 0.25 0.55)))
           ;; a box on the ground, turned 30 degrees about Y
           (make-shape (mat4-from-scale-rotation-translation
                        (vec3 0.35 0.25 0.25)
                        (quat-from-axis-angle (vec3 0.0 1.0 0.0) (deg 30.0))
                        (vec3 1.5 -0.25 -1.3))
                       'cuboid (material (vec3 0.2 0.6 0.6) :specular white :shininess 32.0))))
         (lights
          (list (point-light (vec3 3.0 3.0 1.0) (vec3 1.0 0.92 0.80))      ; warm key
                (point-light (vec3 -2.5 1.5 0.5) (vec3 0.30 0.40 0.55))    ; cool fill
                (spot-light (vec3 1.2 3.5 0.5) (vec3 1.2 -0.1 -2.3)        ; on the green sphere
                            (vec3 0.9 0.95 1.3) 18.0 6.0))))
    (make-scene cam shapes lights (vec3 0.1 0.1 0.1))))

;;; --- rendering and output -------------------------------------------------

;; One row of colours, one sample per pixel at its centre.  The film has y = 0
;; at the top, the camera t = 0 at the bottom.
(define (render-row scene width height y)
  (let ((cam (scene-camera scene))
        (depth (scene-max-depth scene))
        (row (make-vector width))
        (t (- 1.0 (/ (+ y 0.5) height))))
    (do ((x 0 (+ x 1))) ((= x width) row)
      (vector-set! row x (radiance scene (camera-ray cam (/ (+ x 0.5) width) t) depth)))))

;; Clamp to [0, 1], scale, round half up.
(define (to-u8 c) (exact (floor (+ (* (min 1.0 (max 0.0 c)) 255.0) 0.5))))

(define (write-row row port)
  (let ((n (vector-length row)))
    (do ((x 0 (+ x 1))) ((= x n))
      (let ((c (vector-ref row x)))
        (unless (zero? x) (write-char #\space port))
        (write (to-u8 (vec3-x c)) port) (write-char #\space port)
        (write (to-u8 (vec3-y c)) port) (write-char #\space port)
        (write (to-u8 (vec3-z c)) port)))
    (newline port)))

;;; --- command line ---------------------------------------------------------

(define (arg name default)
  (let loop ((args (cdr (command-line))))
    (cond ((or (null? args) (null? (cdr args))) default)
          ((string=? (car args) name) (cadr args))
          (else (loop (cdr args))))))

;; "AxB" or "A/B" as two integers.
(define (split-pair s sep)
  (let loop ((i 0))
    (cond ((= i (string-length s)) (error "expected two numbers separated by" sep s))
          ((char=? (string-ref s i) sep)
           (list (string->number (substring s 0 i))
                 (string->number (substring s (+ i 1) (string-length s)))))
          (else (loop (+ i 1))))))

(define (main)
  (let* ((size (split-pair (arg "--size" "1280x720") #\x))
         (width (car size)) (height (cadr size))
         (stripe-arg (arg "--stripe" #f))
         (stripe (if stripe-arg (split-pair stripe-arg #\/) '(0 1)))
         (k (car stripe)) (n (cadr stripe))
         (out (arg "--out" (if stripe-arg
                               (string-append "stripe-" (number->string k) ".txt")
                               "demo.ppm")))
         (scene (demo-scene (/ width height)))
         (start (current-jiffy))
         (rows (let loop ((y k) (acc '()))
                 (if (>= y height)
                     (reverse acc)
                     (loop (+ y n) (cons (render-row scene width height y) acc)))))
         (seconds (/ (- (current-jiffy) start) (jiffies-per-second)))
         (pixels (* width (length rows))))
    (call-with-output-file out
      (lambda (port)
        (unless stripe-arg
          (write-string "P3\n" port)
          (write width port) (write-char #\space port) (write height port)
          (write-string "\n255\n" port))
        (for-each (lambda (row) (write-row row port)) rows)))
    (display "rendered ") (display pixels) (display " px")
    (when stripe-arg (display " (stripe ") (display stripe-arg) (display ")"))
    (display " in ") (display (/ (round (* seconds 1000.0)) 1000.0)) (display " s, ")
    (display (/ (round (/ pixels seconds 1e4)) 100.0)) (display " Mpix/s -> ")
    (display out) (newline)))

(main)
