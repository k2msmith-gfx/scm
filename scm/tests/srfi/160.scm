;;; (srfi 160) per-type numeric vector operations.
;;;
;;; The operations are written once and aliased per type, so the bulk of
;;; this exercises `f64` and then checks that another type and the
;;; bytevector-backed `u8` behave the same way through their own names.
(import (scm test) (srfi 160 f64) (srfi 160 s32) (srfi 160 u8)
        (srfi 128) (scheme base) (scheme write))

(define (fv . xs) (list->f64vector xs))

;; ── constructors ─────────────────────────────────────────────────────
(check make (f64vector-length (make-f64vector 3)) 3)
(check make-fill (f64vector->list (make-f64vector 2 1.5)) '(1.5 1.5))
(check ctor (f64vector->list (f64vector 1.0 2.0)) '(1.0 2.0))
(check from-list (f64vector->list (list->f64vector '(1.0 2.0))) '(1.0 2.0))
;; `f` is applied to the index and the seed, and returns the element and
;; the next seed.
(check unfold
       (f64vector->list
        (f64vector-unfold (lambda (i s) (values (* 1.0 s) (* s 2))) 4 1))
       '(1.0 2.0 4.0 8.0))
;; Right to left, so the last index gets the first seed.
(check unfold-right
       (f64vector->list
        (f64vector-unfold-right (lambda (i s) (values (* 1.0 s) (+ s 1))) 3 0))
       '(2.0 1.0 0.0))
(check copy (f64vector->list (f64vector-copy (fv 1 2 3))) '(1.0 2.0 3.0))
(check copy-range (f64vector->list (f64vector-copy (fv 1 2 3 4) 1 3)) '(2.0 3.0))
(check reverse-copy (f64vector->list (f64vector-reverse-copy (fv 1 2 3)))
       '(3.0 2.0 1.0))
(check reverse-copy-range
       (f64vector->list (f64vector-reverse-copy (fv 1 2 3 4) 1 3)) '(3.0 2.0))
(check append (f64vector->list (f64vector-append (fv 1) (fv 2 3))) '(1.0 2.0 3.0))
;; No arguments still has to produce an f64vector, which is why these
;; constructors are the ones that have to be told the element type.
(check append-none (f64vector? (f64vector-append)) #t)
(check concatenate
       (f64vector->list (f64vector-concatenate (list (fv 1) (fv 2 3))))
       '(1.0 2.0 3.0))
(check concatenate-empty (f64vector-length (f64vector-concatenate '())) 0)
(check append-subvectors
       (f64vector->list (f64vector-append-subvectors (fv 1 2 3) 0 2 (fv 4 5 6) 2 3))
       '(1.0 2.0 6.0))

;; ── predicates ───────────────────────────────────────────────────────
(check pred (f64vector? (fv 1)) #t)
(check pred-no (f64vector? (vector 1)) #f)
;; The element predicate: can this value be stored without loss?
(check element-yes (f64? 1.5) #t)
(check element-int (f64? 2) #t)
(check element-no (f64? 'a) #f)
(check element-s32-range (s32? 2147483648) #f)
(check element-s32-in (s32? 2147483647) #t)
(check element-u8-range (list (u8? -1) (u8? 0) (u8? 255) (u8? 256))
       '(#f #t #t #f))
(check element-exactness (s32? 1.5) #f)
(check empty (f64vector-empty? (f64vector)) #t)
(check empty-no (f64vector-empty? (fv 1)) #f)
(check equal- (f64vector= (fv 1 2) (fv 1 2)) #t)
(check equal-length (f64vector= (fv 1) (fv 1 2)) #f)
(check equal-many (f64vector= (fv 1) (fv 1) (fv 1)) #t)
(check equal-many-no (f64vector= (fv 1) (fv 1) (fv 2)) #f)

;; ── selection ────────────────────────────────────────────────────────
(check ref (f64vector-ref (fv 1 2 3) 1) 2.0)
(check length- (f64vector-length (fv 1 2 3)) 3)
(check take (f64vector->list (f64vector-take (fv 1 2 3 4) 2)) '(1.0 2.0))
(check take-right (f64vector->list (f64vector-take-right (fv 1 2 3 4) 2)) '(3.0 4.0))
(check drop (f64vector->list (f64vector-drop (fv 1 2 3 4) 2)) '(3.0 4.0))
(check drop-right (f64vector->list (f64vector-drop-right (fv 1 2 3 4) 2)) '(1.0 2.0))
;; The last piece is short when the length does not divide.
(check segment
       (map f64vector->list (f64vector-segment (fv 1 2 3 4 5) 2))
       '((1.0 2.0) (3.0 4.0) (5.0)))

;; ── iteration ────────────────────────────────────────────────────────
(check fold (f64vector-fold + 0 (fv 1 2 3)) 6.0)
;; The accumulator comes first, so the order of the two is visible here.
(check fold-order (f64vector-fold (lambda (acc x) (cons x acc)) '() (fv 1 2 3))
       '(3.0 2.0 1.0))
(check fold-right (f64vector-fold (lambda (acc x) (cons x acc)) '() (fv 1 2 3))
       '(3.0 2.0 1.0))
(check fold-right-order
       (f64vector-fold-right (lambda (acc x) (cons x acc)) '() (fv 1 2 3))
       '(1.0 2.0 3.0))
(check map- (f64vector->list (f64vector-map (lambda (x) (* 2 x)) (fv 1 2))) '(2.0 4.0))
;; Several vectors stop at the shortest, as `map` does on lists.
(check map-many
       (f64vector->list (f64vector-map + (fv 1 2 9) (fv 10 20)))
       '(11.0 22.0))
(check map!
       (let ((v (fv 1 2 3)))
         (f64vector-map! (lambda (x) (* 10 x)) v)
         (f64vector->list v))
       '(10.0 20.0 30.0))
(check for-each
       (let ((n 0)) (f64vector-for-each (lambda (x) (set! n (+ n x))) (fv 1 2 3)) n)
       6.0)
(check count (f64vector-count (lambda (x) (> x 1)) (fv 1 2 3)) 2)
(check count-many (f64vector-count < (fv 1 2 3) (fv 5 0 9)) 2)
;; out[i] is f applied to the running value and v[i].
(check cumulate (f64vector->list (f64vector-cumulate + 0 (fv 1 2 3))) '(1.0 3.0 6.0))

;; ── searching ────────────────────────────────────────────────────────
(check take-while
       (f64vector->list (f64vector-take-while (lambda (x) (< x 3)) (fv 1 2 3 1)))
       '(1.0 2.0))
(check drop-while
       (f64vector->list (f64vector-drop-while (lambda (x) (< x 3)) (fv 1 2 3 1)))
       '(3.0 1.0))
;; The `-right` pair work on the suffix, not the prefix.
(check take-while-right
       (f64vector->list (f64vector-take-while-right (lambda (x) (< x 3)) (fv 1 3 1 2)))
       '(1.0 2.0))
(check drop-while-right
       (f64vector->list (f64vector-drop-while-right (lambda (x) (< x 3)) (fv 1 3 1 2)))
       '(1.0 3.0))
(check index (f64vector-index (lambda (x) (= x 2)) (fv 1 2 3 2)) 1)
(check index-none (f64vector-index (lambda (x) (= x 9)) (fv 1 2)) #f)
(check index-right (f64vector-index-right (lambda (x) (= x 2)) (fv 1 2 3 2)) 3)
;; `skip` is `index` of the complement.
(check skip (f64vector-skip (lambda (x) (= x 1)) (fv 1 1 2)) 2)
(check skip-right (f64vector-skip-right (lambda (x) (= x 1)) (fv 2 1 1)) 0)
;; `any` gives the first true result, not the index.
(check any (f64vector-any (lambda (x) (and (> x 1) (* 10 x))) (fv 1 2 3)) 20.0)
(check any-none (f64vector-any (lambda (x) (> x 9)) (fv 1 2)) #f)
;; `every` gives the last result, and is true of an empty vector.
(check every (f64vector-every (lambda (x) (* 10 x)) (fv 1 2)) 20.0)
(check every-no (f64vector-every (lambda (x) (> x 1)) (fv 1 2)) #f)
(check every-empty (f64vector-every (lambda (x) #f) (f64vector)) #t)
;; Two values: the satisfying elements first, then how many satisfied --
;; which is where the single result vector divides.
;; When nothing satisfies, the count is 0 and the order is untouched.
(check partition-none
       (call-with-values (lambda () (f64vector-partition (lambda (x) #f) (fv 1 2 3 4)))
         (lambda (v n) (list (f64vector->list v) n)))
       '((1.0 2.0 3.0 4.0) 0))
;; When everything does, the count is the length.
(check partition-all
       (call-with-values (lambda () (f64vector-partition (lambda (x) #t) (fv 1 2)))
         (lambda (v n) (list (f64vector->list v) n)))
       '((1.0 2.0) 2))
(check partition-split
       (call-with-values (lambda () (f64vector-partition (lambda (x) (> x 2)) (fv 1 3 2 4)))
         (lambda (v n) (list (f64vector->list v) n)))
       '((3.0 4.0 1.0 2.0) 2))
(check filter
       (f64vector->list (f64vector-filter (lambda (x) (> x 2)) (fv 1 3 2 4)))
       '(3.0 4.0))
(check remove
       (f64vector->list (f64vector-remove (lambda (x) (> x 2)) (fv 1 3 2 4)))
       '(1.0 2.0))

;; ── mutation ─────────────────────────────────────────────────────────
(check set! (let ((v (fv 1 2))) (f64vector-set! v 0 9.0) (f64vector->list v))
       '(9.0 2.0))
(check swap! (let ((v (fv 1 2 3))) (f64vector-swap! v 0 2) (f64vector->list v))
       '(3.0 2.0 1.0))
(check fill! (let ((v (fv 1 2 3))) (f64vector-fill! v 0.0) (f64vector->list v))
       '(0.0 0.0 0.0))
(check fill-range! (let ((v (fv 1 2 3))) (f64vector-fill! v 0.0 1) (f64vector->list v))
       '(1.0 0.0 0.0))
(check reverse! (let ((v (fv 1 2 3))) (f64vector-reverse! v) (f64vector->list v))
       '(3.0 2.0 1.0))
(check reverse-range!
       (let ((v (fv 1 2 3 4))) (f64vector-reverse! v 1 3) (f64vector->list v))
       '(1.0 3.0 2.0 4.0))
(check copy!
       (let ((v (make-f64vector 4 0.0))) (f64vector-copy! v 1 (fv 7 8))
            (f64vector->list v))
       '(0.0 7.0 8.0 0.0))
(check reverse-copy!
       (let ((v (make-f64vector 4 0.0))) (f64vector-reverse-copy! v 1 (fv 7 8))
            (f64vector->list v))
       '(0.0 8.0 7.0 0.0))
;; A target overlapping its own source is the usual reason to call this,
;; so it must not read what it has already written.
(check reverse-copy-overlapping
       (let ((v (fv 1 2 3 4))) (f64vector-reverse-copy! v 0 v) (f64vector->list v))
       '(4.0 3.0 2.0 1.0))
(check unfold!
       (let ((v (make-f64vector 4 0.0)))
         (f64vector-unfold! (lambda (i s) (values (* 1.0 s) (+ s 1))) v 1 3 5)
         (f64vector->list v))
       '(0.0 5.0 6.0 0.0))
(check unfold-right!
       (let ((v (make-f64vector 4 0.0)))
         (f64vector-unfold-right! (lambda (i s) (values (* 1.0 s) (+ s 1))) v 1 3 5)
         (f64vector->list v))
       '(0.0 6.0 5.0 0.0))

;; ── conversion ───────────────────────────────────────────────────────
(check to-list (f64vector->list (fv 1 2 3)) '(1.0 2.0 3.0))
(check to-list-range (f64vector->list (fv 1 2 3 4) 1 3) '(2.0 3.0))
(check reverse-to-list (reverse-f64vector->list (fv 1 2 3)) '(3.0 2.0 1.0))
(check reverse-from-list (f64vector->list (reverse-list->f64vector '(1.0 2.0)))
       '(2.0 1.0))
(check to-vector (f64vector->vector (fv 1 2)) #(1.0 2.0))
(check to-vector-range (f64vector->vector (fv 1 2 3) 1) #(2.0 3.0))
(check from-vector (f64vector->list (vector->f64vector #(1.0 2.0))) '(1.0 2.0))
(check from-vector-range
       (f64vector->list (vector->f64vector #(1.0 2.0 3.0) 1)) '(2.0 3.0))

;; ── generators, comparators, writing ─────────────────────────────────
;; A SRFI 158 generator: each element in turn, then the end-of-file object.
(check generator
       (let ((g (make-f64vector-generator (fv 1 2))))
         (list (g) (g) (eof-object? (g))))
       '(1.0 2.0 #t))
(check comparator (comparator? f64vector-comparator) #t)
(check comparator-equality
       ((comparator-equality-predicate f64vector-comparator) (fv 1 2) (fv 1 2))
       #t)
(check comparator-type
       (list ((comparator-type-test-predicate f64vector-comparator) (fv 1))
             ((comparator-type-test-predicate f64vector-comparator) (vector 1)))
       '(#t #f))
;; Lexicographic by element, then by length.
(check comparator-order
       (list ((comparator-ordering-predicate f64vector-comparator) (fv 1 2) (fv 1 3))
             ((comparator-ordering-predicate f64vector-comparator) (fv 1) (fv 1 0))
             ((comparator-ordering-predicate f64vector-comparator) (fv 2) (fv 1)))
       '(#t #t #f))
;; The external representation the reader accepts back.
(check write-
       (let ((p (open-output-string)))
         (write-f64vector (fv 1 2) p)
         (get-output-string p))
       "#f64(1.0 2.0)")
(check write-reads-back
       (f64vector->list (read (open-input-string
                               (let ((p (open-output-string)))
                                 (write-f64vector (fv 1 2) p)
                                 (get-output-string p)))))
       '(1.0 2.0))

;; ── another type, through its own names ──────────────────────────────
(check s32-map (s32vector->list (s32vector-map (lambda (x) (* 2 x)) (s32vector 1 2)))
       '(2 4))
(check s32-stays-s32 (s32vector? (s32vector-map (lambda (x) x) (s32vector 1))) #t)
(check s32-filter
       (s32vector->list (s32vector-filter even? (s32vector 1 2 3 4))) '(2 4))
(check s32-write
       (let ((p (open-output-string))) (write-s32vector (s32vector 1 2) p)
            (get-output-string p))
       "#s32(1 2)")

;; ── u8, which is the bytevector ──────────────────────────────────────
;; `u8vector` is R7RS's bytevector, so these operations work on one -- the
;; shape an 8-bit image buffer has.
(check u8-is-a-bytevector (bytevector? (u8vector 1 2)) #t)
(check u8-map (u8vector->list (u8vector-map (lambda (x) (* 2 x)) (u8vector 1 2)))
       '(2 4))
(check u8-map-gives-a-bytevector
       (bytevector? (u8vector-map (lambda (x) x) (u8vector 1))) #t)
(check u8-fold (u8vector-fold + 0 (u8vector 1 2 3)) 6)
(check u8-from-bytevector-literal (u8vector-fold + 0 #u8(1 2 3)) 6)
(check u8-count (u8vector-count odd? (u8vector 1 2 3)) 2)
(check u8-take (u8vector->list (u8vector-take (u8vector 1 2 3) 2)) '(1 2))
(check u8-reverse!
       (let ((v (u8vector 1 2 3))) (u8vector-reverse! v) (u8vector->list v))
       '(3 2 1))
(check u8-append (u8vector->list (u8vector-append (u8vector 1) (u8vector 2))) '(1 2))
(check u8-write
       (let ((p (open-output-string))) (write-u8vector (u8vector 1 2) p)
            (get-output-string p))
       "#u8(1 2)")
;; An octet out of range has to be refused rather than wrapped.
(check-raises u8-range (u8vector-set! (u8vector 1) 0 256))

;; ── the base library is the same bindings ────────────────────────────
;; Importing it beside a per-type library must be harmless: the eight core
;; names mean the same binding in both.
(import (srfi 160 base))
(check base-same-binding (f64vector->list (f64vector 1.0 2.0)) '(1.0 2.0))
(check base-u8 (bytevector? (u8vector 1)) #t)

(test-failures)
