;;; (scm linalg)'s native procedures against the Scheme they replaced (M7).
;;;
;;; src/prim/linalg.rs ports each procedure operation for operation, so
;;; the claim is not "close" but BIT-IDENTICAL: `equal?` on vectors of
;;; flonums compares with `eqv?`, which tells -0.0 from 0.0. The Scheme
;;; originals below are verbatim from linalg.sld before M7, renamed ref-…;
;;; the input is a fixed pseudo-random stream, with integers, zeros and
;;; negative zeros among the reals.
(import (scm test) (scm linalg) (scheme base) (scheme inexact) (srfi 4))

;; What the originals used from inside the library.
(define (%ix r c) (+ (* c 4) r))
(define (%set! m r c x) (vector-set! m (%ix r c) x))

;; ── the originals ────────────────────────────────────────────────────
(define (ref-mat4* a . rest)
      (let loop ((acc a) (ms rest))
        (if (null? ms) acc (loop (ref-mul acc (car ms)) (cdr ms)))))

(define (ref-mul a b)
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

(define (ref-mat4-transpose m)
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

(define (ref-mat4-inverse m)
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
              (let ((pivot (ref-best-pivot a col)))
                (when (< (abs (vector-ref a (+ (* pivot 8) col))) 1e-14)
                  (error "ref-mat4-inverse: the matrix is singular"))
                (ref-swap-rows! a col pivot)
                (ref-scale-row! a col (/ 1.0 (vector-ref a (+ (* col 8) col))))
                (let others ((r 0))
                  (when (< r 4)
                    (unless (= r col)
                      (ref-axpy! a r col (- (vector-ref a (+ (* r 8) col)))))
                    (others (+ r 1))))
                (elim (+ col 1)))))))

(define (ref-best-pivot a col)
      (let loop ((r col) (best col) (bv (abs (vector-ref a (+ (* col 8) col)))))
        (if (= r 4)
            best
            (let ((v (abs (vector-ref a (+ (* r 8) col)))))
              (if (> v bv) (loop (+ r 1) r v) (loop (+ r 1) best bv))))))

(define (ref-swap-rows! a r1 r2)
      (unless (= r1 r2)
        (let loop ((c 0))
          (when (< c 8)
            (let ((t (vector-ref a (+ (* r1 8) c))))
              (vector-set! a (+ (* r1 8) c) (vector-ref a (+ (* r2 8) c)))
              (vector-set! a (+ (* r2 8) c) t))
            (loop (+ c 1))))))

(define (ref-scale-row! a r k)
      (let loop ((c 0))
        (when (< c 8)
          (vector-set! a (+ (* r 8) c) (* k (vector-ref a (+ (* r 8) c))))
          (loop (+ c 1)))))

(define (ref-axpy! a r src k)
      (let loop ((c 0))
        (when (< c 8)
          (vector-set! a (+ (* r 8) c)
                       (+ (vector-ref a (+ (* r 8) c))
                          (* k (vector-ref a (+ (* src 8) c)))))
          (loop (+ c 1)))))

(define (ref-mat4-translation t)
      (mat4-from-rows 1.0 0.0 0.0 (v3-x t)
                      0.0 1.0 0.0 (v3-y t)
                      0.0 0.0 1.0 (v3-z t)
                      0.0 0.0 0.0 1.0))

(define (ref-mat4-scaling s)
      (mat4-from-rows (v3-x s) 0.0 0.0 0.0
                      0.0 (v3-y s) 0.0 0.0
                      0.0 0.0 (v3-z s) 0.0
                      0.0 0.0 0.0 1.0))

(define (ref-mat4-rotation-x a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows 1.0 0.0 0.0 0.0
                        0.0 c (- s) 0.0
                        0.0 s c 0.0
                        0.0 0.0 0.0 1.0)))

(define (ref-mat4-rotation-y a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows c 0.0 s 0.0
                        0.0 1.0 0.0 0.0
                        (- s) 0.0 c 0.0
                        0.0 0.0 0.0 1.0)))

