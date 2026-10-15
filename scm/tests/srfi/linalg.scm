;;; (scm linalg) — vectors, matrices and quaternions.
;;;
;;; The checks that matter here are the ones that catch a SILENTLY wrong
;;; picture: handedness, the Euler order ray depends on, a normal under
;;; non-uniform scale, and the two layers agreeing.
(import (scm test) (scm linalg) (scheme base) (scheme inexact))

(define (close? a b) (< (abs (- a b)) 1e-9))
(define pi (* 4.0 (atan 1.0)))
(define (deg d) (degrees->radians d))

;; ── angles ───────────────────────────────────────────────────────────
(check degrees (close? (degrees->radians 180) pi) #t)
(check radians (close? (radians->degrees pi) 180.0) #t)
(check round-trip-angle (close? (radians->degrees (degrees->radians 37)) 37.0) #t)

;; ── vec3 basics ──────────────────────────────────────────────────────
(check ctor (v3->list (v3 1 2 3)) '(1.0 2.0 3.0))
(check ctor-coerces-exact (v3->list (v3 1 2 3)) '(1.0 2.0 3.0))
(check pred (list (v3? (v3 1 2 3)) (v3? (vector 1 2)) (v3? 5)) '(#t #f #f))
;; A bracket literal already is a vec3, which is the point of the
;; representation (plan §4.5).
(check bracket-is-a-v3 (v3? [1.0 2.0 3.0]) #t)
(check accessors (list (v3-x (v3 1 2 3)) (v3-y (v3 1 2 3)) (v3-z (v3 1 2 3)))
       '(1.0 2.0 3.0))
(check add (v3->list (v3+ (v3 1 2 3) (v3 10 20 30))) '(11.0 22.0 33.0))
(check add-variadic (v3->list (v3+ (v3 1 0 0) (v3 0 1 0) (v3 0 0 1))) '(1.0 1.0 1.0))
(check sub (v3->list (v3- (v3 5 5 5) (v3 1 2 3))) '(4.0 3.0 2.0))
;; One argument negates, as `-` does on numbers.
(check sub-negates (v3->list (v3- (v3 1 2 3))) '(-1.0 -2.0 -3.0))
(check mul-componentwise (v3->list (v3* (v3 2 3 4) (v3 10 10 10))) '(20.0 30.0 40.0))
(check div-componentwise (v3->list (v3/ (v3 10 20 30) (v3 2 4 5))) '(5.0 5.0 6.0))
(check negate (v3->list (v3-negate (v3 1 -2 3))) '(-1.0 2.0 -3.0))
(check scale (v3->list (v3-scale (v3 1 2 3) 2)) '(2.0 4.0 6.0))
(check dot (v3-dot (v3 1 2 3) (v3 4 5 6)) 32.0)
(check dot-of-perpendicular (v3-dot (v3-unit-x) (v3-unit-y)) 0.0)
(check length- (v3-length (v3 3 4 0)) 5.0)
(check length-squared (v3-length-squared (v3 3 4 0)) 25.0)
(check distance (v3-distance (v3 0 0 0) (v3 3 4 0)) 5.0)
(check distance-squared (v3-distance-squared (v3 0 0 0) (v3 3 4 0)) 25.0)
(check normalize (v3->list (v3-normalize (v3 0 5 0))) '(0.0 1.0 0.0))
(check normalize-is-unit (close? (v3-length (v3-normalize (v3 1 2 3))) 1.0) #t)
;; A zero direction is an error, not a silent zero: a mis-set direction
;; must not look like a legal one.
(check-raises normalize-rejects-zero (v3-normalize (v3 0 0 0)))
(check normalize-or (v3->list (v3-normalize-or (v3 0 0 0) (v3-unit-y))) '(0.0 1.0 0.0))
(check lerp-endpoints
       (list (v3->list (v3-lerp (v3 0 0 0) (v3 10 10 10) 0))
             (v3->list (v3-lerp (v3 0 0 0) (v3 10 10 10) 1)))
       '((0.0 0.0 0.0) (10.0 10.0 10.0)))
(check lerp-middle (v3->list (v3-lerp (v3 0 0 0) (v3 10 20 30) 0.5)) '(5.0 10.0 15.0))
(check min- (v3->list (v3-min (v3 1 5 3) (v3 4 2 3))) '(1.0 2.0 3.0))
(check max- (v3->list (v3-max (v3 1 5 3) (v3 4 2 3))) '(4.0 5.0 3.0))
(check abs- (v3->list (v3-abs (v3 -1 2 -3))) '(1.0 2.0 3.0))
(check clamp (v3->list (v3-clamp (v3 -5 0.5 9) (v3 0 0 0) (v3 1 1 1))) '(0.0 0.5 1.0))
(check equality (list (v3=? (v3 1 2 3) (v3 1 2 3)) (v3=? (v3 1 2 3) (v3 1 2 4)))
       '(#t #f))
(check approx-equality
       (list (v3-approximately=? (v3 1 2 3) (v3 1 2 3.0000000001))
             (v3-approximately=? (v3 1 2 3) (v3 1 2 3.1)))
       '(#t #f))
(check to-from-list (v3->list (list->v3 '(1 2 3))) '(1.0 2.0 3.0))
(check to-from-f64vector (v3->list (f64vector->v3 (v3->f64vector (v3 1 2 3))))
       '(1.0 2.0 3.0))
;; The constants are fresh each call, because a vector is mutable and a
;; shared one that someone wrote into would corrupt every later use.
(check constants-are-fresh
       (let ((a (v3-zero))) (vector-set! a 0 9.0) (v3->list (v3-zero)))
       '(0.0 0.0 0.0))
(check units (list (v3->list (v3-unit-x)) (v3->list (v3-unit-y)) (v3->list (v3-unit-z)))
       '((1.0 0.0 0.0) (0.0 1.0 0.0) (0.0 0.0 1.0)))

;; ── the cross product, where handedness lives ────────────────────────
;; x cross y = z, which is the right-handed convention. Getting this
;; backwards mirrors every normal in a scene.
(check cross-is-right-handed (v3->list (v3-cross (v3-unit-x) (v3-unit-y)))
       '(0.0 0.0 1.0))
(check cross-cycles
       (list (v3->list (v3-cross (v3-unit-y) (v3-unit-z)))
             (v3->list (v3-cross (v3-unit-z) (v3-unit-x))))
       '((1.0 0.0 0.0) (0.0 1.0 0.0)))
(check cross-anticommutes
       (v3-approximately=? (v3-cross (v3 1 2 3) (v3 4 5 6))
                           (v3-negate (v3-cross (v3 4 5 6) (v3 1 2 3))))
       #t)
(check cross-is-perpendicular
       (let ((c (v3-cross (v3 1 2 3) (v3 4 5 7))))
         (list (close? (v3-dot c (v3 1 2 3)) 0.0) (close? (v3-dot c (v3 4 5 7)) 0.0)))
       '(#t #t))
(check cross-of-parallel-is-zero
       (v3-approximately=? (v3-cross (v3 1 2 3) (v3 2 4 6)) (v3-zero)) #t)

;; ── reflection, refraction, projection ───────────────────────────────
(check reflect-off-a-floor
       (v3->list (v3-reflect (v3 1 -1 0) (v3-unit-y))) '(1.0 1.0 0.0))
(check reflect-preserves-length
       (close? (v3-length (v3-reflect (v3 1 -2 0.5) (v3-unit-y)))
               (v3-length (v3 1 -2 0.5)))
       #t)
(check refract-straight-through
       (v3-approximately=? (v3-refract (v3 0 -1 0) (v3-unit-y) 1.0) (v3 0 -1 0)) #t)
;; Total internal reflection returns #f rather than a vector of NaNs, so
;; the caller has to decide what to do about it.
(check refract-total-internal
       (v3-refract (v3-normalize (v3 1 -0.05 0)) (v3-unit-y) 2.0) #f)
(check project (v3->list (v3-project (v3 3 4 0) (v3-unit-x))) '(3.0 0.0 0.0))
;; The rejection is perpendicular to what it was rejected from, which is
;; the whole property.
(check reject-is-perpendicular
       (close? (v3-dot (v3-reject (v3 3 4 5) (v3-unit-x)) (v3-unit-x)) 0.0) #t)
(check project-plus-reject
       (v3-approximately=? (v3+ (v3-project (v3 3 4 5) (v3 1 1 0))
                                (v3-reject (v3 3 4 5) (v3 1 1 0)))
                           (v3 3 4 5))
       #t)
(check angle-perpendicular (close? (v3-angle (v3-unit-x) (v3-unit-y)) (/ pi 2.0)) #t)
(check angle-parallel (close? (v3-angle (v3 1 2 3) (v3 2 4 6)) 0.0) #t)
;; Rounding can push the cosine past 1 and `acos` would be a NaN.
(check angle-of-identical-vectors-is-not-nan
       (let ((a (v3-normalize (v3 0.3 0.4 0.5)))) (close? (v3-angle a a) 0.0)) #t)

;; ── the two layers must agree ────────────────────────────────────────
;; The vector layer is defined in terms of the component layer, so these
;; cannot drift; this is the check that says so.
(define a (v3 1.5 -2.25 3.75))
(define b (v3 -0.5 4.0 0.125))
(check layers-agree-dot
       (close? (v3-dot a b) (v3-dot* (v3-x a) (v3-y a) (v3-z a)
                                     (v3-x b) (v3-y b) (v3-z b)))
       #t)
(check layers-agree-cross
       (v3-approximately=? (v3-cross a b)
                           (v3-cross* (v3-x a) (v3-y a) (v3-z a)
                                      (v3-x b) (v3-y b) (v3-z b)))
       #t)
(check layers-agree-length
       (close? (v3-length a) (v3-length* (v3-x a) (v3-y a) (v3-z a))) #t)
(check layers-agree-normalize
       (v3-approximately=? (v3-normalize a)
                           (v3-normalize* (v3-x a) (v3-y a) (v3-z a)))
       #t)
(check layers-agree-reflect
       (v3-approximately=? (v3-reflect a (v3-unit-y))
                           (v3-reflect* (v3-x a) (v3-y a) (v3-z a) 0.0 1.0 0.0))
       #t)
(check layers-agree-lerp
       (v3-approximately=? (v3-lerp a b 0.3)
                           (v3-lerp* (v3-x a) (v3-y a) (v3-z a)
                                     (v3-x b) (v3-y b) (v3-z b) 0.3))
       #t)
;; The component layer's zero case does not raise, unlike the vector
;; layer's -- it is the version a branchless loop wants.
(check component-normalize-of-zero (v3-normalize* 0.0 0.0 0.0) #(0.0 0.0 0.0))

;; ── vec4 ─────────────────────────────────────────────────────────────
(check v4-ctor (v4->list (v4 1 2 3 4)) '(1.0 2.0 3.0 4.0))
(check v4-add (v4->list (v4+ (v4 1 2 3 4) (v4 1 1 1 1))) '(2.0 3.0 4.0 5.0))
(check v4-dot- (v4-dot (v4 1 2 3 4) (v4 1 1 1 1)) 10.0)
(check v4-from-v3 (v4->list (v3->v4 (v3 1 2 3) 1)) '(1.0 2.0 3.0 1.0))
(check v4-to-v3 (v3->list (v4->v3 (v4 1 2 3 9))) '(1.0 2.0 3.0))
(check v4-normalize- (close? (v4-length (v4-normalize (v4 1 1 1 1))) 1.0) #t)

;; ── mat4 ─────────────────────────────────────────────────────────────
;; `mat4-ref` takes a row and a column, so the column-major storage never
;; has to be thought about.
(check ref-by-row-and-column
       (let ((m (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)))
         (list (mat4-ref m 0 0) (mat4-ref m 0 1) (mat4-ref m 1 0) (mat4-ref m 3 3)))
       '(1.0 2.0 5.0 16.0))
;; And the storage really is column-major, which is what makes a bridge
;; to glam a copy rather than a transpose.
(check storage-is-column-major
       (vector->list (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16))
       '(1.0 5.0 9.0 13.0 2.0 6.0 10.0 14.0 3.0 7.0 11.0 15.0 4.0 8.0 12.0 16.0))
(check from-columns-is-storage-order
       (vector->list (mat4-from-columns 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16))
       '(1.0 2.0 3.0 4.0 5.0 6.0 7.0 8.0 9.0 10.0 11.0 12.0 13.0 14.0 15.0 16.0))
(check rows-and-columns-are-transposes
       (let ((m (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)))
         (list (v4->list (mat4-row m 1)) (v4->list (mat4-column m 1))))
       '((5.0 6.0 7.0 8.0) (2.0 6.0 10.0 14.0)))
(check identity-is-identity
       (let ((m (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)))
         (list (mat4-approximately=? (mat4* m (mat4-identity)) m)
               (mat4-approximately=? (mat4* (mat4-identity) m) m)))
       '(#t #t))
(check transpose-is-an-involution
       (let ((m (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)))
         (mat4-approximately=? (mat4-transpose (mat4-transpose m)) m))
       #t)
(check transpose-swaps-indices
       (let ((m (mat4-from-rows 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)))
         (= (mat4-ref (mat4-transpose m) 0 1) (mat4-ref m 1 0)))
       #t)
(check multiply-is-associative
       (let ((x (mat4-translation (v3 1 2 3)))
             (y (mat4-rotation-z (deg 30)))
             (z (mat4-scaling (v3 2 3 4))))
         (mat4-approximately=? (mat4* (mat4* x y) z) (mat4* x (mat4* y z))))
       #t)
(check multiply-is-variadic
       (let ((x (mat4-translation (v3 1 2 3)))
             (y (mat4-rotation-z (deg 30))))
         (mat4-approximately=? (mat4* x y) (mat4* x y (mat4-identity))))
       #t)
(check determinant-of-identity (mat4-determinant (mat4-identity)) 1.0)
(check determinant-of-a-scale
       (close? (mat4-determinant (mat4-scaling (v3 2 3 4))) 24.0) #t)
;; A rotation preserves volume, so its determinant is 1.
(check determinant-of-a-rotation
       (close? (mat4-determinant (mat4-rotation-y (deg 37))) 1.0) #t)
(check inverse-times-matrix-is-identity
       (let ((m (mat4* (mat4-translation (v3 1 2 3))
                       (mat4-rotation-xyz (deg 20) (deg 30) (deg 40))
                       (mat4-scaling (v3 2 0.5 3)))))
         (mat4-approximately=? (mat4* m (mat4-inverse m)) (mat4-identity) 1e-9))
       #t)
;; Reported, not silently infinite.
(check-raises inverse-of-a-singular-matrix
              (mat4-inverse (mat4-scaling (v3 1 0 1))))
;; Partial pivoting is why a badly scaled transform still inverts.
(check inverse-survives-a-tiny-scale
       (let ((m (mat4* (mat4-translation (v3 5 5 5)) (mat4-scaling (v3 1e-8 1 1)))))
         (mat4-approximately=? (mat4* m (mat4-inverse m)) (mat4-identity) 1e-6))
       #t)

;; ── applying a matrix: the point/direction/normal distinction ────────
;; A point carries the translation; a direction does not. Confusing the
;; two is the classic silent error.
(check point-carries-the-translation
       (v3->list (mat4-transform-point (mat4-translation (v3 1 2 3)) (v3 0 0 0)))
       '(1.0 2.0 3.0))
(check direction-ignores-the-translation
       (v3->list (mat4-transform-direction (mat4-translation (v3 1 2 3)) (v3 1 0 0)))
       '(1.0 0.0 0.0))
(check point-under-rotation
       (v3-approximately=? (mat4-transform-point (mat4-rotation-z (deg 90)) (v3 1 0 0))
                           (v3 0 1 0))
       #t)
;; A normal transforms by the inverse transpose. Under a non-uniform
;; scale the matrix itself would tilt it off the surface, which shows up
;; as wrong shading rather than as an error -- so this is the check.
(check normal-stays-perpendicular-under-non-uniform-scale
       (let* ((m (mat4-scaling (v3 2 0.5 1)))
              ;; a surface tangent and its normal, both in the xy plane
              (tangent (v3-normalize (v3 1 1 0)))
              (normal (v3-normalize (v3 -1 1 0)))
              (t* (mat4-transform-direction m tangent))
              (n* (mat4-transform-normal m normal)))
         (close? (v3-dot (v3-normalize t*) n*) 0.0))
       #t)
(check the-naive-normal-would-be-wrong
       (let* ((m (mat4-scaling (v3 2 0.5 1)))
              (tangent (v3-normalize (v3 1 1 0)))
              (normal (v3-normalize (v3 -1 1 0)))
              (t* (v3-normalize (mat4-transform-direction m tangent)))
              (naive (v3-normalize (mat4-transform-direction m normal))))
         ;; not perpendicular any more, which is the bug being avoided
         (> (abs (v3-dot t* naive)) 0.1))
       #t)
(check normal-is-unit-length
       (close? (v3-length (mat4-transform-normal (mat4-scaling (v3 2 0.5 1))
                                                 (v3-normalize (v3 1 1 1))))
               1.0)
       #t)

;; ── ray's Euler order, which cost two wrong renders ─────────────────
;; `mat4-rotation-xyz` is the product Rx·Ry·Rz, so Z is applied to the
;; vector FIRST. These two checks are the whole reason the order is in
;; the name.
(check euler-is-rx-ry-rz
       (let ((rx (deg 20)) (ry (deg 30)) (rz (deg 40)))
         (mat4-approximately=?
          (mat4-rotation-xyz rx ry rz)
          (mat4* (mat4-rotation-x rx) (mat4-rotation-y ry) (mat4-rotation-z rz))))
       #t)
;; And it is NOT the other order, so a test that passed under either
;; convention would be worthless.
(check euler-order-actually-matters
       (let ((rx (deg 20)) (ry (deg 30)) (rz (deg 40)))
         (mat4-approximately=?
          (mat4-rotation-xyz rx ry rz)
          (mat4* (mat4-rotation-z rz) (mat4-rotation-y ry) (mat4-rotation-x rx))))
       #f)
;; The quaternion and the matrix must agree on it too.
(check quaternion-euler-matches-the-matrix
       (let ((rx (deg 20)) (ry (deg 30)) (rz (deg 40)))
         (mat4-approximately=? (quat->mat4 (quat-from-euler-xyz rx ry rz))
                               (mat4-rotation-xyz rx ry rz)
                               1e-9))
       #t)

;; ── the camera matrices ──────────────────────────────────────────────
;; The eye goes to the origin and the view direction to -Z, which is the
;; right-handed convention glam uses.
(check look-at-puts-the-eye-at-the-origin
       (v3-approximately=?
        (mat4-transform-point (mat4-look-at (v3 0 0 5) (v3 0 0 0) (v3-unit-y)) (v3 0 0 5))
        (v3-zero))
       #t)
(check look-at-looks-down-negative-z
       (v3-approximately=?
        (mat4-transform-point (mat4-look-at (v3 0 0 5) (v3 0 0 0) (v3-unit-y)) (v3 0 0 0))
        (v3 0 0 -5))
       #t)
(check look-at-keeps-up-up
       (let ((m (mat4-look-at (v3 0 0 5) (v3 0 0 0) (v3-unit-y))))
         (v3-approximately=? (mat4-transform-direction m (v3-unit-y)) (v3-unit-y)))
       #t)
;; A point on the near plane lands on -1 and the far plane on +1, which
;; is the OpenGL depth convention this uses.
(check perspective-maps-the-depth-range
       (let* ((m (mat4-perspective (deg 60) 1.0 1.0 100.0))
              (near (mat4-transform-point m (v3 0 0 -1.0)))
              (far (mat4-transform-point m (v3 0 0 -100.0))))
         (list (close? (v3-z near) -1.0) (close? (v3-z far) 1.0)))
       '(#t #t))
(check orthographic-maps-the-box
       (let ((m (mat4-orthographic -2.0 2.0 -1.0 1.0 1.0 10.0)))
         (list (v3-approximately=? (mat4-transform-point m (v3 2 1 -1)) (v3 1 1 -1))
               (v3-approximately=? (mat4-transform-point m (v3 -2 -1 -10)) (v3 -1 -1 1))))
       '(#t #t))

;; ── compose and decompose ────────────────────────────────────────────
(check compose-is-translate-rotate-scale
       (v3-approximately=?
        (mat4-transform-point
         (mat4-compose (v3 10 0 0) (quat-from-axis-angle (v3-unit-z) (deg 90)) (v3 2 2 2))
         (v3 1 0 0))
        (v3 10 2 0))
       #t)
(check decompose-round-trips
       (let* ((t (v3 1 2 3))
              (r (quat-from-euler-xyz (deg 20) (deg 30) (deg 40)))
              (s (v3 2 3 4))
              (m (mat4-compose t r s)))
         (call-with-values (lambda () (mat4-decompose m))
           (lambda (t* r* s*)
             (list (v3-approximately=? t t* 1e-9)
                   (quat-approximately=? (quat-normalize r) r* 1e-9)
                   (v3-approximately=? s s* 1e-9)))))
       '(#t #t #t))
;; A mirroring transform cannot be a rotation, so the sign goes into the
;; scale, where compose puts it back.
(check decompose-handles-a-mirror
       (let ((m (mat4-compose (v3-zero) (quat-identity) (v3 -1 1 1))))
         (call-with-values (lambda () (mat4-decompose m))
           (lambda (t* r* s*)
             (mat4-approximately=? (mat4-compose t* r* s*) m 1e-9))))
       #t)
(check-raises decompose-rejects-a-zero-scale
              (mat4-decompose (mat4-scaling (v3 1 0 1))))

;; ── quaternions ──────────────────────────────────────────────────────
(check quat-ctor (quat->list (quat 1 2 3 4)) '(1.0 2.0 3.0 4.0))
;; w is LAST, as glam stores it.
(check w-is-last (quat-w (quat-identity)) 1.0)
(check identity-rotates-nothing
       (v3-approximately=? (quat-rotate (quat-identity) (v3 1 2 3)) (v3 1 2 3)) #t)
(check axis-angle-rotation
       (v3-approximately=?
        (quat-rotate (quat-from-axis-angle (v3-unit-z) (deg 90)) (v3 1 0 0))
        (v3 0 1 0))
       #t)
(check rotation-preserves-length
       (close? (v3-length (quat-rotate (quat-from-euler-xyz (deg 11) (deg 22) (deg 33))
                                       (v3 1 2 3)))
               (v3-length (v3 1 2 3)))
       #t)
;; Composition applies the right one first, like matrix multiplication.
(check composition-order
       (let ((qz (quat-from-axis-angle (v3-unit-z) (deg 90)))
             (qy (quat-from-axis-angle (v3-unit-y) (deg 90))))
         (v3-approximately=? (quat-rotate (quat* qy qz) (v3 1 0 0))
                             (quat-rotate qy (quat-rotate qz (v3 1 0 0)))))
       #t)
(check conjugate-undoes-a-unit-rotation
       (let ((q (quat-from-euler-xyz (deg 20) (deg 30) (deg 40))))
         (v3-approximately=? (quat-rotate (quat-conjugate q) (quat-rotate q (v3 1 2 3)))
                             (v3 1 2 3)))
       #t)
(check inverse-undoes-a-rotation
       (let ((q (quat-from-axis-angle (v3 1 1 1) (deg 73))))
         (quat-approximately=? (quat* q (quat-inverse q)) (quat-identity) 1e-9))
       #t)
(check normalize-is-unit-quat
       (close? (quat-length (quat-normalize (quat 1 2 3 4))) 1.0) #t)
(check-raises normalize-rejects-the-zero-quaternion (quat-normalize (quat 0 0 0 0)))
(check dot-of-identical (close? (quat-dot (quat-identity) (quat-identity)) 1.0) #t)

;; slerp hits both endpoints and the halfway rotation.
(check slerp-endpoints
       (let ((a (quat-identity)) (b (quat-from-axis-angle (v3-unit-y) (deg 90))))
         (list (quat-approximately=? (quat-slerp a b 0.0) a 1e-9)
               (quat-approximately=? (quat-slerp a b 1.0) b 1e-9)))
       '(#t #t))
(check slerp-halfway
       (let ((a (quat-identity)) (b (quat-from-axis-angle (v3-unit-y) (deg 90))))
         (quat-approximately=? (quat-slerp a b 0.5)
                               (quat-from-axis-angle (v3-unit-y) (deg 45))
                               1e-9))
       #t)
;; q and -q are the same rotation, so slerp must take the short way
;; round rather than the 270-degree path.
(check slerp-takes-the-short-way
       (let* ((a (quat-from-axis-angle (v3-unit-y) (deg 10)))
              (nb (let ((b (quat-from-axis-angle (v3-unit-y) (deg 20))))
                    (quat (- (quat-x b)) (- (quat-y b)) (- (quat-z b)) (- (quat-w b)))))
              (mid (quat-slerp a nb 0.5)))
         (quat-approximately=? mid (quat-from-axis-angle (v3-unit-y) (deg 15)) 1e-9))
       #t)
(check slerp-of-nearly-equal-does-not-divide-by-zero
       (let ((a (quat-from-axis-angle (v3-unit-y) (deg 10))))
         (quat-approximately=? (quat-slerp a a 0.5) a 1e-9))
       #t)

;; from-to aims one direction at another, which is what pointing a model
;; at a target needs.
(check from-to-aims
       (let ((q (quat-from-to (v3-unit-x) (v3-unit-y))))
         (v3-approximately=? (quat-rotate q (v3-unit-x)) (v3-unit-y)))
       #t)
(check from-to-identical-is-identity
       (quat-approximately=? (quat-from-to (v3-unit-x) (v3-unit-x)) (quat-identity) 1e-9)
       #t)
;; The antiparallel case has no unique answer, so any perpendicular axis
;; will do -- but it must still land on the target.
(check from-to-antiparallel
       (let ((q (quat-from-to (v3-unit-x) (v3-negate (v3-unit-x)))))
         (v3-approximately=? (quat-rotate q (v3-unit-x)) (v3-negate (v3-unit-x)) 1e-9))
       #t)

(check quat-matrix-round-trip
       (let ((q (quat-normalize (quat-from-euler-xyz (deg 20) (deg 30) (deg 40)))))
         (quat-approximately=? (mat4->quat (quat->mat4 q)) q 1e-9))
       #t)
;; Shepperd's method matters at 180 degrees, where one fixed formula
;; loses all its precision.
(check quat-matrix-round-trip-at-180
       (let ((q (quat-from-axis-angle (v3-unit-y) pi)))
         (quat-approximately=? (mat4->quat (quat->mat4 q)) q 1e-7))
       #t)
(check quat-matrix-round-trip-each-axis
       (map (lambda (axis)
              (let ((q (quat-from-axis-angle axis pi)))
                (quat-approximately=? (mat4->quat (quat->mat4 q)) q 1e-7)))
            (list (v3-unit-x) (v3-unit-y) (v3-unit-z)))
       '(#t #t #t))
(check euler-round-trip
       (let* ((rx (deg 20)) (ry (deg 30)) (rz (deg 40))
              (q (quat-from-euler-xyz rx ry rz)))
         (call-with-values (lambda () (quat-to-euler-xyz q))
           (lambda (x y z)
             (quat-approximately=? (quat-from-euler-xyz x y z) q 1e-9))))
       #t)
;; At a pitch of ninety degrees the other two angles are not separable,
;; so the round trip must still give the same ROTATION even though the
;; angles differ.
(check euler-round-trip-at-gimbal-lock
       (let ((q (quat-from-euler-xyz (deg 10) (deg 90) (deg 25))))
         (call-with-values (lambda () (quat-to-euler-xyz q))
           (lambda (x y z)
             (quat-approximately=? (quat-from-euler-xyz x y z) q 1e-6))))
       #t)
(check quat-to-from-list (quat->list (list->quat '(1 2 3 4))) '(1.0 2.0 3.0 4.0))

(test-failures)
