;;; (srfi 63) — arrays, shared arrays, and equal? over them.
(import (scm test) (srfi 63) (srfi 4))

(define (fill-2x3!)
  (let ((a (make-array a:fixz32b 2 3)))
    (let loop ((i 0))
      (when (< i 2)
        (let inner ((j 0))
          (when (< j 3)
            (array-set! a (+ (* 10 i) j) i j)
            (inner (+ j 1))))
        (loop (+ i 1))))
    a))

;; shape and access
(check array? (array? (make-array a:flor64b 2 2)) #t)
(check array?-no (array? (vector 1 2)) #f)
(check rank (array-rank (make-array a:flor64b 2 3)) 2)
(check rank-1 (array-rank (make-array a:flor64b 4)) 1)
(check rank-3 (array-rank (make-array a:flor64b 2 2 2)) 3)
;; SRFI 63: everything that is not an array has rank 0.
(check rank-of-non-array (array-rank 7) 0)
(check dimensions (array-dimensions (make-array a:flor64b 2 3)) '(2 3))
(check dimensions-1 (array-dimensions (make-array a:flor64b 5)) '(5))

(check ref-and-set (array->list (fill-2x3!)) '(0 1 2 10 11 12))
(check ref-corner (array-ref (fill-2x3!) 1 2) 12)
(check in-bounds (array-in-bounds? (fill-2x3!) 1 2) #t)
(check out-of-bounds (array-in-bounds? (fill-2x3!) 2 0) #f)
(check out-of-bounds-negative (array-in-bounds? (fill-2x3!) -1 0) #f)
(check wrong-rank-is-out-of-bounds (array-in-bounds? (fill-2x3!) 1) #f)
(check ref-out-of-range (array-ref (fill-2x3!) 2 0)
       '(raised "array-ref: subscript 2 is out of range for dimension 0 of 2"))
(check ref-wrong-rank (array-ref (fill-2x3!) 1)
       '(raised "array-ref: this array has rank 2, but 1 subscript was given"))

;; the element type is the store's
(check element-type-enforced
       (let ((a (make-array a:fixz8b 1))) (array-set! a 200 0) a)
       '(raised "array-set!: 200 is out of range for a s8vector"))
(check float-store (let ((a (make-array a:flor64b 1))) (array-set! a 1.5 0) (array-ref a 0)) 1.5)
(check bool-store
       (let ((a (make-array a:bool 2))) (array-set! a #t 0) (list (array-ref a 0) (array-ref a 1)))
       '(#t #f))
(check vector-as-prototype
       (let ((a (make-array (vector) 2))) (array-set! a 'x 0) (array-ref a 0))
       'x)
(check string-as-prototype
       (let ((a (make-array (string) 2))) (array-set! a #\a 0) (array-ref a 0))
       #\a)
(check u8-prototype (array-ref (make-array a:fixn8b 2) 0) 0)

;; shared arrays: views that share the store
(check transpose
       (let* ((a (fill-2x3!)) (t (make-shared-array a (lambda (i j) (list j i)) 3 2)))
         (list (array-dimensions t) (array->list t)))
       '((3 2) (0 10 1 11 2 12)))
(check row
       (array->list (make-shared-array (fill-2x3!) (lambda (j) (list 1 j)) 3))
       '(10 11 12))
(check column
       (array->list (make-shared-array (fill-2x3!) (lambda (i) (list i 2)) 2))
       '(2 12))
(check subrectangle
       (array->list (make-shared-array (fill-2x3!) (lambda (i j) (list i (+ j 1))) 2 2))
       '(1 2 11 12))
;; A view writes through to the array it was made from.
(check view-writes-through
       (let* ((a (fill-2x3!)) (t (make-shared-array a (lambda (i j) (list j i)) 3 2)))
         (array-set! t 99 0 1)
         (list (array-ref a 1 0) (array-ref t 0 1)))
       '(99 99))
;; A view of a view.
(check view-of-view
       (let* ((a (fill-2x3!))
              (t (make-shared-array a (lambda (i j) (list j i)) 3 2))
              (r (make-shared-array t (lambda (j) (list 2 j)) 2)))
         (array->list r))
       '(2 12))
;; A reversing view.
(check reversed
       (array->list (make-shared-array (fill-2x3!) (lambda (j) (list 0 (- 2 j))) 3))
       '(2 1 0))
;; A mapper that reaches outside the store is refused, not read.
(check view-out-of-store
       (make-shared-array (fill-2x3!) (lambda (i j) (list i j)) 3 3)
       '(raised "%make-array-view: these dimensions and strides reach outside the store"))

;; equal? is extended to arrays: same shape, equal elements, any strides
(check equal-self (let ((a (fill-2x3!))) (equal? a a)) #t)
(check equal-independent-copies (equal? (fill-2x3!) (fill-2x3!)) #t)
(check equal-different-shape
       (equal? (make-array a:flor64b 2 3) (make-array a:flor64b 3 2)) #f)
(check equal-different-elements
       (let ((a (make-array a:flor64b 1)) (b (make-array a:flor64b 1)))
         (array-set! a 1.0 0)
         (equal? a b))
       #f)
;; A transpose equals a materialised copy of itself, which is the case the
;; extension exists for.
(check equal-transpose-vs-copy
       (let* ((a (fill-2x3!))
              (t (make-shared-array a (lambda (i j) (list j i)) 3 2))
              (c (make-array a:fixz32b 3 2)))
         (let loop ((i 0))
           (when (< i 3)
             (let inner ((j 0))
               (when (< j 2)
                 (array-set! c (array-ref t i j) i j)
                 (inner (+ j 1))))
             (loop (+ i 1))))
         (list (equal? t c) (equal? c t)))
       '(#t #t))
;; Different element types holding the same numbers still compare equal,
;; because `equal?` compares the elements and 0 is 0.
(check equal-across-store-types
       (equal? (make-array a:fixz32b 2) (make-array a:fixz8b 2)) #t)
;; An array is not a vector, not a record, not its store.
(check array-is-disjoint
       (let ((a (make-array a:flor64b 1)))
         (list (vector? a) (f64vector? a) (array? a) (equal? a #f64(0))))
       '(#f #f #t #f))

;; conversions
(check ->vector (array->vector (fill-2x3!)) (vector 0 1 2 10 11 12))
(check ->list-rank-3
       (let ((a (make-array a:fixz32b 2 1 2)))
         (array-set! a 7 1 0 1)
         (array->list a))
       '(0 0 0 7))

;; a big one: an image-shaped f64 array
(check big-image
       (let ((img (make-array a:flor64b 64 64)))
         (array-set! img 0.5 32 32)
         (list (array-dimensions img) (array-ref img 32 32) (array-ref img 0 0)))
       '((64 64) 0.5 0.0))

(test-failures)
