;;; (srfi 2), (srfi 8), (srfi 26), (srfi 111) and (srfi 132).
(import (scm test) (srfi 2) (srfi 8) (srfi 26) (srfi 111) (srfi 132)
        (only (srfi 1) iota))

;; ── (srfi 2) and-let* ────────────────────────────────────────────────
(check and-let*-empty (and-let* ()) #t)
(check and-let*-body (and-let* () 1 2) 2)
(check and-let*-bind (and-let* ((x 5) (y (+ x 1))) (* x y)) 30)
(check and-let*-false (and-let* ((x #f) (y (error "never"))) y) #f)
(check and-let*-test (and-let* ((x 2) ((even? x))) 'yes) 'yes)
(check and-let*-test-fails (and-let* ((x 3) ((even? x))) 'yes) #f)
(check and-let*-last (and-let* ((x 1) (y 2))) 2)
(check and-let*-bound (let ((v 7)) (and-let* (v) v)) 7)
(check and-let*-bound-false (let ((v #f)) (and-let* (v) 'yes)) #f)

;; ── (srfi 8) receive ─────────────────────────────────────────────────
(check receive (receive (a b) (values 1 2) (list a b)) '(1 2))
(check receive-rest (receive (a . rest) (values 1 2 3) (list a rest))
       '(1 (2 3)))
(check receive-all (receive all (values 1 2) all) '(1 2))

;; ── (srfi 26) cut and cute ───────────────────────────────────────────
(check cut-none ((cut list)) '())
(check cut-one ((cut list 1 <>) 2) '(1 2))
(check cut-two ((cut list <> 2 <>) 1 3) '(1 2 3))
(check cut-rest ((cut list 1 <...>) 2 3) '(1 2 3))
(check cut-rest-only ((cut list <...>) 1 2) '(1 2))
(check cut-mixed ((cut list <> 'x <...>) 1 2 3) '(1 x 2 3))
(check cut-proc-slot ((cut <> 1 2) list) '(1 2))
;; `cut` re-evaluates its non-slot expressions at every call, `cute` once.
(check cut-reevaluates
       (let* ((n 0) (f (cut list (begin (set! n (+ n 1)) n) <>)))
         (list (f 'a) (f 'b)))
       '((1 a) (2 b)))
(check cute-evaluates-once
       (let* ((n 0) (f (cute list (begin (set! n (+ n 1)) n) <>)))
         (list (f 'a) (f 'b)))
       '((1 a) (1 b)))
(check cute-rest ((cute list 1 <...>) 2 3) '(1 2 3))

;; ── (srfi 111) boxes ─────────────────────────────────────────────────
(check box-roundtrip (unbox (box 42)) 42)
(check box? (list (box? (box 1)) (box? 1)) '(#t #f))
(check set-box! (let ((b (box 1))) (set-box! b 2) (unbox b)) 2)
(check box-identity (let ((b (box 1))) (eq? b b)) #t)
(check box-distinct (equal? (box 1) (box 1)) #f)

;; ── (srfi 132) sorting ───────────────────────────────────────────────
(check list-sorted? (list (list-sorted? < '(1 2 2 3)) (list-sorted? < '(2 1)))
       '(#t #f))
(check list-sorted?-empty (list-sorted? < '()) #t)
(check vector-sorted? (vector-sorted? < (vector 1 2 3)) #t)
(check vector-sorted?-range (vector-sorted? < (vector 9 1 2 9) 1 3) #t)
(check list-sort (list-sort < '(3 1 2)) '(1 2 3))
(check list-sort-empty (list-sort < '()) '())
(check list-sort-one (list-sort < '(5)) '(5))
(check list-sort-dups (list-sort < '(2 1 2 1)) '(1 1 2 2))
(check list-sort-big
       (equal? (list-sort < (reverse (iota 50))) (iota 50))
       #t)
;; Stability: pairs with equal keys keep their input order.
(check list-stable-sort
       (list-stable-sort (lambda (a b) (< (car a) (car b)))
                         '((1 a) (0 x) (1 b) (0 y)))
       '((0 x) (0 y) (1 a) (1 b)))
(check vector-sort (vector->list (vector-sort < (vector 3 1 2))) '(1 2 3))
(check vector-sort-range
       (vector->list (vector-sort < (vector 9 3 1 2 9) 1 4))
       '(1 2 3))
(check vector-sort! (let ((v (vector 3 1 2))) (vector-sort! < v) (vector->list v))
       '(1 2 3))
(check vector-sort!-range
       (let ((v (vector 9 3 1 2 8))) (vector-sort! < v 1 4) (vector->list v))
       '(9 1 2 3 8))
(check list-merge (list-merge < '(1 3) '(2 4)) '(1 2 3 4))
(check list-merge-empty (list-merge < '() '(1)) '(1))
(check vector-merge (vector->list (vector-merge < (vector 1 3) (vector 2 4)))
       '(1 2 3 4))
(check vector-merge!
       (let ((to (make-vector 4 0)))
         (vector-merge! < to (vector 1 3) (vector 2 4))
         (vector->list to))
       '(1 2 3 4))
(check list-delete-neighbor-dups
       (list-delete-neighbor-dups equal? '(1 1 2 2 2 3 1)) '(1 2 3 1))
(check list-delete-neighbor-dups-empty
       (list-delete-neighbor-dups equal? '()) '())
(check vector-delete-neighbor-dups
       (vector->list (vector-delete-neighbor-dups equal? (vector 1 1 2 3 3)))
       '(1 2 3))
(check vector-delete-neighbor-dups!
       (let* ((v (vector 1 1 2 3 3)) (n (vector-delete-neighbor-dups! equal? v)))
         (list n (vector->list (vector-copy v 0 n))))
       '(3 (1 2 3)))
(check vector-select! (vector-select! < (vector 5 1 3) 1) 3)
(check vector-find-median-odd (vector-find-median < (vector 3 1 2) 'none) 2)
(check vector-find-median-even (vector-find-median < (vector 1 2 3 4) (quote none)) 2.5)
(check vector-find-median-empty (vector-find-median < (vector) 'none) 'none)

(test-failures)
