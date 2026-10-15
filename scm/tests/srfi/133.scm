;;; (srfi 133) — vector operations on boxed vectors.
;;;
;;; The three argument orders this SRFI is easy to get backwards -- `kons`
;;; takes the state first, `cmp` takes the element first, and
;;; `vector-unfold-right` fills from the end -- are each pinned with a
;;; case that would come out differently the other way round.
(import (scm test) (srfi 133))

;; ── constructors: vector-unfold ──────────────────────────────────────
;;
;; Zero seeds: `f` sees only the index and returns only the element.
(check unfold-no-seed (vector-unfold (lambda (i) (* i i)) 5) '#(0 1 4 9 16))
(check unfold-no-seed-empty (vector-unfold (lambda (i) i) 0) '#())
;; One seed: `f` returns the element and the next seed.
(check unfold-one-seed
       (vector-unfold (lambda (i x) (values x (- x 1))) 5 0)
       '#(0 -1 -2 -3 -4))
;; Two seeds: the element, then one new value per seed.
(check unfold-two-seeds
       (vector-unfold (lambda (i a b) (values (list a b) b (+ a b))) 5 0 1)
       '#((0 1) (1 1) (1 2) (2 3) (3 5)))
(check unfold-index-is-passed
       (vector-unfold (lambda (i x) (values (cons i x) x)) 3 'k)
       '#((0 . k) (1 . k) (2 . k)))

;; `vector-unfold-right` fills from the last slot backwards, so the first
;; element `f` yields lands at the END. Reversed, this test would be
;; #(0 1 2 3 4).
(check unfold-right-fills-backwards
       (vector-unfold-right (lambda (i x) (values x (+ x 1))) 5 0)
       '#(4 3 2 1 0))
(check unfold-right-no-seed
       (vector-unfold-right (lambda (i) i) 4)
       '#(0 1 2 3))
;; The SRFI's own idiom: unfold-right reverses a vector.
(check unfold-right-reverses
       (let ((v '#(a b c d e)))
         (vector-unfold-right (lambda (i x) (values (vector-ref v x) (+ x 1)))
                              (vector-length v) 0))
       '#(e d c b a))

;; ── constructors: copies and appends ─────────────────────────────────
(check reverse-copy (vector-reverse-copy '#(1 2 3 4)) '#(4 3 2 1))
(check reverse-copy-range (vector-reverse-copy '#(1 2 3 4 5) 1 4) '#(4 3 2))
(check reverse-copy-start-only (vector-reverse-copy '#(1 2 3 4) 2) '#(4 3))
(check reverse-copy-empty (vector-reverse-copy '#(1 2 3) 1 1) '#())
(check concatenate (vector-concatenate '(#(1 2) #(3) #(4 5))) '#(1 2 3 4 5))
(check concatenate-empty (vector-concatenate '()) '#())
(check concatenate-of-empties (vector-concatenate '(#() #())) '#())
(check append-subvectors
       (vector-append-subvectors '#(1 2 3 4) 1 3 '#(a b c) 0 2)
       '#(2 3 a b))
(check append-subvectors-none (vector-append-subvectors) '#())
(check append-subvectors-one (vector-append-subvectors '#(1 2 3) 0 3) '#(1 2 3))

;; ── predicates ───────────────────────────────────────────────────────
(check empty-yes (vector-empty? '#()) #t)
(check empty-no (vector-empty? '#(1)) #f)
;; Zero or one vector is #t by specification, not by accident.
(check vector=-zero (vector= eq?) #t)
(check vector=-one (vector= eq? '#(1 2 3)) #t)
(check vector=-two (vector= eq? '#(1 2) '#(1 2)) #t)
(check vector=-unequal-length (vector= eq? '#(1 2) '#(1 2 3)) #f)
(check vector=-three (vector= eq? '#(1) '#(1) '#(1)) #t)
(check vector=-three-last-differs (vector= eq? '#(1) '#(1) '#(2)) #f)
(check vector=-both-empty (vector= eq? '#() '#()) #t)
;; The element predicate is the caller's, so `equal?` sees deeper than `eq?`.
(check vector=-uses-elt= (vector= equal? '#("a") '#("a")) #t)

;; ── iteration: fold takes the STATE FIRST ────────────────────────────
;;
;; `(kons state elt)`, the opposite of SRFI 1's `fold`. A `cons` that
;; built the pair the other way round would give (1 2 3) here.
(check fold-state-first
       (vector-fold (lambda (acc elt) (cons elt acc)) '() '#(1 2 3))
       '(3 2 1))
(check fold-left-to-right
       (vector-fold (lambda (acc elt) (string-append acc elt)) "" '#("a" "b" "c"))
       "abc")
(check fold-empty (vector-fold (lambda (acc elt) (+ acc elt)) 99 '#()) 99)
;; fold-right visits right to left and the state is STILL first.
(check fold-right-state-first
       (vector-fold-right (lambda (acc elt) (cons elt acc)) '() '#(1 2 3))
       '(1 2 3))
(check fold-right-order
       (vector-fold-right (lambda (acc elt) (string-append acc elt)) "" '#("a" "b" "c"))
       "cba")
(check fold-right-empty
       (vector-fold-right (lambda (acc elt) (+ acc elt)) 99 '#())
       99)
;; Parallel folds stop at the shortest vector.
(check fold-two-vectors
       (vector-fold (lambda (acc a b) (cons (list a b) acc)) '()
                    '#(1 2 3) '#(a b))
       '((2 b) (1 a)))
(check fold-right-two-vectors
       (vector-fold-right (lambda (acc a b) (cons (list a b) acc)) '()
                          '#(1 2 3) '#(a b))
       '((1 a) (2 b)))

;; ── iteration: map!, count, cumulate ─────────────────────────────────
(check map!-mutates-first
       (let ((v (vector 1 2 3)))
         (vector-map! (lambda (x) (* x 10)) v)
         v)
       '#(10 20 30))
;; With several vectors the first is the one written, and the length of
;; the shortest is how far it gets -- so slot 2 keeps its old value.
(check map!-writes-first-of-many
       (let ((v (vector 1 2 3)))
         (vector-map! + v '#(10 20))
         v)
       '#(11 22 3))
(check count-one (vector-count even? '#(1 2 3 4 5)) 2)
(check count-none (vector-count even? '#(1 3)) 0)
(check count-empty (vector-count even? '#()) 0)
(check count-two-vectors (vector-count < '#(1 2 3) '#(2 1 4)) 2)
;; Element i is (f result[i-1] vec[i]), with knil for result[-1].
(check cumulate (vector-cumulate + 0 '#(1 2 3 4)) '#(1 3 6 10))
(check cumulate-knil-is-used (vector-cumulate + 100 '#(1 2)) '#(101 103))
(check cumulate-empty (vector-cumulate + 0 '#()) '#())
(check cumulate-is-not-commutative
       (vector-cumulate (lambda (acc elt) (cons elt acc)) '() '#(1 2))
       '#((1) (2 1)))

;; ── searching: index and skip ────────────────────────────────────────
(check index (vector-index even? '#(1 3 4 5 6)) 2)
(check index-none (vector-index even? '#(1 3)) #f)
(check index-empty (vector-index even? '#()) #f)
(check index-right (vector-index-right even? '#(1 3 4 5 6)) 4)
(check index-right-none (vector-index-right even? '#(1 3)) #f)
(check index-right-empty (vector-index-right even? '#()) #f)
(check skip (vector-skip even? '#(2 4 5 6)) 2)
(check skip-none (vector-skip even? '#(2 4)) #f)
(check skip-right (vector-skip even? '#(2 4 5 6)) 2)
(check skip-right-last (vector-skip-right even? '#(1 2 4 6)) 0)
(check skip-right-none (vector-skip-right even? '#(2 4)) #f)
(check index-two-vectors (vector-index < '#(3 2 1) '#(1 2 3)) 2)
(check skip-two-vectors (vector-skip < '#(1 2 3) '#(2 3 1)) 2)

;; ── searching: binary-search takes the ELEMENT FIRST ─────────────────
;;
;; `(cmp element value)`. With the arguments the other way round the
;; comparison's sign is inverted, the search walks away from the target,
;; and these would be #f.
(check binary-search-middle
       (vector-binary-search '#(1 3 5 7 9) 5 (lambda (a b) (- a b)))
       2)
(check binary-search-first
       (vector-binary-search '#(1 3 5 7 9) 1 (lambda (a b) (- a b)))
       0)
(check binary-search-last
       (vector-binary-search '#(1 3 5 7 9) 9 (lambda (a b) (- a b)))
       4)
(check binary-search-absent
       (vector-binary-search '#(1 3 5 7 9) 4 (lambda (a b) (- a b)))
       #f)
(check binary-search-empty
       (vector-binary-search '#() 1 (lambda (a b) (- a b)))
       #f)
(check binary-search-below-all
       (vector-binary-search '#(5 6 7) 1 (lambda (a b) (- a b)))
       #f)
(check binary-search-above-all
       (vector-binary-search '#(5 6 7) 99 (lambda (a b) (- a b)))
       #f)
(check binary-search-range
       (vector-binary-search '#(9 1 3 5 7) 5 (lambda (a b) (- a b)) 1 5)
       3)

;; ── searching: any, every, partition ─────────────────────────────────
;;
;; `vector-any` returns what the predicate returned, not #t.
(check any-returns-the-value
       (vector-any (lambda (x) (and (even? x) (* x 10))) '#(1 3 4 5))
       40)
(check any-none (vector-any even? '#(1 3)) #f)
;; Empty: #f for any, #t for every. The prose says neither; both come
;; from the reference implementation.
(check any-empty (vector-any even? '#()) #f)
(check every-empty (vector-every even? '#()) #t)
;; `vector-every` returns the LAST value the predicate returned.
(check every-returns-last-value
       (vector-every (lambda (x) (* x 2)) '#(1 2 3))
       6)
(check every-false (vector-every even? '#(2 3 4)) #f)
(check every-two-vectors (vector-every < '#(1 2) '#(2 3)) #t)
(check any-two-vectors (vector-any < '#(5 2) '#(1 3)) #t)
;; `any` stops at the first true value: a counting predicate proves it
;; never reaches the rest.
(check any-stops-early
       (let ((n 0))
         (vector-any (lambda (x) (set! n (+ n 1)) (even? x)) '#(1 2 3 4))
         n)
       2)
(check every-stops-early
       (let ((n 0))
         (vector-every (lambda (x) (set! n (+ n 1)) (even? x)) '#(2 3 4))
         n)
       2)
;; Two values: the partitioned vector and the number that satisfied.
(check partition-vector
       (call-with-values
         (lambda () (vector-partition even? '#(1 2 3 4 5 6)))
         (lambda (v n) v))
       '#(2 4 6 1 3 5))
(check partition-count
       (call-with-values
         (lambda () (vector-partition even? '#(1 2 3 4 5 6)))
         (lambda (v n) n))
       3)
(check partition-all-satisfy
       (call-with-values
         (lambda () (vector-partition even? '#(2 4)))
         (lambda (v n) (list v n)))
       '(#(2 4) 2))
(check partition-none-satisfy
       (call-with-values
         (lambda () (vector-partition even? '#(1 3)))
         (lambda (v n) (list v n)))
       '(#(1 3) 0))
(check partition-empty
       (call-with-values
         (lambda () (vector-partition even? '#()))
         (lambda (v n) (list v n)))
       '(#() 0))
;; The count is also the index of the first element that does not
;; satisfy, which is what makes the second value useful.
(check partition-count-is-the-split-index
       (call-with-values
         (lambda () (vector-partition even? '#(1 2 3 4)))
         (lambda (v n) (vector-ref v n)))
       1)
;; `pred?` is applied exactly once per element, so a predicate with a
;; side effect is not surprised.
(check partition-applies-pred-once
       (let ((n 0))
         (vector-partition (lambda (x) (set! n (+ n 1)) (even? x)) '#(1 2 3 4))
         n)
       4)

;; ── mutators ─────────────────────────────────────────────────────────
(check swap!
       (let ((v (vector 1 2 3))) (vector-swap! v 0 2) v)
       '#(3 2 1))
(check swap!-same-index
       (let ((v (vector 1 2 3))) (vector-swap! v 1 1) v)
       '#(1 2 3))
(check reverse!
       (let ((v (vector 1 2 3 4))) (vector-reverse! v) v)
       '#(4 3 2 1))
(check reverse!-odd-length
       (let ((v (vector 1 2 3))) (vector-reverse! v) v)
       '#(3 2 1))
(check reverse!-range
       (let ((v (vector 1 2 3 4 5))) (vector-reverse! v 1 4) v)
       '#(1 4 3 2 5))
(check reverse!-empty-range
       (let ((v (vector 1 2 3))) (vector-reverse! v 1 1) v)
       '#(1 2 3))
(check reverse-copy!
       (let ((v (vector 0 0 0 0)))
         (vector-reverse-copy! v 0 '#(1 2 3 4))
         v)
       '#(4 3 2 1))
(check reverse-copy!-at
       (let ((v (vector 9 9 9 9 9)))
         (vector-reverse-copy! v 1 '#(1 2 3))
         v)
       '#(9 3 2 1 9))
(check reverse-copy!-range
       (let ((v (vector 0 0)))
         (vector-reverse-copy! v 0 '#(1 2 3 4) 1 3)
         v)
       '#(3 2))
;; Source and destination may be the same vector with overlapping
;; ranges, which is how a slice is reversed in place.
(check reverse-copy!-overlapping
       (let ((v (vector 1 2 3 4 5)))
         (vector-reverse-copy! v 0 v 0 5)
         v)
       '#(5 4 3 2 1))
(check reverse-copy!-overlapping-shifted
       (let ((v (vector 1 2 3 4 5)))
         (vector-reverse-copy! v 0 v 2 5)
         v)
       '#(5 4 3 4 5))
;; The in-place unfolds write into a range of an existing vector.
(check unfold!
       (let ((v (vector 0 0 0 0 0)))
         (vector-unfold! (lambda (i x) (values x (+ x 1))) v 1 4 10)
         v)
       '#(0 10 11 12 0))
(check unfold!-no-seed
       (let ((v (vector 9 9 9)))
         (vector-unfold! (lambda (i) i) v 0 3)
         v)
       '#(0 1 2))
(check unfold-right!
       (let ((v (vector 0 0 0 0 0)))
         (vector-unfold-right! (lambda (i x) (values x (+ x 1))) v 1 4 10)
         v)
       '#(0 12 11 10 0))
(check unfold!-empty-range
       (let ((v (vector 1 2 3)))
         (vector-unfold! (lambda (i) 99) v 1 1)
         v)
       '#(1 2 3))

;; ── conversion ───────────────────────────────────────────────────────
(check reverse-vector->list (reverse-vector->list '#(1 2 3)) '(3 2 1))
(check reverse-vector->list-range
       (reverse-vector->list '#(1 2 3 4 5) 1 4)
       '(4 3 2))
(check reverse-vector->list-empty (reverse-vector->list '#()) '())
(check reverse-list->vector (reverse-list->vector '(1 2 3)) '#(3 2 1))
(check reverse-list->vector-empty (reverse-list->vector '()) '#())
;; They are inverses.
(check reverse-round-trip
       (reverse-list->vector (reverse-vector->list '#(1 2 3 4)))
       '#(1 2 3 4))

;; ── the sixteen re-exported R7RS procedures ──────────────────────────
;;
;; These are the prelude's, exported by the library rather than rewritten
;; in it, so what is checked here is that importing `(srfi 133)` really
;; supplies them -- with the optional arguments this SRFI specifies.
(check base-make-vector (make-vector 3 'x) '#(x x x))
(check base-vector (vector 1 2) '#(1 2))
(check base-vector? (list (vector? '#(1)) (vector? 'x)) '(#t #f))
(check base-vector-ref (vector-ref '#(1 2 3) 1) 2)
(check base-vector-length (vector-length '#(1 2 3)) 3)
(check base-vector-set!
       (let ((v (vector 1 2))) (vector-set! v 0 'a) v)
       '#(a 2))
(check base-vector-copy (vector-copy '#(1 2 3)) '#(1 2 3))
(check base-vector-copy-range (vector-copy '#(1 2 3 4) 1 3) '#(2 3))
(check base-vector-append (vector-append '#(1) '#(2 3)) '#(1 2 3))
(check base-vector-fill!
       (let ((v (vector 1 2 3))) (vector-fill! v 'x) v)
       '#(x x x))
(check base-vector-fill!-range
       (let ((v (vector 1 2 3 4))) (vector-fill! v 'x 1 3) v)
       '#(1 x x 4))
(check base-vector-copy!
       (let ((v (vector 0 0 0))) (vector-copy! v 0 '#(1 2 3)) v)
       '#(1 2 3))
(check base-vector-copy!-range
       (let ((v (vector 0 0 0 0))) (vector-copy! v 1 '#(1 2 3 4) 2 4) v)
       '#(0 3 4 0))
(check base-vector->list (vector->list '#(1 2 3)) '(1 2 3))
(check base-vector->list-range (vector->list '#(1 2 3 4) 1 3) '(2 3))
(check base-list->vector (list->vector '(1 2 3)) '#(1 2 3))
(check base-string->vector (string->vector "abc") '#(#\a #\b #\c))
(check base-vector->string (vector->string '#(#\a #\b)) "ab")
;; `vector-map` and `vector-for-each` take any number of vectors and stop
;; at the shortest, which is what this SRFI asks of them.
(check base-vector-map (vector-map (lambda (x) (* x x)) '#(1 2 3)) '#(1 4 9))
(check base-vector-map-shortest (vector-map + '#(1 2 3) '#(10 20)) '#(11 22))
(check base-vector-for-each
       (let ((acc '()))
         (vector-for-each (lambda (x) (set! acc (cons x acc))) '#(1 2 3))
         acc)
       '(3 2 1))
(check base-vector-for-each-shortest
       (let ((acc '()))
         (vector-for-each (lambda (a b) (set! acc (cons (list a b) acc)))
                          '#(1 2 3) '#(a b))
         acc)
       '((2 b) (1 a)))

(test-failures)