(define (ref-mat4-rotation-z a)
      (let ((c (cos a)) (s (sin a)))
        (mat4-from-rows c (- s) 0.0 0.0
                        s c 0.0 0.0
                        0.0 0.0 1.0 0.0
                        0.0 0.0 0.0 1.0)))

(define (ref-mat4-transform-point m p)
      (let* ((x (v3-x p)) (y (v3-y p)) (z (v3-z p))
             (rx (+ (* (mat4-ref m 0 0) x) (* (mat4-ref m 0 1) y) (* (mat4-ref m 0 2) z) (mat4-ref m 0 3)))
             (ry (+ (* (mat4-ref m 1 0) x) (* (mat4-ref m 1 1) y) (* (mat4-ref m 1 2) z) (mat4-ref m 1 3)))
             (rz (+ (* (mat4-ref m 2 0) x) (* (mat4-ref m 2 1) y) (* (mat4-ref m 2 2) z) (mat4-ref m 2 3)))
             (rw (+ (* (mat4-ref m 3 0) x) (* (mat4-ref m 3 1) y) (* (mat4-ref m 3 2) z) (mat4-ref m 3 3))))
        (if (or (= rw 1.0) (= rw 0.0))
            (v3 rx ry rz)
            (v3 (/ rx rw) (/ ry rw) (/ rz rw)))))

(define (ref-mat4-transform-direction m d)
      (let ((x (v3-x d)) (y (v3-y d)) (z (v3-z d)))
        (v3 (+ (* (mat4-ref m 0 0) x) (* (mat4-ref m 0 1) y) (* (mat4-ref m 0 2) z))
            (+ (* (mat4-ref m 1 0) x) (* (mat4-ref m 1 1) y) (* (mat4-ref m 1 2) z))
            (+ (* (mat4-ref m 2 0) x) (* (mat4-ref m 2 1) y) (* (mat4-ref m 2 2) z)))))

(define (ref-quat* a . rest)
      (let loop ((acc a) (qs rest))
        (if (null? qs) acc (loop (ref-qmul acc (car qs)) (cdr qs)))))

(define (ref-qmul a b)
      (let ((ax (quat-x a)) (ay (quat-y a)) (az (quat-z a)) (aw (quat-w a))
            (bx (quat-x b)) (by (quat-y b)) (bz (quat-z b)) (bw (quat-w b)))
        (quat (+ (* aw bx) (* ax bw) (* ay bz) (- (* az by)))
              (+ (* aw by) (- (* ax bz)) (* ay bw) (* az bx))
              (+ (* aw bz) (* ax by) (- (* ay bx)) (* az bw))
              (- (* aw bw) (* ax bx) (* ay by) (* az bz)))))

(define (ref-quat->mat4 q)
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

;; ── S39: the vec3 layer's originals, verbatim from linalg.sld ──────
(define (%f x) (exact->inexact x))
(define (ref-v3 x y z) (vector (%f x) (%f y) (%f z)))
(define (ref-v3+ a . rest)
      (let loop ((acc a) (vs rest))
        (if (null? vs)
            acc
            (loop (v3-add* (v3-x acc) (v3-y acc) (v3-z acc)
                           (v3-x (car vs)) (v3-y (car vs)) (v3-z (car vs)))
                  (cdr vs)))))
(define (ref-v3- a . rest)
      (if (null? rest)
          (ref-v3-negate a)
          (let loop ((acc a) (vs rest))
            (if (null? vs)
                acc
                (loop (v3-sub* (v3-x acc) (v3-y acc) (v3-z acc)
                               (v3-x (car vs)) (v3-y (car vs)) (v3-z (car vs)))
                      (cdr vs))))))
(define (ref-v3* a . rest)
      (let loop ((acc a) (vs rest))
        (if (null? vs)
            acc
            (let ((b (car vs)))
              (loop (vector (* (v3-x acc) (v3-x b))
                            (* (v3-y acc) (v3-y b))
                            (* (v3-z acc) (v3-z b)))
                    (cdr vs))))))
(define (ref-v3/ a b)
      (vector (/ (v3-x a) (v3-x b)) (/ (v3-y a) (v3-y b)) (/ (v3-z a) (v3-z b))))
(define (ref-v3-negate a) (vector (- (v3-x a)) (- (v3-y a)) (- (v3-z a))))
(define (ref-v3-scale a k) (v3-scale* (v3-x a) (v3-y a) (v3-z a) (%f k)))
(define (ref-v3-dot a b)
      (v3-dot* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))
(define (ref-v3-cross a b)
      (v3-cross* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))
(define (ref-v3-length a) (v3-length* (v3-x a) (v3-y a) (v3-z a)))
(define (ref-v3-length-squared a)
      (v3-length-squared* (v3-x a) (v3-y a) (v3-z a)))
(define (ref-v3-distance a b)
      (v3-distance* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))
(define (ref-v3-distance-squared a b)
      (v3-distance-squared* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b)))
