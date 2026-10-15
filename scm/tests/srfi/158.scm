;;; (srfi 158) generators and accumulators.
(import (scm test) (srfi 158) (srfi 160 f64) (srfi 194) (srfi 27)
        (scheme base))

(define (g->l g) (generator->list g))
(define (andmap p xs)
  (cond ((null? xs) #t) ((p (car xs)) (andmap p (cdr xs))) (else #f)))

;; ── constructors ─────────────────────────────────────────────────────
(check generator- (g->l (generator 1 2 3)) '(1 2 3))
(check generator-empty (g->l (generator)) '())
;; Exhaustion is permanent: a drained generator keeps giving eof.
(check exhaustion-sticks
       (let ((g (generator 1)))
         (list (g) (eof-object? (g)) (eof-object? (g))))
       '(1 #t #t))
(check circular (g->l (gtake (circular-generator 1 2) 5)) '(1 2 1 2 1))
(check circular-needs-a-value
       (guard (e (#t 'raised)) (circular-generator) 'no) 'raised)
(check iota- (g->l (make-iota-generator 4)) '(0 1 2 3))
(check iota-start (g->l (make-iota-generator 3 10)) '(10 11 12))
(check iota-step (g->l (make-iota-generator 3 0 5)) '(0 5 10))
(check range- (g->l (make-range-generator 2 5)) '(2 3 4))
(check range-step (g->l (make-range-generator 0 10 4)) '(0 4 8))
(check range-negative (g->l (make-range-generator 5 0 -2)) '(5 3 1))
;; With no end it is unbounded, which is why gtake exists.
(check range-unbounded (g->l (gtake (make-range-generator 7) 3)) '(7 8 9))
(check list->gen (g->l (list->generator '(a b))) '(a b))
(check vector->gen (g->l (vector->generator (vector 1 2 3))) '(1 2 3))
(check vector->gen-range (g->l (vector->generator (vector 1 2 3 4) 1 3)) '(2 3))
(check reverse-vector->gen (g->l (reverse-vector->generator (vector 1 2 3))) '(3 2 1))
(check reverse-vector->gen-range
       (g->l (reverse-vector->generator (vector 1 2 3 4) 1 3)) '(3 2))
(check string->gen (g->l (string->generator "ab")) '(#\a #\b))
(check string->gen-range (g->l (string->generator "abcd" 1 3)) '(#\b #\c))
(check bytevector->gen (g->l (bytevector->generator (bytevector 1 2))) '(1 2))
(check unfold-gen
       (g->l (make-unfold-generator (lambda (i) (= i 4)) (lambda (i) (* i i))
                                    (lambda (i) (+ i 1)) 0))
       '(0 1 4 9))
(check for-each-gen
       (g->l (make-for-each-generator for-each '(x y z))) '(x y z))
;; for-each over a vector works too, which is the point of taking the
;; iterator as an argument.
(check for-each-gen-vector
       (g->l (make-for-each-generator vector-for-each (vector 1 2))) '(1 2))

;; This implementation runs the producer eagerly, because continuations
;; are escape-only, so a finite producer works and a non-terminating one
;; raises rather than hanging.
(check coroutine-finite
       (g->l (make-coroutine-generator (lambda (yield) (yield 1) (yield 2))))
       '(1 2))
(check coroutine-yields-nothing
       (g->l (make-coroutine-generator (lambda (yield) (if #f #f)))) '())
(check coroutine-caps-a-runaway-producer
       (parameterize ((%coroutine-limit 50))
         (guard (e (#t 'raised))
           (make-coroutine-generator (lambda (yield) (let loop ((i 0)) (yield i) (loop (+ i 1)))))
           'no-raise))
       'raised)

;; ── operations ───────────────────────────────────────────────────────
(check gcons*- (g->l (gcons* 1 2 (generator 3 4))) '(1 2 3 4))
(check gappend- (g->l (gappend (generator 1 2) (generator 3))) '(1 2 3))
(check gappend-empty (g->l (gappend)) '())
(check gappend-skips-empties
       (g->l (gappend (generator) (generator 1) (generator))) '(1))
(check gflatten- (g->l (gflatten (generator '(1 2) '(3)))) '(1 2 3))
(check gflatten-empty-lists (g->l (gflatten (generator '() '(1) '()))) '(1))
(check ggroup- (g->l (ggroup (generator 1 2 3 4) 2)) '((1 2) (3 4)))
;; Without padding the last group is short; with it, every group is full.
(check ggroup-short-last (g->l (ggroup (generator 1 2 3) 2)) '((1 2) (3)))
(check ggroup-padded (g->l (ggroup (generator 1 2 3) 2 0)) '((1 2) (3 0)))
(check gmerge- (g->l (gmerge < (generator 1 4 7) (generator 2 3 8)))
       '(1 2 3 4 7 8))
(check gmerge-uneven (g->l (gmerge < (generator 1) (generator 2 3 4)))
       '(1 2 3 4))
(check gmerge-one (g->l (gmerge < (generator 3 1))) '(3 1))
(check gmap- (g->l (gmap (lambda (x) (* x x)) (generator 1 2 3))) '(1 4 9))
;; Several generators stop at the shortest.
(check gmap-many (g->l (gmap + (generator 1 2 9) (generator 10 20))) '(11 22))
(check gcombine-
       (g->l (gcombine (lambda (x acc) (values (+ x acc) (+ x acc))) 0
                       (generator 1 2 3)))
       '(1 3 6))
(check gfilter- (g->l (gfilter even? (generator 1 2 3 4))) '(2 4))
(check gremove- (g->l (gremove even? (generator 1 2 3 4))) '(1 3))
;; The decision may depend on everything seen so far.
(check gstate-filter-
       (g->l (gstate-filter (lambda (x sum) (values (> (+ x sum) 3) (+ x sum))) 0
                            (generator 1 1 1 1 1)))
       '(1 1))
(check gtake- (g->l (gtake (generator 1 2 3 4) 2)) '(1 2))
(check gtake-past-the-end (g->l (gtake (generator 1) 3)) '(1))
(check gtake-padded (g->l (gtake (generator 1) 3 'z)) '(1 z z))
(check gdrop- (g->l (gdrop (generator 1 2 3 4) 2)) '(3 4))
(check gdrop-past-the-end (g->l (gdrop (generator 1) 5)) '())
(check gtake-while- (g->l (gtake-while even? (generator 2 4 5 6))) '(2 4))
(check gtake-while-none (g->l (gtake-while even? (generator 1 2))) '())
(check gdrop-while- (g->l (gdrop-while even? (generator 2 4 5 6))) '(5 6))
(check gdrop-while-all (g->l (gdrop-while even? (generator 2 4))) '())
(check gdelete- (g->l (gdelete 2 (generator 1 2 3 2))) '(1 3))
(check gdelete-with-equality
       (g->l (gdelete "a" (generator "a" "b") string=?)) '("b"))
;; Only CONSECUTIVE duplicates go, which is why the 1 at the end stays.
(check gdelete-neighbor-dups-
       (g->l (gdelete-neighbor-dups (generator 1 1 2 2 2 3 1))) '(1 2 3 1))
(check gindex- (g->l (gindex (generator 'a 'b 'c 'd) (generator 0 2))) '(a c))
(check gselect- (g->l (gselect (generator 'a 'b 'c) (generator #t #f #t))) '(a c))

;; ── consumers ────────────────────────────────────────────────────────
(check to-list (generator->list (generator 1 2 3)) '(1 2 3))
(check to-list-bounded (generator->list (make-range-generator 0) 3) '(0 1 2))
(check to-reverse-list (generator->reverse-list (generator 1 2 3)) '(3 2 1))
(check to-vector (generator->vector (generator 1 2)) #(1 2))
(check to-vector-bounded (generator->vector (make-range-generator 0) 2) #(0 1))
(check to-string (generator->string (generator #\h #\i)) "hi")
;; Returns the count, which is the only way to tell a full vector from an
;; exhausted generator.
(check to-vector!
       (let* ((v (make-vector 4 0)) (n (generator->vector! v 1 (generator 7 8))))
         (list n v))
       '(2 #(0 7 8 0)))
(check to-vector!-stops-at-the-end
       (let* ((v (make-vector 2 0)) (n (generator->vector! v 0 (generator 1 2 3))))
         (list n v))
       '(2 #(1 2)))
;; The accumulator comes last, after the values.
(check fold- (generator-fold + 0 (generator 1 2 3)) 6)
(check fold-order (generator-fold cons '() (generator 1 2 3)) '(3 2 1))
(check fold-many (generator-fold (lambda (a b acc) (+ acc (* a b))) 0
                                 (generator 1 2) (generator 10 20))
       50)
(check for-each-
       (let ((n 0)) (generator-for-each (lambda (x) (set! n (+ n x))) (generator 1 2 3)) n)
       6)
(check for-each-many
       (let ((acc '()))
         (generator-for-each (lambda (a b) (set! acc (cons (list a b) acc)))
                             (generator 1 2) (generator 'x 'y))
         (reverse acc))
       '((1 x) (2 y)))
(check map->list (generator-map->list (lambda (x) (* 2 x)) (generator 1 2)) '(2 4))
(check find- (generator-find even? (generator 1 3 4 5)) 4)
(check find-none (generator-find even? (generator 1 3)) #f)
(check count- (generator-count even? (generator 1 2 3 4)) 2)
(check any- (generator-any (lambda (x) (and (even? x) (* x 10))) (generator 1 2 3)) 20)
(check any-none (generator-any even? (generator 1 3)) #f)
(check every- (generator-every (lambda (x) (* x 10)) (generator 1 2)) 20)
(check every-no (generator-every even? (generator 2 3)) #f)
(check every-empty (generator-every even? (generator)) #t)
;; `find` and `any` must stop rather than drain, which only a count shows.
(check find-stops-early
       (let ((n 0))
         (generator-find (lambda (x) (set! n (+ n 1)) (> x 2)) (make-range-generator 0))
         n)
       4)
(check unfold-
       (generator-unfold (generator 1 2 3)
                         (lambda (stop? map succ seed)
                           (let loop ((s seed) (acc '()))
                             (if (stop? s) (reverse acc) (loop (succ s) (cons (map s) acc))))))
       '(1 2 3))

;; ── accumulators ─────────────────────────────────────────────────────
(define (feed acc . xs)
  (for-each acc xs)
  (acc (eof-object)))
(check make-accumulator-
       (feed (make-accumulator + 0 (lambda (s) (* s 10))) 1 2 3) 60)
(check count-accumulator- (feed (count-accumulator) 'a 'b 'c) 3)
(check list-accumulator- (feed (list-accumulator) 1 2 3) '(1 2 3))
(check reverse-list-accumulator- (feed (reverse-list-accumulator) 1 2 3) '(3 2 1))
(check vector-accumulator- (feed (vector-accumulator) 1 2) #(1 2))
(check reverse-vector-accumulator- (feed (reverse-vector-accumulator) 1 2) #(2 1))
(check vector-accumulator!-
       (let ((v (make-vector 4 0))) (feed (vector-accumulator! v 1) 7 8)) #(0 7 8 0))
(check string-accumulator- (feed (string-accumulator) #\h #\i) "hi")
(check bytevector-accumulator- (feed (bytevector-accumulator) 1 2) #u8(1 2))
(check bytevector-accumulator!-
       (let ((b (bytevector 0 0 0))) (feed (bytevector-accumulator! b 1) 9)) #u8(0 9 0))
(check sum-accumulator- (feed (sum-accumulator) 1 2 3) 6)
(check product-accumulator- (feed (product-accumulator) 2 3 4) 24)
(check empty-accumulators
       (list ((count-accumulator) (eof-object))
             ((list-accumulator) (eof-object))
             ((sum-accumulator) (eof-object))
             ((product-accumulator) (eof-object)))
       '(0 () 0 1))

;; ── the whole point: pipelines without intermediate lists ────────────
;; Pull a hundred thousand values, keep a handful, build nothing in
;; between. This would be a hundred-thousand-element list otherwise.
(check a-lazy-pipeline
       (generator->list
        (gtake (gfilter (lambda (n) (= 0 (modulo n 9999)))
                        (make-range-generator 1 100000))
               4))
       '(9999 19998 29997 39996))

;; ── the libraries that were already producing generators ─────────────
;; `(srfi 160)` ships a `make-@vector-generator` for every element type
;; and, until this library, there was nothing to consume one.
(check numeric-vector-generator
       (generator->list (make-f64vector-generator (f64vector 1.5 2.5))) '(1.5 2.5))
(check numeric-vector-through-a-pipeline
       (generator->list (gmap (lambda (x) (* 2 x))
                              (make-f64vector-generator (f64vector 1.0 2.0 3.0))))
       '(2.0 4.0 6.0))
;; `(srfi 194)`'s samplers are generators too, so they compose with these.
(check sampler-through-a-pipeline
       (parameterize ((current-random-source (make-random-source 11)))
         (let ((xs (generator->list (gtake (make-random-real-generator 0.0 1.0) 50))))
           (list (length xs) (andmap (lambda (x) (and (>= x 0.0) (<= x 1.0))) xs))))
       '(50 #t))
(check sphere-sampler-is-a-generator
       (parameterize ((current-random-source (make-random-source 12)))
         (let ((vs (generator->list (gtake (make-sphere-generator 2) 10))))
           (list (length vs) (andmap (lambda (v) (= 3 (vector-length v))) vs))))
       '(10 #t))

(test-failures)
