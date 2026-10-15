;;; (srfi 69) hash tables and (srfi 128) comparators.
(import (scm test) (srfi 69) (srfi 128))

;; ── (srfi 69) ────────────────────────────────────────────────────────
(check table? (hash-table? (make-hash-table)) #t)
(check table?-no (hash-table? (vector)) #f)
(check empty-size (hash-table-size (make-hash-table)) 0)
(check set-and-ref
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (hash-table-ref t 'a))
       1)
(check overwrite
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (hash-table-set! t 'a 2)
         (list (hash-table-ref t 'a) (hash-table-size t)))
       '(2 1))
;; The default equality is `equal?`, so a structural key works -- which is
;; the whole contract `equal-hash` is written to satisfy.
(check structural-key
       (let ((t (make-hash-table)))
         (hash-table-set! t (list 1 (vector 2 "three")) 'found)
         (hash-table-ref/default t (list 1 (vector 2 "three")) 'missing))
       'found)
(check string-key
       (let ((t (make-hash-table)))
         (hash-table-set! t "key" 1)
         (hash-table-ref/default t (string-append "k" "ey") 0))
       1)
(check ref-missing-raises
       (hash-table-ref (make-hash-table) 'nope)
       '(raised "hash-table-ref: no such key"))
(check ref-failure-thunk
       (hash-table-ref (make-hash-table) 'nope (lambda () 'default))
       'default)
(check ref-success-proc
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 5)
         (hash-table-ref t 'a (lambda () 'none) (lambda (v) (* v 2))))
       10)
(check ref/default (hash-table-ref/default (make-hash-table) 'x 'd) 'd)
(check exists?
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (list (hash-table-exists? t 'a) (hash-table-exists? t 'b)))
       '(#t #f))
(check delete
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (hash-table-set! t 'b 2)
         (hash-table-delete! t 'a)
         (list (hash-table-exists? t 'a) (hash-table-exists? t 'b)
               (hash-table-size t)))
       '(#f #t 1))
(check delete-missing-is-quiet
       (let ((t (make-hash-table)))
         (hash-table-delete! t 'nope)
         (hash-table-size t))
       0)
(check update
       (let ((t (make-hash-table)))
         (hash-table-set! t 'n 1)
         (hash-table-update! t 'n (lambda (v) (+ v 10)))
         (hash-table-ref t 'n))
       11)
(check update-missing-raises
       (hash-table-update! (make-hash-table) 'n (lambda (v) v))
       '(raised "hash-table-update!: no such key"))
(check update-default
       (let ((t (make-hash-table)))
         (hash-table-update!/default t 'n (lambda (v) (+ v 1)) 100)
         (hash-table-update!/default t 'n (lambda (v) (+ v 1)) 100)
         (hash-table-ref t 'n))
       102)

;; a custom equality, which is the reason this table is written in Scheme
(check custom-equality
       (let ((t (make-hash-table string-ci=? string-ci-hash)))
         (hash-table-set! t "Key" 1)
         (list (hash-table-ref/default t "KEY" 0)
               (hash-table-ref/default t "key" 0)))
       '(1 1))
(check eq-table
       (let ((t (make-hash-table eq?)))
         (hash-table-set! t 'a 1)
         ;; Two equal strings are not `eq?`, so they are two keys.
         (hash-table-set! t (string-copy "s") 2)
         (hash-table-set! t (string-copy "s") 3)
         (list (hash-table-ref t 'a) (hash-table-size t)))
       '(1 3))
(check equivalence-accessor
       (eq? (hash-table-equivalence-function (make-hash-table eq?)) eq?)
       #t)
(check hash-accessor
       (procedure? (hash-table-hash-function (make-hash-table)))
       #t)

;; iteration and conversion
(check keys-and-values
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (hash-table-set! t 'b 2)
         (list (length (hash-table-keys t)) (length (hash-table-values t))
               (apply + (hash-table-values t))))
       '(2 2 3))
(check walk
       (let ((t (make-hash-table)) (n 0))
         (hash-table-set! t 'a 1)
         (hash-table-set! t 'b 2)
         (hash-table-walk t (lambda (k v) (set! n (+ n v))))
         n)
       3)
(check fold
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (hash-table-set! t 'b 2)
         (hash-table-fold t (lambda (k v acc) (+ v acc)) 0))
       3)
(check ->alist
       (let ((t (make-hash-table)))
         (hash-table-set! t 'a 1)
         (length (hash-table->alist t)))
       1)
(check alist->
       (let ((t (alist->hash-table '((a . 1) (b . 2)))))
         (list (hash-table-ref t 'a) (hash-table-ref t 'b) (hash-table-size t)))
       '(1 2 2))
;; An earlier entry wins, as an alist's own lookup would.
(check alist->-duplicates
       (hash-table-ref (alist->hash-table '((a . 1) (a . 2))) 'a)
       1)
(check copy-is-independent
       (let* ((t (make-hash-table)) (c (begin (hash-table-set! t 'a 1)
                                              (hash-table-copy t))))
         (hash-table-set! c 'a 99)
         (list (hash-table-ref t 'a) (hash-table-ref c 'a)))
       '(1 99))
(check merge
       (let ((a (make-hash-table)) (b (make-hash-table)))
         (hash-table-set! a 'x 1)
         (hash-table-set! b 'x 99)
         (hash-table-set! b 'y 2)
         (hash-table-merge! a b)
         (list (hash-table-ref a 'x) (hash-table-ref a 'y)))
       '(1 2))

;; growth: more entries than the initial bucket count, so it has rehashed
(check growth
       (let ((t (make-hash-table)))
         (let loop ((i 0))
           (when (< i 500)
             (hash-table-set! t i (* i i))
             (loop (+ i 1))))
         (list (hash-table-size t)
               (hash-table-ref t 499)
               (hash-table-ref/default t 500 'absent)
               (length (hash-table-keys t))))
       '(500 249001 absent 500))
(check growth-then-delete
       (let ((t (make-hash-table)))
         (let loop ((i 0))
           (when (< i 100) (hash-table-set! t i i) (loop (+ i 1))))
         (let loop ((i 0))
           (when (< i 50) (hash-table-delete! t i) (loop (+ i 1))))
         (list (hash-table-size t) (hash-table-exists? t 0)
               (hash-table-exists? t 99)))
       '(50 #f #t))

;; the SRFI's own hash procedures
(check hash-agrees (= (hash '(1 2)) (hash '(1 2))) #t)
(check hash-bounded (< (hash "x" 8) 8) #t)
(check string-hash-agrees (= (string-hash "ab") (string-hash "ab")) #t)
(check string-ci-hash-folds (= (string-ci-hash "AB") (string-ci-hash "ab")) #t)
(check hash-by-identity-is-stable
       (let ((s (string-copy "s"))) (= (hash-by-identity s) (hash-by-identity s)))
       #t)

;; ── (srfi 128) ───────────────────────────────────────────────────────
(check comparator? (comparator? (make-default-comparator)) #t)
(check default-ordered (comparator-ordered? (make-default-comparator)) #t)
(check default-hashable (comparator-hashable? (make-default-comparator)) #t)
;; The eq/eqv comparators are deliberately unordered: the default total
;; order agrees with `equal?`, not with identity.
(check eq-unordered (comparator-ordered? (make-eq-comparator)) #f)
(check eq-hashable (comparator-hashable? (make-eqv-comparator)) #t)
(check equal-ordered (comparator-ordered? (make-equal-comparator)) #t)
(check type-test (comparator-test-type (make-default-comparator) 'anything) #t)
(check check-type (comparator-check-type (make-default-comparator) 1) #t)

(check =?-chain (=? (make-default-comparator) 1 1 1) #t)
(check =?-no (=? (make-default-comparator) 1 2) #f)
(check <?-chain (<? (make-default-comparator) 1 2 3) #t)
(check <?-unordered-chain (<? (make-default-comparator) 1 3 2) #f)
(check >? (>? (make-default-comparator) 3 2 1) #t)
(check <=? (<=? (make-default-comparator) 1 1 2) #t)
(check >=? (>=? (make-default-comparator) 2 2 1) #t)

;; the default order across types: () < pair < boolean < char < string <
;; symbol < number < vector < bytevector
(check order-across-types
       (let ((c (make-default-comparator)))
         (list (<? c '() '(1)) (<? c '(1) #t) (<? c #t #\a) (<? c #\a "s")
               (<? c "s" 'sym) (<? c 'sym 1) (<? c 1 (vector)) 
               (<? c (vector) (bytevector))))
       '(#t #t #t #t #t #t #t #t))
(check order-within-strings (<? (make-default-comparator) "abc" "abd") #t)
(check order-within-symbols (<? (make-default-comparator) 'aa 'ab) #t)
(check order-within-numbers (<? (make-default-comparator) 1 1.5) #t)
(check order-booleans (<? (make-default-comparator) #f #t) #t)
(check order-lists-lexicographic
       (let ((c (make-default-comparator)))
         (list (<? c '(1 2) '(1 3)) (<? c '(1) '(1 2)) (<? c '(2) '(1 9))))
       '(#t #t #f))
(check order-vectors-by-length-then-element
       (let ((c (make-default-comparator)))
         (list (<? c (vector 1) (vector 1 2)) (<? c (vector 1 2) (vector 1 3))))
       '(#t #t))

;; the hash functions, each taking the optional bound
(check boolean-hash (= (boolean-hash #t) (boolean-hash #t)) #t)
(check boolean-hash-differs (= (boolean-hash #t) (boolean-hash #f)) #f)
(check char-hash (= (char-hash #\a) (char-hash #\a)) #t)
(check char-ci-hash (= (char-ci-hash #\A) (char-ci-hash #\a)) #t)
(check number-hash (= (number-hash 42) (number-hash 42)) #t)
(check symbol-hash (= (symbol-hash 'x) (symbol-hash 'x)) #t)
(check default-hash-agrees (= (default-hash '(1 2)) (default-hash '(1 2))) #t)
(check hash-bounded-128 (< (default-hash "s" 4) 4) #t)
(check hash-bound-is-positive (> (hash-bound) 0) #t)
(check hash-salt-is-a-number (number? (hash-salt)) #t)

;; comparators built from comparators
(check pair-comparator
       (let* ((c (make-default-comparator)) (pc (make-pair-comparator c c)))
         (list (=? pc (cons 1 2) (cons 1 2))
               (=? pc (cons 1 2) (cons 1 3))
               (<? pc (cons 1 2) (cons 1 3))
               (<? pc (cons 1 9) (cons 2 0))
               (comparator-test-type pc (cons 1 2))
               (comparator-test-type pc 5)))
       '(#t #f #t #t #t #f))
(check pair-comparator-hashes
       (let* ((c (make-default-comparator)) (pc (make-pair-comparator c c)))
         (= (comparator-hash pc (cons 1 2)) (comparator-hash pc (cons 1 2))))
       #t)
(check list-comparator
       (let* ((c (make-default-comparator))
              (lc (make-list-comparator c list? null? car cdr)))
         (list (=? lc '(1 2) '(1 2)) (=? lc '(1 2) '(1 3))
               (<? lc '(1) '(1 2)) (<? lc '(1 2) '(1 3))
               (= (comparator-hash lc '(1 2)) (comparator-hash lc '(1 2)))))
       '(#t #f #t #t #t))
(check vector-comparator
       (let* ((c (make-default-comparator))
              (vc (make-vector-comparator c vector? vector-length vector-ref)))
         (list (=? vc (vector 1 2) (vector 1 2))
               (<? vc (vector 1) (vector 1 2))
               (<? vc (vector 1 2) (vector 1 3))
               (= (comparator-hash vc (vector 1 2))
                  (comparator-hash vc (vector 1 2)))))
       '(#t #t #t #t))

;; a comparator whose equality is derived from its ordering
(check derived-equality
       (let ((c (make-comparator #t #t < #f)))
         (list (=? c 1 1) (=? c 1 2) (comparator-hashable? c)))
       '(#t #f #f))

(check if<=> (comparator-if<=> 1 2 'less 'equal 'greater) 'less)
(check if<=>-equal (comparator-if<=> 2 2 'less 'equal 'greater) 'equal)
(check if<=>-greater (comparator-if<=> 3 2 'less 'equal 'greater) 'greater)
(check if<=>-explicit
       (comparator-if<=> (make-default-comparator) "a" "b" 'less 'equal 'greater)
       'less)

;; A comparator-keyed table: the pair that SRFI 125 is built on, usable
;; already through SRFI 69.
(check comparator-keyed-table
       (let* ((c (make-default-comparator))
              (t (make-hash-table (comparator-equality-predicate c)
                                  (comparator-hash-function c))))
         (hash-table-set! t (list 1 2) 'v)
         (hash-table-ref/default t (list 1 2) 'missing))
       'v)

(test-failures)