(define (ref-v3-normalize a)
      (let ((n (ref-v3-length a)))
        (if (= n 0.0)
            (error "v3-normalize: the zero vector has no direction")
            (ref-v3-scale a (/ 1.0 n)))))
(define (ref-v3-lerp a b t)
      (v3-lerp* (v3-x a) (v3-y a) (v3-z a) (v3-x b) (v3-y b) (v3-z b) (%f t)))
(define (ref-v3-min a b)
      (vector (min (v3-x a) (v3-x b)) (min (v3-y a) (v3-y b)) (min (v3-z a) (v3-z b))))
(define (ref-v3-max a b)
      (vector (max (v3-x a) (v3-x b)) (max (v3-y a) (v3-y b)) (max (v3-z a) (v3-z b))))

;; ── the input ────────────────────────────────────────────────────────
(define seed 20261121)
(define (next!)
  (set! seed (modulo (+ (* seed 1103515245) 12345) 2147483648))
  seed)
;; A real in [-10, 10), now and then an integer, a zero or a negative zero.
(define (any-real)
  (let ((k (modulo (next!) 20)))
    (cond ((= k 0) 0.0)
          ((= k 1) -0.0)
          ((= k 2) (- (modulo (next!) 7) 3))
          (else (- (* 20.0 (/ (next!) 2147483648.0)) 10.0)))))
(define (any-mat4) (let ((m (make-vector 16))) (do ((i 0 (+ i 1))) ((= i 16) m) (vector-set! m i (any-real)))))
(define (any-v3) (vector (any-real) (any-real) (any-real)))
(define (any-quat)
  (let ((q (vector (any-real) (any-real) (any-real) (any-real))))
    (if (equal? q (vector 0.0 0.0 0.0 0.0)) (vector 0.0 0.0 0.0 1.0) q)))

