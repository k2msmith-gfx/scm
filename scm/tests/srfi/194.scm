;;; (srfi 194) random data generators.
;;;
;;; Ranges are the easy half. The half that matters is that the
;;; DISTRIBUTIONS are right, because a biased sampler in a renderer is a
;;; faint wrong answer rather than a crash. Every check below runs under a
;;; fixed seed, so the numbers are deterministic and the tolerances are
;;; there to describe the distribution rather than to absorb flakiness.
(import (scm test) (srfi 194) (srfi 27) (srfi 132) (scheme base) (scheme inexact))

(define (%unique xs)
  (let loop ((xs xs) (acc '()))
    (cond ((null? xs) (reverse acc))
          ((member (car xs) acc) (loop (cdr xs) acc))
          (else (loop (cdr xs) (cons (car xs) acc))))))

(define (take-n g n)
  (let loop ((i 0) (acc '()))
    (if (= i n) (reverse acc) (loop (+ i 1) (cons (g) acc)))))

(define (fold-left f init xs)
  (if (null? xs) init (fold-left f (f init (car xs)) (cdr xs))))
(define (sum xs) (fold-left + 0 xs))
(define (mean xs) (/ (exact->inexact (sum xs)) (length xs)))
(define (variance xs)
  (let ((m (mean xs)))
    (/ (sum (map (lambda (x) (let ((d (- x m))) (* d d))) xs))
       (length xs))))
(define (near? a b tol) (< (abs (- a b)) tol))
(define (all? p xs) (if (null? xs) #t (and (p (car xs)) (all? p (cdr xs)))))
(define (filter p xs)
  (cond ((null? xs) '())
        ((p (car xs)) (cons (car xs) (filter p (cdr xs))))
        (else (filter p (cdr xs)))))
(define (count-if p xs) (length (filter p xs)))
(define (vector-sum-of-squares v)
  (let loop ((i 0) (s 0.0))
    (if (= i (vector-length v)) s (loop (+ i 1) (+ s (square (vector-ref v i)))))))

;; Everything under one fixed seed, so this file is reproducible.
(define src (make-random-source 20261015))
(define-syntax seeded
  (syntax-rules ()
    ((_ body ...) (parameterize ((current-random-source src)) body ...))))

;; ── uniform ranges ───────────────────────────────────────────────────
(check integer-range
       (seeded (all? (lambda (n) (and (exact? n) (>= n 5) (< n 10)))
                     (take-n (make-random-integer-generator 5 10) 300)))
       #t)
(check integer-covers-the-range
       (seeded (let ((xs (take-n (make-random-integer-generator 0 4) 400)))
                 (list-sort < (%unique xs))))
       '(0 1 2 3))
(check integer-rejects-an-empty-range
       (seeded (guard (e (#t 'raised)) (make-random-integer-generator 5 5) 'no))
       'raised)
;; The SRFI's real interval is CLOSED, unlike (srfi 27)'s open one.
(check real-range
       (seeded (all? (lambda (x) (and (inexact? x) (>= x 2.0) (<= x 3.0)))
                     (take-n (make-random-real-generator 2.0 3.0) 300)))
       #t)
(check real-mean
       (seeded (near? (mean (take-n (make-random-real-generator 0.0 1.0) 4000)) 0.5 0.03))
       #t)
(check u1 (seeded (list-sort < (%unique (take-n (make-random-u1-generator) 200)))) '(0 1))
(check u8-range
       (seeded (all? (lambda (n) (and (>= n 0) (< n 256)))
                     (take-n (make-random-u8-generator) 300)))
       #t)
(check s8-range
       (seeded (all? (lambda (n) (and (>= n -128) (< n 128)))
                     (take-n (make-random-s8-generator) 300)))
       #t)
(check u16-s16-u32-s32-ranges
       (seeded (list (all? (lambda (n) (and (>= n 0) (< n 65536)))
                           (take-n (make-random-u16-generator) 100))
                     (all? (lambda (n) (and (>= n -32768) (< n 32768)))
                           (take-n (make-random-s16-generator) 100))
                     (all? (lambda (n) (and (>= n 0) (< n 4294967296)))
                           (take-n (make-random-u32-generator) 100))
                     (all? (lambda (n) (and (>= n -2147483648) (< n 2147483648)))
                           (take-n (make-random-s32-generator) 100))))
       '(#t #t #t #t))
;; s64 spans the whole signed word, so both signs must turn up.
(check s64-spans-both-signs
       (seeded (let ((xs (take-n (make-random-s64-generator) 200)))
                 (list (> (count-if negative? xs) 0) (> (count-if positive? xs) 0)
                       (all? exact? xs))))
       '(#t #t #t))
(check u64-is-non-negative
       (seeded (all? (lambda (n) (>= n 0)) (take-n (make-random-u64-generator) 200)))
       #t)

(check clamp (list (clamp-real-number 0.0 1.0 -3.0)
                   (clamp-real-number 0.0 1.0 0.5)
                   (clamp-real-number 0.0 1.0 7.0))
       '(0.0 0.5 1.0))

(check boolean-is-even
       (seeded (let ((xs (take-n (make-random-boolean-generator) 2000)))
                 (near? (/ (exact->inexact (count-if (lambda (b) b) xs)) 2000) 0.5 0.04)))
       #t)

(check char-from-the-set
       (seeded (all? (lambda (c) (memv c (string->list "abc")))
                     (take-n (make-random-char-generator "abc") 200)))
       #t)
(check char-rejects-an-empty-string
       (seeded (guard (e (#t 'raised)) (make-random-char-generator "") 'no))
       'raised)
(check string-length-and-alphabet
       (seeded (all? (lambda (s) (and (< (string-length s) 5)
                                      (all? (lambda (c) (memv c (string->list "xy")))
                                            (string->list s))))
                     (take-n (make-random-string-generator 5 "xy") 200)))
       #t)

;; ── nonuniform distributions ─────────────────────────────────────────
(check bernoulli-values
       (seeded (list-sort < (%unique (take-n (make-bernoulli-generator 0.5) 200)))) '(0 1))
(check bernoulli-mean
       (seeded (near? (mean (take-n (make-bernoulli-generator 0.3) 4000)) 0.3 0.03)) #t)
(check bernoulli-rejects-a-bad-p
       (seeded (guard (e (#t 'raised)) (make-bernoulli-generator 1.5) 'no)) 'raised)

(check binomial-mean
       (seeded (near? (mean (take-n (make-binomial-generator 20 0.25) 2000)) 5.0 0.3)) #t)
(check binomial-range
       (seeded (all? (lambda (k) (and (>= k 0) (<= k 20)))
                     (take-n (make-binomial-generator 20 0.25) 500)))
       #t)
;; n = 0 is degenerate but legal.
(check binomial-zero
       (seeded (%unique (take-n (make-binomial-generator 0 0.5) 20))) '(0))

;; Proportions should follow the weights, not merely stay in range.
(check categorical-proportions
       (seeded (let* ((xs (take-n (make-categorical-generator (vector 1 3)) 4000))
                      (ones (/ (exact->inexact (count-if (lambda (k) (= k 1)) xs)) 4000)))
                 (near? ones 0.75 0.03)))
       #t)
(check categorical-range
       (seeded (list-sort < (%unique (take-n (make-categorical-generator (vector 2 1 1)) 500))))
       '(0 1 2))
;; A zero weight must never be chosen.
(check categorical-skips-a-zero-weight
       (seeded (%unique (take-n (make-categorical-generator (vector 0 1 0)) 300))) '(1))
(check categorical-rejects-all-zero
       (seeded (guard (e (#t 'raised)) (make-categorical-generator (vector 0 0)) 'no))
       'raised)

(check normal-mean-and-deviation
       (seeded (let ((xs (take-n (make-normal-generator 5.0 2.0) 6000)))
                 (list (near? (mean xs) 5.0 0.1) (near? (sqrt (variance xs)) 2.0 0.1))))
       '(#t #t))
(check normal-defaults
       (seeded (let ((xs (take-n (make-normal-generator) 6000)))
                 (list (near? (mean xs) 0.0 0.06) (near? (sqrt (variance xs)) 1.0 0.06))))
       '(#t #t))
;; About 68% inside one deviation is the shape, not just the moments.
(check normal-shape
       (seeded (let* ((xs (take-n (make-normal-generator 0.0 1.0) 6000))
                      (inside (/ (exact->inexact (count-if (lambda (x) (< (abs x) 1.0)) xs))
                                 6000)))
                 (near? inside 0.6827 0.02)))
       #t)

;; An exponential has mean and deviation both equal to its mean.
(check exponential
       (seeded (let ((xs (take-n (make-exponential-generator 3.0) 6000)))
                 (list (near? (mean xs) 3.0 0.2) (near? (sqrt (variance xs)) 3.0 0.3)
                       (all? (lambda (x) (> x 0.0)) xs))))
       '(#t #t #t))

;; The support starts at 1 and the mean is 1/p.
(check geometric
       (seeded (let ((xs (take-n (make-geometric-generator 0.25) 6000)))
                 (list (near? (mean xs) 4.0 0.2) (all? (lambda (k) (>= k 1)) xs)
                       (all? exact? xs))))
       '(#t #t #t))
(check geometric-certain (seeded (%unique (take-n (make-geometric-generator 1.0) 20))) '(1))

;; Poisson's variance equals its mean, which is the check that catches a
;; generator that is merely in the right neighbourhood.
(check poisson
       (seeded (let ((xs (take-n (make-poisson-generator 4.0) 6000)))
                 (list (near? (mean xs) 4.0 0.15) (near? (variance xs) 4.0 0.4)
                       (all? (lambda (k) (and (exact? k) (>= k 0))) xs))))
       '(#t #t #t))
;; A large mean must not loop forever: the product form needs exp(-L),
;; which underflows past about 745, so this is in log space.
(check poisson-large-mean
       (seeded (near? (mean (take-n (make-poisson-generator 900.0) 200)) 900.0 15.0)) #t)

(check zipf-range
       (seeded (all? (lambda (k) (and (>= k 1) (<= k 10)))
                     (take-n (make-zipf-generator 10) 500)))
       #t)
;; The whole point of Zipf: 1 is the most common and the tail falls away.
(check zipf-is-decreasing
       (seeded (let* ((xs (take-n (make-zipf-generator 10) 6000))
                      (n1 (count-if (lambda (k) (= k 1)) xs))
                      (n2 (count-if (lambda (k) (= k 2)) xs))
                      (n10 (count-if (lambda (k) (= k 10)) xs)))
                 (list (> n1 n2) (> n2 n10))))
       '(#t #t))
;; s = 0 makes every weight equal, so it degenerates to uniform.
(check zipf-with-s-zero-is-uniform
       (seeded (let ((xs (take-n (make-zipf-generator 4 0.0) 4000)))
                 (near? (/ (exact->inexact (count-if (lambda (k) (= k 1)) xs)) 4000)
                        0.25 0.03)))
       #t)

;; ── the geometric generators ─────────────────────────────────────────
;; (make-sphere-generator n) lives in n+1 dimensions.
(check sphere-dimension
       (seeded (vector-length ((make-sphere-generator 2)))) 3)
(check sphere-is-on-the-unit-sphere
       (seeded (all? (lambda (v) (near? (vector-sum-of-squares v) 1.0 1e-9))
                     (take-n (make-sphere-generator 2) 300)))
       #t)
(check sphere-1-is-the-circle
       (seeded (let ((v ((make-sphere-generator 1))))
                 (list (vector-length v) (near? (vector-sum-of-squares v) 1.0 1e-9))))
       '(2 #t))
;; The real correctness check. For a sphere in 3-space the z-component is
;; UNIFORM on [-1,1] (Archimedes), so ten equal bands must each hold about
;; a tenth of the points. Normalising a vector of uniforms instead of
;; gaussians passes every length check above and fails this one, because it
;; concentrates toward the cube's corners.
(check sphere-is-actually-uniform
       (seeded (let ((bands (make-vector 10 0)))
                 (for-each (lambda (v)
                             (let* ((z (vector-ref v 2))
                                    (i (min 9 (exact (floor (* 5.0 (+ z 1.0)))))))
                               (vector-set! bands i (+ 1 (vector-ref bands i)))))
                           (take-n (make-sphere-generator 2) 8000))
                 (let loop ((i 0) (ok #t))
                   (if (= i 10)
                       ok
                       (loop (+ i 1)
                             (and ok (near? (exact->inexact (vector-ref bands i))
                                            800.0 170.0)))))))
       #t)
(check sphere-components-are-centred
       (seeded (let ((vs (take-n (make-sphere-generator 2) 4000)))
                 (all? (lambda (i) (near? (mean (map (lambda (v) (vector-ref v i)) vs))
                                          0.0 0.05))
                       '(0 1 2))))
       #t)

(check ball-is-inside
       (seeded (all? (lambda (v) (<= (vector-sum-of-squares v) 1.0))
                     (take-n (make-ball-generator 3) 500)))
       #t)
(check ball-dimension (seeded (vector-length ((make-ball-generator 4)))) 4)
;; Uniform BY VOLUME: in three dimensions an eighth of the points fall
;; inside half the radius, not half of them. Scaling the radius linearly
;; would pile them at the centre and this is what catches it.
(check ball-is-uniform-by-volume
       (seeded (let* ((vs (take-n (make-ball-generator 3) 6000))
                      (inner (count-if (lambda (v) (< (sqrt (vector-sum-of-squares v)) 0.5))
                                       vs)))
                 (near? (/ (exact->inexact inner) 6000) 0.125 0.02)))
       #t)

(check ellipsoid-is-on-the-surface
       (seeded (let ((axes (vector 2.0 3.0 4.0)))
                 (all? (lambda (v)
                         (near? (let loop ((i 0) (s 0.0))
                                  (if (= i 3)
                                      s
                                      (loop (+ i 1)
                                            (+ s (square (/ (vector-ref v i)
                                                            (vector-ref axes i)))))))
                                1.0 1e-9))
                       (take-n (make-ellipsoid-generator (vector 2.0 3.0 4.0)) 300))))
       #t)
;; A ball may be given axes instead of a dimension.
(check ball-from-axes
       (seeded (let ((v ((make-ball-generator (vector 2.0 4.0)))))
                 (list (vector-length v)
                       (<= (+ (square (/ (vector-ref v 0) 2.0))
                              (square (/ (vector-ref v 1) 4.0)))
                           1.0))))
       '(2 #t))

;; ── sources and combining ────────────────────────────────────────────
;; The source is read at each draw, so a seeded region is reproducible.
(check with-random-source-is-reproducible
       (let ((g (make-random-integer-generator 0 1000)))
         (equal? (with-random-source (make-random-source 5) (lambda () (take-n g 10)))
                 (with-random-source (make-random-source 5) (lambda () (take-n g 10)))))
       #t)
(check source-generator-gives-distinct-sources
       (let* ((gen (make-random-source-generator 1))
              (a (gen)) (b (gen)))
         (list (random-source? a) (random-source? b)
               (equal? (with-random-source a (lambda () (take-n (make-random-u8-generator) 8)))
                       (with-random-source b (lambda () (take-n (make-random-u8-generator) 8))))))
       '(#t #t #f))
;; Same index, same substream: a tile keeps its own stream and stays
;; replayable across runs.
(check source-generator-is-reproducible
       (let ((one (make-random-source-generator 7))
             (two (make-random-source-generator 7)))
         (equal? (with-random-source (one) (lambda () (take-n (make-random-u8-generator) 8)))
                 (with-random-source (two) (lambda () (take-n (make-random-u8-generator) 8)))))
       #t)

(check gsampling-draws-from-both
       (seeded (let* ((g (gsampling (lambda () 'a) (lambda () 'b)))
                      (xs (take-n g 400)))
                 (list (> (count-if (lambda (x) (eq? x 'a)) xs) 100)
                       (> (count-if (lambda (x) (eq? x 'b)) xs) 100))))
       '(#t #t))
;; An exhausted generator is dropped, and the result ends only when every
;; input has.
(check gsampling-drops-the-exhausted
       (seeded (let* ((n 0)
                      (finite (lambda () (set! n (+ n 1)) (if (> n 3) (eof-object) 'x)))
                      (g (gsampling finite (lambda () 'y)))
                      (xs (take-n g 100)))
                 (list (= 0 (count-if eof-object? xs))
                       (<= (count-if (lambda (v) (eq? v 'x)) xs) 3))))
       '(#t #t))
(check gsampling-ends-when-all-do
       (let* ((g (gsampling (lambda () (eof-object)) (lambda () (eof-object)))))
         (eof-object? (g)))
       #t)

(test-failures)
