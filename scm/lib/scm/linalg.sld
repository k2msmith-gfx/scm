;;; (scm linalg) — vectors, matrices and quaternions for 3D.
;;;
;;; No SRFI covers this, and procedural 3D cannot do without it. The
;;; conventions below are not invented: they match ray, which is built on
;;; glam, so a value crossing the bridge needs no reinterpretation.
;;;
;;; ## Representation
;;;
;;; A **vec3** is a 3-element Scheme vector of reals, so `[0 1 0]` already
;;; is one (plan §4.5) and ray's `:at [0 1 0]` already passes one. A
;;; **vec4** is 4 elements. A **quaternion** is 4 elements ordered
;;; `x y z w`, w last, as glam stores it.
;;;
;;; A **mat4** is 16 elements, **column-major**, which is glam's layout --
;;; so the bridge is a copy rather than a transpose, and getting it
;;; backwards is the kind of error that renders a wrong picture without
;;; an error anywhere. Nothing needs to know that: `mat4-ref` takes a row
;;; and a column, and both `mat4-from-rows` and `mat4-from-columns` exist,
;;; so a matrix written out by hand reads the way it is drawn.
;;;
;;; ## Angles are radians
;;;
;;; Every procedure here takes and returns radians, because that is what
;;; R7RS's trigonometry uses. **ray's API is in degrees**, so a scene verb's
;;; `:rotate [rx ry rz]` maps onto
;;;
;;;     (quat-from-euler-xyz (degrees->radians rx)
;;;                          (degrees->radians ry)
;;;                          (degrees->radians rz))
;;;
;;; and that `xyz` is load-bearing: it is the product Rx·Ry·Rz, so **Z is
;;; applied to the vector first**. Assuming the other order gives mirrored
;;; attitudes, and it cost ray two wrong renders before anyone noticed.
;;;
;;; ## Two layers, and why
;;;
;;; Every operation exists twice.
;;;
;;; The **vector layer** -- `v3-dot`, `v3-cross`, `mat4*` -- takes and
;;; returns vectors. It is what scene code should write.
;;;
;;; The **component layer**, named with a trailing `*`, takes loose scalars:
;;; `(v3-dot* ax ay az bx by bz)`. It exists because the leaf compiler
;;; (plan §6) will only compile a procedure whose parameters are scalars
;;; and whose result is one or three `f64`s; a sequence may be the *return*
;;; shape but never a parameter and never an intermediate. So the vector
;;; layer cannot be compiled and the component layer can, at 199× the
;;; bytecode VM on the texture that was measured. `scm-llvm/tests/
;;; leaf_shapes.rs` pins that boundary.
;;;
;;; The rule for an author: write the readable form, and if a loop turns
;;; out to be hot, rewrite that loop against the `*` layer. The vector
;;; layer is defined in terms of the component layer, so there is one
;;; implementation of each piece of arithmetic and they cannot drift.
;;;
;;; The `*` layer covers the operations that actually turn up in inner
;;; loops, not all ninety: it is a performance escape hatch, not a second
;;; interface.
(define-library (scm linalg)
  (export
    ;; angles
    degrees->radians radians->degrees
    ;; vec3, vector layer
    v3 v3? v3-x v3-y v3-z v3-ref
    v3+ v3- v3* v3/ v3-negate v3-scale
    v3-dot v3-cross v3-length v3-length-squared
    v3-distance v3-distance-squared
    v3-normalize v3-normalize-or v3-lerp
    v3-min v3-max v3-abs v3-clamp
    v3-reflect v3-refract v3-project v3-reject v3-angle
    v3=? v3-approximately=?
    v3->list list->v3 v3->f64vector f64vector->v3
    v3-zero v3-one v3-unit-x v3-unit-y v3-unit-z
    ;; vec3, component layer -- leaf-compilable
    v3-dot* v3-cross* v3-length* v3-length-squared*
    v3-distance* v3-distance-squared* v3-normalize*
    v3-add* v3-sub* v3-scale* v3-lerp* v3-reflect*
    ;; vec4
    v4 v4? v4-x v4-y v4-z v4-w v4-ref
    v4+ v4- v4-scale v4-dot v4-length v4-normalize
    v4->list list->v4 v3->v4 v4->v3
    ;; mat4
    mat4? mat4-identity mat4-ref mat4-from-rows mat4-from-columns
    mat4->list mat4-row mat4-column
    mat4* mat4-transpose mat4-determinant mat4-inverse
    mat4-translation mat4-scaling mat4-uniform-scaling
    mat4-rotation-x mat4-rotation-y mat4-rotation-z
    mat4-rotation-axis-angle mat4-rotation-xyz mat4-from-quaternion
    mat4-compose mat4-decompose
    mat4-transform-point mat4-transform-direction mat4-transform-normal
    mat4-look-at mat4-perspective mat4-orthographic
    mat4-approximately=?
    ;; quaternions
    quat quat? quat-x quat-y quat-z quat-w quat-identity
    quat-from-axis-angle quat-from-euler-xyz quat-to-euler-xyz
    quat-from-to
    quat* quat-conjugate quat-inverse quat-normalize
    quat-length quat-dot quat-rotate quat-slerp
    quat->mat4 mat4->quat quat->list list->quat
    quat-approximately=?)
  (import (scheme base) (scheme inexact) (srfi 4))
  (begin

    ;; ── angles ───────────────────────────────────────────────────────
    (define %pi (* 4.0 (atan 1.0)))
    (define (degrees->radians d) (* (exact->inexact d) (/ %pi 180.0)))
    (define (radians->degrees r) (* (exact->inexact r) (/ 180.0 %pi)))

    (define (%f x) (exact->inexact x))

    ;; ── the component layer: what the leaf compiler can compile ──────
    ;; Scalars in; a scalar or three out. No sequence is ever a parameter
    ;; or an intermediate, which is the whole constraint (see the header).
    (define (v3-dot* ax ay az bx by bz)
      (+ (* ax bx) (* ay by) (* az bz)))

    (define (v3-length-squared* x y z) (+ (* x x) (* y y) (* z z)))

    (define (v3-length* x y z) (sqrt (v3-length-squared* x y z)))

    (define (v3-cross* ax ay az bx by bz)
      (vector (- (* ay bz) (* az by))
              (- (* az bx) (* ax bz))
              (- (* ax by) (* ay bx))))

    (define (v3-distance-squared* ax ay az bx by bz)
      (v3-length-squared* (- ax bx) (- ay by) (- az bz)))

    (define (v3-distance* ax ay az bx by bz)
      (sqrt (v3-distance-squared* ax ay az bx by bz)))

    ;; A zero vector has no direction, so it is returned unchanged rather
    ;; than divided by zero. `v3-normalize` raises instead; this one is
    ;; the branchless-caller version a loop wants.
    (define (v3-normalize* x y z)
      (let ((n (v3-length* x y z)))
        (if (= n 0.0)
            (vector 0.0 0.0 0.0)
            (vector (/ x n) (/ y n) (/ z n)))))

    (define (v3-add* ax ay az bx by bz) (vector (+ ax bx) (+ ay by) (+ az bz)))
    (define (v3-sub* ax ay az bx by bz) (vector (- ax bx) (- ay by) (- az bz)))
    (define (v3-scale* x y z k) (vector (* x k) (* y k) (* z k)))

    (define (v3-lerp* ax ay az bx by bz t)
      (vector (+ ax (* t (- bx ax)))
              (+ ay (* t (- by ay)))
              (+ az (* t (- bz az)))))

    ;; The mirror of `d` about the plane with unit normal `n`:
    ;; d - 2(d·n)n. The normal must be unit length, which is the caller's
    ;; job -- normalising here would double the cost of every bounce.
    (define (v3-reflect* dx dy dz nx ny nz)
      (let ((k (* 2.0 (v3-dot* dx dy dz nx ny nz))))
        (vector (- dx (* k nx)) (- dy (* k ny)) (- dz (* k nz)))))

    ;; ── vec3: the readable layer ─────────────────────────────────────
    (define (v3 x y z) (vector (%f x) (%f y) (%f z)))

    (define (v3? v)
      (and (vector? v) (= 3 (vector-length v))
           (real? (vector-ref v 0)) (real? (vector-ref v 1))
           (real? (vector-ref v 2))))

    (define (v3-x v) (vector-ref v 0))
    (define (v3-y v) (vector-ref v 1))
    (define (v3-z v) (vector-ref v 2))
    (define (v3-ref v i) (vector-ref v i))

    ;; Variadic, because summing offsets is the common case and
    ;; `(v3+ a b c)` beats nesting.
    (define (v3+ a . rest)
      (let loop ((acc a) (vs rest))
        (if (null? vs)
            acc
            (loop (v3-add* (v3-x acc) (v3-y acc) (v3-z acc)
                           (v3-x (car vs)) (v3-y (car vs)) (v3-z (car vs)))
                  (cdr vs)))))

    ;; One argument negates, as `-` does on numbers.
    (define (v3- a . rest)
      (if (null? rest)
          (v3-negate a)
          (let loop ((acc a) (vs rest))
            (if (null? vs)
                acc
                (loop (v3-sub* (v3-x acc) (v3-y acc) (v3-z acc)
                               (v3-x (car vs)) (v3-y (car vs)) (v3-z (car vs)))
                      (cdr vs))))))

    ;; Componentwise, which is what a colour or a per-axis scale wants.
    (define (v3* a . rest)
      (let loop ((acc a) (vs rest))
        (if (null? vs)
            acc
            (let ((b (car vs)))
              (loop (vector (* (v3-x acc) (v3-x b))
                            (* (v3-y acc) (v3-y b))
                            (* (v3-z acc) (v3-z b)))
                    (cdr vs))))))

    (define (v3/ a b)
      (vector (/ (v3-x a) (v3-x b)) (/ (v3-y a) (v3-y b)) (/ (v3-z a) (v3-z b))))

    (define (v3-negate a) (vector (- (v3-x a)) (- (v3-y a)) (- (v3-z a))))

    (define (v3-scale a k) (v3-scale* (v3-x a) (v3-y a) (v3-z a) (%f k)))

    (define (v3-dot a b)
      (v3-dot* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))

    (define (v3-cross a b)
      (v3-cross* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))

    (define (v3-length a) (v3-length* (v3-x a) (v3-y a) (v3-z a)))

    (define (v3-length-squared a)
      (v3-length-squared* (v3-x a) (v3-y a) (v3-z a)))

    (define (v3-distance a b)
      (v3-distance* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))

    (define (v3-distance-squared a b)
      (v3-distance-squared* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))

    ;; Raises on a zero vector: silently returning zero would make a
    ;; mis-set direction look like a legal one.
    (define (v3-normalize a)
      (let ((n (v3-length a)))
        (if (= n 0.0)
            (error "v3-normalize: the zero vector has no direction")
            (v3-scale a (/ 1.0 n)))))

    (define (v3-normalize-or a fallback)
      (if (= 0.0 (v3-length a)) fallback (v3-normalize a)))

    (define (v3-lerp a b t)
      (v3-lerp* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b) (%f t)))

    (define (v3-min a b)
      (vector (min (v3-x a) (v3-x b)) (min (v3-y a) (v3-y b)) (min (v3-z a) (v3-z b))))

    (define (v3-max a b)
      (vector (max (v3-x a) (v3-x b)) (max (v3-y a) (v3-y b)) (max (v3-z a) (v3-z b))))

    (define (v3-abs a)
      (vector (abs (v3-x a)) (abs (v3-y a)) (abs (v3-z a))))

    (define (v3-clamp a lo hi) (v3-min (v3-max a lo) hi))

    (define (v3-reflect d n)
      (v3-reflect* (v3-x d) (v3-y d) (v3-z d) (v3-x n) (v3-y n) (v3-z n)))

    ;; Snell's law. Returns #f on total internal reflection rather than a
    ;; NaN vector, so the caller has to decide what to do about it.
    (define (v3-refract d n eta)
      (let* ((e (%f eta))
             (cosi (- (v3-dot d n)))
             (k (- 1.0 (* e e (- 1.0 (* cosi cosi))))))
        (if (< k 0.0)
            #f
            (v3+ (v3-scale d e)
                 (v3-scale n (- (* e cosi) (sqrt k)))))))

    ;; The part of `a` along `onto`, and the part perpendicular to it.
    (define (v3-project a onto)
      (let ((d (v3-length-squared onto)))
        (if (= d 0.0)
            (error "v3-project: cannot project onto the zero vector")
            (v3-scale onto (/ (v3-dot a onto) d)))))

    (define (v3-reject a onto) (v3- a (v3-project a onto)))

    ;; Through the dot product of the normalised pair, clamped: rounding
    ;; can push it just past ±1 and `acos` would then be a NaN.
    (define (v3-angle a b)
      (let ((la (v3-length a)) (lb (v3-length b)))
        (if (or (= la 0.0) (= lb 0.0))
            (error "v3-angle: the zero vector has no direction")
            (acos (max -1.0 (min 1.0 (/ (v3-dot a b) (* la lb))))))))

    (define (v3=? a b)
      (and (= (v3-x a) (v3-x b)) (= (v3-y a) (v3-y b)) (= (v3-z a) (v3-z b))))

    ;; Floating-point results are never exactly equal, so this is what a
    ;; test should use.
    (define (v3-approximately=? a b . opt)
      (let ((eps (if (pair? opt) (car opt) 1e-9)))
        (and (< (abs (- (v3-x a) (v3-x b))) eps)
             (< (abs (- (v3-y a) (v3-y b))) eps)
             (< (abs (- (v3-z a) (v3-z b))) eps))))

    (define (v3->list a) (list (v3-x a) (v3-y a) (v3-z a)))
    (define (list->v3 l) (v3 (car l) (cadr l) (caddr l)))
    (define (v3->f64vector a) (f64vector (v3-x a) (v3-y a) (v3-z a)))
    (define (f64vector->v3 a)
      (v3 (f64vector-ref a 0) (f64vector-ref a 1) (f64vector-ref a 2)))

    ;; Fresh each time rather than shared constants: a vector is mutable,
    ;; and a shared `v3-zero` that someone wrote into would corrupt every
    ;; later use of it.
    (define (v3-zero) (vector 0.0 0.0 0.0))
    (define (v3-one) (vector 1.0 1.0 1.0))
    (define (v3-unit-x) (vector 1.0 0.0 0.0))
    (define (v3-unit-y) (vector 0.0 1.0 0.0))
    (define (v3-unit-z) (vector 0.0 0.0 1.0))

    ;; ── vec4 ─────────────────────────────────────────────────────────
    (define (v4 x y z w) (vector (%f x) (%f y) (%f z) (%f w)))
    (define (v4? v) (and (vector? v) (= 4 (vector-length v))))
    (define (v4-x v) (vector-ref v 0))
    (define (v4-y v) (vector-ref v 1))
    (define (v4-z v) (vector-ref v 2))
    (define (v4-w v) (vector-ref v 3))
    (define (v4-ref v i) (vector-ref v i))

    (define (v4+ a b)
      (vector (+ (v4-x a) (v4-x b)) (+ (v4-y a) (v4-y b))
              (+ (v4-z a) (v4-z b)) (+ (v4-w a) (v4-w b))))

    (define (v4- a b)
      (vector (- (v4-x a) (v4-x b)) (- (v4-y a) (v4-y b))
              (- (v4-z a) (v4-z b)) (- (v4-w a) (v4-w b))))

    (define (v4-scale a k)
      (let ((k (%f k)))
        (vector (* (v4-x a) k) (* (v4-y a) k) (* (v4-z a) k) (* (v4-w a) k))))

    (define (v4-dot a b)
      (+ (* (v4-x a) (v4-x b)) (* (v4-y a) (v4-y b))
         (* (v4-z a) (v4-z b)) (* (v4-w a) (v4-w b))))

    (define (v4-length a) (sqrt (v4-dot a a)))

    (define (v4-normalize a)
      (let ((n (v4-length a)))
        (if (= n 0.0)
            (error "v4-normalize: the zero vector has no direction")
            (v4-scale a (/ 1.0 n)))))

    (define (v4->list a) (list (v4-x a) (v4-y a) (v4-z a) (v4-w a)))
    (define (list->v4 l) (v4 (car l) (cadr l) (caddr l) (cadddr l)))
    (define (v3->v4 a w) (v4 (v3-x a) (v3-y a) (v3-z a) w))
    (define (v4->v3 a) (v3 (v4-x a) (v4-y a) (v4-z a)))

    ;; ── mat4 ─────────────────────────────────────────────────────────
    ;; Sixteen elements, COLUMN-major: element (row r, column c) lives at
    ;; c*4+r, which is glam's layout. Only the three procedures below know
    ;; that; everything else goes through them.
    (define (%ix r c) (+ (* c 4) r))
    (define (mat4-ref m r c) (vector-ref m (%ix r c)))
    (define (%set! m r c x) (vector-set! m (%ix r c) x))

    (define (mat4? m) (and (vector? m) (= 16 (vector-length m))))

    (define (mat4-identity)
      (vector 1.0 0.0 0.0 0.0
              0.0 1.0 0.0 0.0
              0.0 0.0 1.0 0.0
              0.0 0.0 0.0 1.0))

    ;; Sixteen values as they are drawn on paper, row by row.
    (define (mat4-from-rows . xs)
      (let ((l (if (and (= 1 (length xs)) (list? (car xs))) (car xs) xs))
            (m (make-vector 16 0.0)))
        (let loop ((i 0) (l l))
          (if (null? l)
              m
              (begin (%set! m (quotient i 4) (remainder i 4) (%f (car l)))
                     (loop (+ i 1) (cdr l)))))))

    ;; Sixteen values in storage order.
    (define (mat4-from-columns . xs)
      (let ((l (if (and (= 1 (length xs)) (list? (car xs))) (car xs) xs)))
        (list->vector (map %f l))))

    (define (mat4->list m) (vector->list m))

    (define (mat4-row m r)
      (v4 (mat4-ref m r 0) (mat4-ref m r 1) (mat4-ref m r 2) (mat4-ref m r 3)))

    (define (mat4-column m c)
      (v4 (mat4-ref m 0 c) (mat4-ref m 1 c) (mat4-ref m 2 c) (mat4-ref m 3 c)))

    ;; Variadic and left to right, so `(mat4* t r s)` is the usual
    ;; translate-rotate-scale reading: S applies to the point first.
    (define (mat4* a . rest)
      (let loop ((acc a) (ms rest))
        (if (null? ms) acc (loop (%mul acc (car ms)) (cdr ms)))))

    (define (%mul a b)
      (let ((m (make-vector 16 0.0)))
        (let rows ((r 0))
          (if (= r 4)
              m
              (begin
                (let cols ((c 0))
                  (when (< c 4)
                    (%set! m r c
                           (+ (* (mat4-ref a r 0) (mat4-ref b 0 c))
                              (* (mat4-ref a r 1) (mat4-ref b 1 c))
                              (* (mat4-ref a r 2) (mat4-ref b 2 c))
                              (* (mat4-ref a r 3) (mat4-ref b 3 c))))
                    (cols (+ c 1))))
                (rows (+ r 1)))))))

    (define (mat4-transpose m)
      (let ((out (make-vector 16 0.0)))
        (let rows ((r 0))
          (if (= r 4)
              out
              (begin
                (let cols ((c 0))
                  (when (< c 4)
                    (%set! out c r (mat4-ref m r c))
                    (cols (+ c 1))))
                (rows (+ r 1)))))))

    ;; Cofactor expansion along the first row, with 3x3 minors. Longer
    ;; than a row reduction but exact in the same way the formula is.
    (define (mat4-determinant m)
      (let ((c (lambda (r0 r1 r2 c0 c1 c2) (%det3 m r0 r1 r2 c0 c1 c2))))
        (- (+ (* (mat4-ref m 0 0) (c 1 2 3 1 2 3))
              (* (mat4-ref m 0 2) (c 1 2 3 0 1 3)))
           (+ (* (mat4-ref m 0 1) (c 1 2 3 0 2 3))
              (* (mat4-ref m 0 3) (c 1 2 3 0 1 2))))))

    (define (%det3 m r0 r1 r2 c0 c1 c2)
      (let ((a (mat4-ref m r0 c0)) (b (mat4-ref m r0 c1)) (c (mat4-ref m r0 c2))
            (d (mat4-ref m r1 c0)) (e (mat4-ref m r1 c1)) (f (mat4-ref m r1 c2))
            (g (mat4-ref m r2 c0)) (h (mat4-ref m r2 c1)) (i (mat4-ref m r2 c2)))
        (+ (* a (- (* e i) (* f h)))
           (- (* b (- (* d i) (* f g))))
           (* c (- (* d h) (* e g))))))

    ;; Gauss-Jordan with partial pivoting rather than the adjugate. The
    ;; pivoting is the point: a transform with a very small scale on one
    ;; axis inverts without the cancellation the closed form suffers, and
    ;; a singular matrix is reported rather than returning infinities.
    (define (mat4-inverse m)
      (let ((a (make-vector 32 0.0)))
        ;; [ m | I ], row-major in a 4x8 scratch
        (let fill ((r 0))
          (when (< r 4)
            (let cols ((c 0))
              (when (< c 4)
                (vector-set! a (+ (* r 8) c) (mat4-ref m r c))
                (vector-set! a (+ (* r 8) 4 c) (if (= r c) 1.0 0.0))
                (cols (+ c 1))))
            (fill (+ r 1))))
        (let elim ((col 0))
          (if (= col 4)
              (let ((out (make-vector 16 0.0)))
                (let gather ((r 0))
                  (if (= r 4)
                      out
                      (begin
                        (let cols ((c 0))
                          (when (< c 4)
                            (%set! out r c (vector-ref a (+ (* r 8) 4 c)))
                            (cols (+ c 1))))
                        (gather (+ r 1))))))
              (let ((pivot (%best-pivot a col)))
                (when (< (abs (vector-ref a (+ (* pivot 8) col))) 1e-14)
                  (error "mat4-inverse: the matrix is singular"))
                (%swap-rows! a col pivot)
                (%scale-row! a col (/ 1.0 (vector-ref a (+ (* col 8) col))))
                (let others ((r 0))
                  (when (< r 4)
                    (unless (= r col)
                      (%axpy! a r col (- (vector-ref a (+ (* r 8) col)))))
                    (others (+ r 1))))
                (elim (+ col 1)))))))

    (define (%best-pivot a col)
      (let loop ((r col) (best col) (bv (abs (vector-ref a (+ (* col 8) col)))))
        (if (= r 4)
            best
            (let ((v (abs (vector-ref a (+ (* r 8) col)))))
              (if (> v bv) (loop (+ r 1) r v) (loop (+ r 1) best bv))))))

    (define (%swap-rows! a r1 r2)
      (unless (= r1 r2)
        (let loop ((c 0))
          (when (< c 8)
            (let ((t (vector-ref a (+ (* r1 8) c))))
              (vector-set! a (+ (* r1 8) c) (vector-ref a (+ (* r2 8) c)))
              (vector-set! a (+ (* r2 8) c) t))
            (loop (+ c 1))))))

    (define (%scale-row! a r k)
      (let loop ((c 0))
        (when (< c 8)
          (vector-set! a (+ (* r 8) c) (* k (vector-ref a (+ (* r 8) c))))
          (loop (+ c 1)))))

    ;; row[r] += k * row[src]
    (define (%axpy! a r src k)
      (let loop ((c 0))
        (when (< c 8)
          (vector-set! a (+ (* r 8) c)
                       (+ (vector-ref a (+ (* r 8) c))
                          (* k (vector-ref a (+ (* src 8) c)))))
          (loop (+ c 1)))))

    ;; ── the constructors ─────────────────────────────────────────────
    (define (mat4-translation t)
      (mat4-from-rows 1.0 0.0 0.0 (v3-x t)
                      0.0 1.0 0.0 (v3-y t)
                      0.0 0.0 1.0 (v3-z t)
                      0.0 0.0 0.0 1.0))

    (define (mat4-scaling s)
      (mat4-from-rows (v3-x s) 0.0 0.0 0.0
                      0.0 (v3-y s) 0.0 0.0
                      0.0 0.0 (v3-z s) 0.0
                      0.0 0.0 0.0 1.0))

    (define (mat4-uniform-scaling k) (mat4-scaling (v3 k k k)))

    (define (mat4-rotation-x a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows 1.0 0.0 0.0 0.0
                        0.0 c (- s) 0.0
                        0.0 s c 0.0
                        0.0 0.0 0.0 1.0)))

    (define (mat4-rotation-y a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows c 0.0 s 0.0
                        0.0 1.0 0.0 0.0
                        (- s) 0.0 c 0.0
                        0.0 0.0 0.0 1.0)))

    (define (mat4-rotation-z a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows c (- s) 0.0 0.0
                        s c 0.0 0.0
                        0.0 0.0 1.0 0.0
                        0.0 0.0 0.0 1.0)))

    (define (mat4-rotation-axis-angle axis angle)
      (quat->mat4 (quat-from-axis-angle axis angle)))

    ;; The product Rx·Ry·Rz, so **Z is applied to the vector first**. This
    ;; is ray's `:rotate` convention and the order is in the name because
    ;; assuming the other one gives mirrored attitudes with no error.
    (define (mat4-rotation-xyz rx ry rz)
      (mat4* (mat4-rotation-x rx) (mat4-rotation-y ry) (mat4-rotation-z rz)))

    (define (mat4-from-quaternion q) (quat->mat4 q))

    ;; Translate · rotate · scale, which is the order that means "scale
    ;; the object, then orient it, then put it somewhere".
    (define (mat4-compose translation rotation scale)
      (mat4* (mat4-translation translation)
             (quat->mat4 rotation)
             (mat4-scaling scale)))

    ;; The inverse of `mat4-compose`, as three values. A negative
    ;; determinant means the transform mirrors, and a rotation cannot
    ;; mirror -- so the sign goes into the x scale, where `mat4-compose`
    ;; will put it back.
    (define (mat4-decompose m)
      (let* ((translation (v3 (mat4-ref m 0 3) (mat4-ref m 1 3) (mat4-ref m 2 3)))
             (cx (v3 (mat4-ref m 0 0) (mat4-ref m 1 0) (mat4-ref m 2 0)))
             (cy (v3 (mat4-ref m 0 1) (mat4-ref m 1 1) (mat4-ref m 2 1)))
             (cz (v3 (mat4-ref m 0 2) (mat4-ref m 1 2) (mat4-ref m 2 2)))
             (mirrored (< (v3-dot (v3-cross cx cy) cz) 0.0))
             (sx (let ((l (v3-length cx))) (if mirrored (- l) l)))
             (sy (v3-length cy))
             (sz (v3-length cz)))
        (if (or (= sx 0.0) (= sy 0.0) (= sz 0.0))
            (error "mat4-decompose: a zero scale axis leaves the rotation undefined")
            (let ((r (mat4-from-rows
                      (/ (mat4-ref m 0 0) sx) (/ (mat4-ref m 0 1) sy) (/ (mat4-ref m 0 2) sz) 0.0
                      (/ (mat4-ref m 1 0) sx) (/ (mat4-ref m 1 1) sy) (/ (mat4-ref m 1 2) sz) 0.0
                      (/ (mat4-ref m 2 0) sx) (/ (mat4-ref m 2 1) sy) (/ (mat4-ref m 2 2) sz) 0.0
                      0.0 0.0 0.0 1.0)))
              (values translation (mat4->quat r) (v3 sx sy sz))))))

    ;; ── applying a matrix ────────────────────────────────────────────
    ;; A point carries w=1, so the translation applies, and the result is
    ;; divided through by w -- which only matters for a projection, and
    ;; silently gives the wrong point if skipped.
    (define (mat4-transform-point m p)
      (let* ((x (v3-x p)) (y (v3-y p)) (z (v3-z p))
             (rx (+ (* (mat4-ref m 0 0) x) (* (mat4-ref m 0 1) y) (* (mat4-ref m 0 2) z) (mat4-ref m 0 3)))
             (ry (+ (* (mat4-ref m 1 0) x) (* (mat4-ref m 1 1) y) (* (mat4-ref m 1 2) z) (mat4-ref m 1 3)))
             (rz (+ (* (mat4-ref m 2 0) x) (* (mat4-ref m 2 1) y) (* (mat4-ref m 2 2) z) (mat4-ref m 2 3)))
             (rw (+ (* (mat4-ref m 3 0) x) (* (mat4-ref m 3 1) y) (* (mat4-ref m 3 2) z) (mat4-ref m 3 3))))
        (if (or (= rw 1.0) (= rw 0.0))
            (v3 rx ry rz)
            (v3 (/ rx rw) (/ ry rw) (/ rz rw)))))

    ;; A direction carries w=0, so the translation does NOT apply.
    (define (mat4-transform-direction m d)
      (let ((x (v3-x d)) (y (v3-y d)) (z (v3-z d)))
        (v3 (+ (* (mat4-ref m 0 0) x) (* (mat4-ref m 0 1) y) (* (mat4-ref m 0 2) z))
            (+ (* (mat4-ref m 1 0) x) (* (mat4-ref m 1 1) y) (* (mat4-ref m 1 2) z))
            (+ (* (mat4-ref m 2 0) x) (* (mat4-ref m 2 1) y) (* (mat4-ref m 2 2) z)))))

    ;; A normal transforms by the INVERSE TRANSPOSE, not by the matrix. A
    ;; non-uniform scale shears a surface, and a normal carried by the
    ;; matrix itself stops being perpendicular to it -- which shows up as
    ;; wrong shading rather than as an error.
    (define (mat4-transform-normal m n)
      (v3-normalize
       (mat4-transform-direction (mat4-transpose (mat4-inverse m)) n)))

    ;; ── the camera matrices ──────────────────────────────────────────
    ;; Right-handed, looking down -Z, matching glam's `look_at_rh`.
    (define (mat4-look-at eye target up)
      (let* ((f (v3-normalize (v3- target eye)))
             (s (v3-normalize (v3-cross f up)))
             (u (v3-cross s f)))
        (mat4-from-rows
         (v3-x s) (v3-y s) (v3-z s) (- (v3-dot s eye))
         (v3-x u) (v3-y u) (v3-z u) (- (v3-dot u eye))
         (- (v3-x f)) (- (v3-y f)) (- (v3-z f)) (v3-dot f eye)
         0.0 0.0 0.0 1.0)))

    ;; Right-handed, with depth mapped to [-1,1] -- the OpenGL convention.
    ;; An API that wants [0,1] needs a depth remap after this.
    (define (mat4-perspective fovy aspect near far)
      (let ((t (tan (/ fovy 2.0))))
        (mat4-from-rows
         (/ 1.0 (* aspect t)) 0.0 0.0 0.0
         0.0 (/ 1.0 t) 0.0 0.0
         0.0 0.0 (/ (+ far near) (- near far)) (/ (* 2.0 far near) (- near far))
         0.0 0.0 -1.0 0.0)))

    (define (mat4-orthographic left right bottom top near far)
      (mat4-from-rows
       (/ 2.0 (- right left)) 0.0 0.0 (- (/ (+ right left) (- right left)))
       0.0 (/ 2.0 (- top bottom)) 0.0 (- (/ (+ top bottom) (- top bottom)))
       0.0 0.0 (/ -2.0 (- far near)) (- (/ (+ far near) (- far near)))
       0.0 0.0 0.0 1.0))

    (define (mat4-approximately=? a b . opt)
      (let ((eps (if (pair? opt) (car opt) 1e-9)))
        (let loop ((i 0))
          (cond ((= i 16) #t)
                ((< (abs (- (vector-ref a i) (vector-ref b i))) eps) (loop (+ i 1)))
                (else #f)))))

    ;; ── quaternions ──────────────────────────────────────────────────
    ;; Ordered x y z w, **w last**, as glam stores it.
    (define (quat x y z w) (vector (%f x) (%f y) (%f z) (%f w)))
    (define (quat? q) (and (vector? q) (= 4 (vector-length q))))
    (define (quat-x q) (vector-ref q 0))
    (define (quat-y q) (vector-ref q 1))
    (define (quat-z q) (vector-ref q 2))
    (define (quat-w q) (vector-ref q 3))
    (define (quat-identity) (vector 0.0 0.0 0.0 1.0))

    (define (quat-from-axis-angle axis angle)
      (let* ((u (v3-normalize axis))
             (h (/ (%f angle) 2.0))
             (s (sin h)))
        (quat (* s (v3-x u)) (* s (v3-y u)) (* s (v3-z u)) (cos h))))

    ;; The product Rx·Ry·Rz: Z is applied to the vector first. ray's
    ;; `:rotate` convention, in degrees at its API boundary.
    (define (quat-from-euler-xyz rx ry rz)
      (quat* (quat-from-axis-angle (v3-unit-x) rx)
             (quat-from-axis-angle (v3-unit-y) ry)
             (quat-from-axis-angle (v3-unit-z) rz)))

    ;; The inverse, as three values, read off the rotation matrix. For
    ;; R = Rx(a)·Ry(b)·Rz(c) the entries are
    ;;
    ;;     m02 = sin b        m12 = -sin a·cos b   m22 = cos a·cos b
    ;;     m00 = cos b·cos c  m01 = -cos b·sin c
    ;;
    ;; so each angle comes from an `atan` of two entries rather than from
    ;; a chain of quaternion products, which is both shorter and checkable
    ;; against the matrix the other direction builds.
    ;;
    ;; At a pitch of ±90° cos b is zero, every one of those entries
    ;; vanishes, and only the SUM or DIFFERENCE of the other two angles is
    ;; determined -- gimbal lock. There is no way to recover both, so the
    ;; roll is set to zero and the whole rotation is put in x, which gives
    ;; back the same rotation even though the angles differ.
    (define (quat-to-euler-xyz q)
      (let* ((m (quat->mat4 (quat-normalize q)))
             (m02 (mat4-ref m 0 2))
             (b (asin (max -1.0 (min 1.0 m02)))))
        (if (> (abs m02) 0.9999999)
            (let ((m10 (mat4-ref m 1 0)) (m11 (mat4-ref m 1 1)))
              (values (if (> m02 0.0) (atan m10 m11) (atan (- m10) m11))
                      b
                      0.0))
            (values (atan (- (mat4-ref m 1 2)) (mat4-ref m 2 2))
                    b
                    (atan (- (mat4-ref m 0 1)) (mat4-ref m 0 0))))))

    ;; The shortest rotation taking direction `from` to direction `to`.
    ;; The antiparallel case has no unique answer, so any perpendicular
    ;; axis is chosen rather than producing a zero quaternion.
    (define (quat-from-to from to)
      (let* ((a (v3-normalize from))
             (b (v3-normalize to))
             (d (v3-dot a b)))
        (cond ((> d 0.999999) (quat-identity))
              ((< d -0.999999)
               (let ((axis (let ((try (v3-cross a (v3-unit-x))))
                             (if (< (v3-length try) 1e-6)
                                 (v3-cross a (v3-unit-y))
                                 try))))
                 (quat-from-axis-angle axis %pi)))
              (else
               (let ((c (v3-cross a b)))
                 (quat-normalize
                  (quat (v3-x c) (v3-y c) (v3-z c) (+ 1.0 d))))))))

    ;; Composition: `(quat* a b)` applied to a vector is a applied after b.
    (define (quat* a . rest)
      (let loop ((acc a) (qs rest))
        (if (null? qs) acc (loop (%qmul acc (car qs)) (cdr qs)))))

    (define (%qmul a b)
      (let ((ax (quat-x a)) (ay (quat-y a)) (az (quat-z a)) (aw (quat-w a))
            (bx (quat-x b)) (by (quat-y b)) (bz (quat-z b)) (bw (quat-w b)))
        (quat (+ (* aw bx) (* ax bw) (* ay bz) (- (* az by)))
              (+ (* aw by) (- (* ax bz)) (* ay bw) (* az bx))
              (+ (* aw bz) (* ax by) (- (* ay bx)) (* az bw))
              (- (* aw bw) (* ax bx) (* ay by) (* az bz)))))

    (define (quat-conjugate q)
      (quat (- (quat-x q)) (- (quat-y q)) (- (quat-z q)) (quat-w q)))

    (define (quat-dot a b)
      (+ (* (quat-x a) (quat-x b)) (* (quat-y a) (quat-y b))
         (* (quat-z a) (quat-z b)) (* (quat-w a) (quat-w b))))

    (define (quat-length q) (sqrt (quat-dot q q)))

    (define (quat-normalize q)
      (let ((n (quat-length q)))
        (if (= n 0.0)
            (error "quat-normalize: the zero quaternion is not a rotation")
            (quat (/ (quat-x q) n) (/ (quat-y q) n) (/ (quat-z q) n) (/ (quat-w q) n)))))

    ;; For a UNIT quaternion the inverse is the conjugate, which is what
    ;; makes rotating by one cheap. A non-unit one is divided through.
    (define (quat-inverse q)
      (let ((d (quat-dot q q)))
        (if (= d 0.0)
            (error "quat-inverse: the zero quaternion has no inverse")
            (let ((c (quat-conjugate q)))
              (if (< (abs (- d 1.0)) 1e-12)
                  c
                  (quat (/ (quat-x c) d) (/ (quat-y c) d)
                        (/ (quat-z c) d) (/ (quat-w c) d)))))))

    ;; v + 2·(qv × (qv × v + w·v)), which is the sandwich product without
    ;; building the two intermediate quaternions.
    (define (quat-rotate q v)
      (let* ((qv (v3 (quat-x q) (quat-y q) (quat-z q)))
             (w (quat-w q))
             (t (v3+ (v3-cross qv v) (v3-scale v w))))
        (v3+ v (v3-scale (v3-cross qv t) 2.0))))

    ;; Shortest-path: if the pair points more than 90° apart, one is
    ;; negated first, because q and -q are the same rotation but
    ;; interpolating between them takes the long way round. Nearly
    ;; parallel inputs fall back to a straight lerp, where the sine of the
    ;; angle is too small to divide by.
    (define (quat-slerp a b t)
      (let* ((t (%f t))
             (d0 (quat-dot a b))
             (flip (< d0 0.0))
             (b (if flip (quat (- (quat-x b)) (- (quat-y b))
                               (- (quat-z b)) (- (quat-w b)))
                    b))
             (d (abs d0)))
        (if (> d 0.9995)
            (quat-normalize
             (quat (+ (quat-x a) (* t (- (quat-x b) (quat-x a))))
                   (+ (quat-y a) (* t (- (quat-y b) (quat-y a))))
                   (+ (quat-z a) (* t (- (quat-z b) (quat-z a))))
                   (+ (quat-w a) (* t (- (quat-w b) (quat-w a))))))
            (let* ((theta (acos (max -1.0 (min 1.0 d))))
                   (s (sin theta))
                   (wa (/ (sin (* (- 1.0 t) theta)) s))
                   (wb (/ (sin (* t theta)) s)))
              (quat (+ (* wa (quat-x a)) (* wb (quat-x b)))
                    (+ (* wa (quat-y a)) (* wb (quat-y b)))
                    (+ (* wa (quat-z a)) (* wb (quat-z b)))
                    (+ (* wa (quat-w a)) (* wb (quat-w b))))))))

    (define (quat->mat4 q)
      (let* ((n (quat-normalize q))
             (x (quat-x n)) (y (quat-y n)) (z (quat-z n)) (w (quat-w n))
             (xx (* x x)) (yy (* y y)) (zz (* z z))
             (xy (* x y)) (xz (* x z)) (yz (* y z))
             (wx (* w x)) (wy (* w y)) (wz (* w z)))
        (mat4-from-rows
         (- 1.0 (* 2.0 (+ yy zz))) (* 2.0 (- xy wz)) (* 2.0 (+ xz wy)) 0.0
         (* 2.0 (+ xy wz)) (- 1.0 (* 2.0 (+ xx zz))) (* 2.0 (- yz wx)) 0.0
         (* 2.0 (- xz wy)) (* 2.0 (+ yz wx)) (- 1.0 (* 2.0 (+ xx yy))) 0.0
         0.0 0.0 0.0 1.0)))

    ;; Shepperd's method: take the largest of the four candidate
    ;; denominators. Using one fixed formula loses all precision when its
    ;; denominator is near zero, which happens at a 180° rotation.
    (define (mat4->quat m)
      (let ((m00 (mat4-ref m 0 0)) (m01 (mat4-ref m 0 1)) (m02 (mat4-ref m 0 2))
            (m10 (mat4-ref m 1 0)) (m11 (mat4-ref m 1 1)) (m12 (mat4-ref m 1 2))
            (m20 (mat4-ref m 2 0)) (m21 (mat4-ref m 2 1)) (m22 (mat4-ref m 2 2)))
        (let ((trace (+ m00 m11 m22)))
          (cond ((> trace 0.0)
                 (let* ((s (sqrt (+ trace 1.0))) (k (/ 0.5 s)))
                   (quat (* k (- m21 m12)) (* k (- m02 m20)) (* k (- m10 m01)) (* 0.5 s))))
                ((and (>= m00 m11) (>= m00 m22))
                 (let* ((s (sqrt (+ 1.0 m00 (- m11) (- m22)))) (k (/ 0.5 s)))
                   (quat (* 0.5 s) (* k (+ m01 m10)) (* k (+ m02 m20)) (* k (- m21 m12)))))
                ((>= m11 m22)
                 (let* ((s (sqrt (+ 1.0 m11 (- m00) (- m22)))) (k (/ 0.5 s)))
                   (quat (* k (+ m01 m10)) (* 0.5 s) (* k (+ m12 m21)) (* k (- m02 m20)))))
                (else
                 (let* ((s (sqrt (+ 1.0 m22 (- m00) (- m11)))) (k (/ 0.5 s)))
                   (quat (* k (+ m02 m20)) (* k (+ m12 m21)) (* 0.5 s) (* k (- m10 m01)))))))))

    (define (quat->list q) (list (quat-x q) (quat-y q) (quat-z q) (quat-w q)))
    (define (list->quat l) (quat (car l) (cadr l) (caddr l) (cadddr l)))

    ;; q and -q are the same rotation, so this accepts either sign.
    (define (quat-approximately=? a b . opt)
      (let ((eps (if (pair? opt) (car opt) 1e-9)))
        (define (close? a b)
          (and (< (abs (- (quat-x a) (quat-x b))) eps)
               (< (abs (- (quat-y a) (quat-y b))) eps)
               (< (abs (- (quat-z a) (quat-z b))) eps)
               (< (abs (- (quat-w a) (quat-w b))) eps)))
        (or (close? a b)
            (close? a (quat (- (quat-x b)) (- (quat-y b))
                            (- (quat-z b)) (- (quat-w b)))))))))