;; How many of N trials disagree; a raise on either side counts as the
;; answer 'raised, so both must refuse the same input.
(define (disagreements n make native reference)
  (let loop ((i 0) (bad 0))
    (if (= i n)
        bad
        (let* ((args (make))
               (try (lambda (f) (guard (e (#t 'raised)) (apply f args)))))
          (loop (+ i 1) (if (same? (try native) (try reference)) bad (+ bad 1)))))))

;; `equal?`, but a NaN is the same as a NaN, which `eqv?` says it is not:
;; v3/ divides 0 by 0. -0.0 and 0.0, and exact and inexact, still differ.
(define (nan? x) (and (real? x) (not (= x x))))
(define (same? a b)
  (cond ((and (vector? a) (vector? b))
         (and (= (vector-length a) (vector-length b))
              (let loop ((i 0))
                (or (= i (vector-length a))
                    (and (same? (vector-ref a i) (vector-ref b i)) (loop (+ i 1)))))))
        ((and (nan? a) (nan? b)) #t)
        (else (equal? a b))))

(define n 300)
(check mat4*-two (disagreements n (lambda () (list (any-mat4) (any-mat4))) mat4* ref-mat4*) 0)
(check mat4*-three (disagreements n (lambda () (list (any-mat4) (any-mat4) (any-mat4))) mat4* ref-mat4*) 0)
(check mat4*-one-is-itself (let ((m (any-mat4))) (eq? (mat4* m) m)) #t)
(check mat4-transpose (disagreements n (lambda () (list (any-mat4))) mat4-transpose ref-mat4-transpose) 0)
(check mat4-inverse (disagreements n (lambda () (list (any-mat4))) mat4-inverse ref-mat4-inverse) 0)
(check mat4-inverse-of-transforms
       (disagreements n (lambda () (list (mat4* (mat4-translation (any-v3)) (mat4-rotation-y (any-real)) (mat4-scaling (any-v3)))))
                      mat4-inverse ref-mat4-inverse)
       0)
(check mat4-inverse-singular (guard (e (#t 'raised)) (mat4-inverse (make-vector 16 0.0))) 'raised)
(check mat4-translation (disagreements n (lambda () (list (any-v3))) mat4-translation ref-mat4-translation) 0)
(check mat4-scaling (disagreements n (lambda () (list (any-v3))) mat4-scaling ref-mat4-scaling) 0)
(check mat4-rotation-x (disagreements n (lambda () (list (any-real))) mat4-rotation-x ref-mat4-rotation-x) 0)
(check mat4-rotation-y (disagreements n (lambda () (list (any-real))) mat4-rotation-y ref-mat4-rotation-y) 0)
(check mat4-rotation-z (disagreements n (lambda () (list (any-real))) mat4-rotation-z ref-mat4-rotation-z) 0)
(check mat4-transform-point (disagreements n (lambda () (list (any-mat4) (any-v3))) mat4-transform-point ref-mat4-transform-point) 0)
(check mat4-transform-direction (disagreements n (lambda () (list (any-mat4) (any-v3))) mat4-transform-direction ref-mat4-transform-direction) 0)
(check quat*-two (disagreements n (lambda () (list (any-quat) (any-quat))) quat* ref-quat*) 0)
(check quat*-three (disagreements n (lambda () (list (any-quat) (any-quat) (any-quat))) quat* ref-quat*) 0)
(check quat->mat4 (disagreements n (lambda () (list (any-quat))) quat->mat4 ref-quat->mat4) 0)
(check quat->mat4-zero (guard (e (#t 'raised)) (quat->mat4 (vector 0.0 0.0 0.0 0.0))) 'raised)

;; A wrong shape is refused, not misread.
(check-raises mat4*-short (mat4* (make-vector 15 0.0) (mat4-identity)))
(check-raises mat4-translation-not-v3 (mat4-translation (vector 1.0 2.0)))
(check-raises mat4*-not-numbers (mat4* (make-vector 16 'x) (mat4-identity)))

;; ── S39: the vec3 layer ─────────────────────────────────────────────
;; Exact integers, among them zeros and large ones, to check that a result
;; is as exact as the Scheme's -- (v3+ [0 1 0] [1 0 0]) is #(1 1 0).
(define (any-exact) (let ((k (modulo (next!) 10)))
                      (cond ((= k 0) 0) ((= k 1) (expt 2 70)) (else (- (modulo (next!) 21) 10)))))
(define (any-exact-v3) (vector (any-exact) (any-exact) (any-exact)))
(define (any-mixed-v3) (vector (any-exact) (any-real) (any-exact)))
(define (each-shape name native reference make-args)
  ;; disagreements over reals, exact integers and a mix
  (list name (disagreements n (lambda () (make-args any-v3)) native reference)
        (disagreements n (lambda () (make-args any-exact-v3)) native reference)
        (disagreements n (lambda () (make-args any-mixed-v3)) native reference)))
(define (one-v3 g) (list (g)))
(define (two-v3 g) (list (g) (g)))
(define (three-v3 g) (list (g) (g) (g)))
(define (v3-and-real g) (list (g) (any-real)))
(define (v3-and-exact g) (list (g) (any-exact)))
(define (two-v3-and-real g) (list (g) (g) (any-real)))
(define-syntax check-v3
  (syntax-rules ()
    ((_ name native reference make-args)
     (check name (cdr (each-shape 'name native reference make-args)) '(0 0 0)))))
(check v3 (disagreements n (lambda () (list (any-real) (any-exact) (any-real))) v3 ref-v3) 0)
(check-v3 v3+-two v3+ ref-v3+ two-v3)
(check-v3 v3+-three v3+ ref-v3+ three-v3)
(check-v3 v3--one v3- ref-v3- one-v3)
(check-v3 v3--two v3- ref-v3- two-v3)
(check-v3 v3--three v3- ref-v3- three-v3)
(check-v3 v3*-two v3* ref-v3* two-v3)
(check-v3 v3*-three v3* ref-v3* three-v3)
(check-v3 v3/ v3/ ref-v3/ two-v3)
(check-v3 v3-negate v3-negate ref-v3-negate one-v3)
(check-v3 v3-scale v3-scale ref-v3-scale v3-and-real)
(check-v3 v3-scale-exact v3-scale ref-v3-scale v3-and-exact)
(check-v3 v3-dot v3-dot ref-v3-dot two-v3)
(check-v3 v3-cross v3-cross ref-v3-cross two-v3)
(check-v3 v3-length v3-length ref-v3-length one-v3)
(check-v3 v3-length-squared v3-length-squared ref-v3-length-squared one-v3)
(check-v3 v3-distance v3-distance ref-v3-distance two-v3)
(check-v3 v3-distance-squared v3-distance-squared ref-v3-distance-squared two-v3)
(check-v3 v3-normalize v3-normalize ref-v3-normalize one-v3)
(check-v3 v3-lerp v3-lerp ref-v3-lerp two-v3-and-real)
(check-v3 v3-min v3-min ref-v3-min two-v3)
(check-v3 v3-max v3-max ref-v3-max two-v3)
(check v3+-one-is-itself (let ((a (any-v3))) (eq? (v3+ a) a)) #t)
(check v3+-exact (v3+ [0 1 0] [1 0 0]) #(1 1 0))
(check v3-dot-exact (v3-dot #(1 2 3) #(4 5 6)) 32)
(check v3-length-exact (v3-length #(3 4 0)) 5)
(check v3-normalize-zero-message
       (guard (e ((error-object? e) (error-object-message e))) (v3-normalize #(0 0 0)))
       "v3-normalize: the zero vector has no direction")
(check v3-normalize-or-zero (v3-normalize-or #(0.0 0.0 0.0) 'none) 'none)
(check v3-clamp (v3-clamp #(-2.0 0.5 9) #(0 0 0) #(1 1 1)) #(0.0 0.5 1))
(check-raises v3+-short (v3+ (vector 1.0 2.0) (vector 1.0 2.0 3.0)))
(check-raises v3-dot-not-numbers (v3-dot (vector 'x 'y 'z) (vector 1.0 2.0 3.0)))
(check-raises v3/-by-exact-zero (v3/ #(1 2 3) #(0 1 1)))

;; ── mat4-store!: new, with no original ───────────────────────────────
(define m (mat4* (mat4-translation (vector 1.0 2.0 3.0)) (mat4-rotation-z 0.5)))
(define f64 (make-f64vector 40 -1.0))
(mat4-store! f64 4 m)
(check store-f64-exact
       (let loop ((i 0)) (cond ((= i 16) #t) ((eqv? (f64vector-ref f64 (+ 4 i)) (vector-ref m i)) (loop (+ i 1))) (else #f)))
       #t)
(check store-f64-untouched-around (list (f64vector-ref f64 3) (f64vector-ref f64 20)) '(-1.0 -1.0))
(define f32 (make-f32vector 20 0.0))
(mat4-store! f32 0 m)
(check store-f32-close
       (let loop ((i 0)) (cond ((= i 16) #t) ((< (abs (- (f32vector-ref f32 i) (vector-ref m i))) 1e-6) (loop (+ i 1))) (else #f)))
       #t)
(check store-f32-translation-column (list (f32vector-ref f32 12) (f32vector-ref f32 13) (f32vector-ref f32 14)) '(1.0 2.0 3.0))
(check-raises store-does-not-fit (mat4-store! f32 5 m))
(check-raises store-negative-offset (mat4-store! f32 -1 m))
(check-raises store-not-a-numvector (mat4-store! (make-vector 16 0.0) 0 m))

(test-failures)
