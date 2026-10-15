;;; (srfi 125) intermediate hash tables.
(import (scm test) (srfi 125) (srfi 128) (scheme base))

(define (cmp) (make-default-comparator))
(import (srfi 132))
(define (sorted lst) (list-sort < lst))
(define (sorted-strings lst) (list-sort string<? lst))

;; ── constructors ─────────────────────────────────────────────────────
(check make (hash-table? (make-hash-table (cmp))) #t)
(check make-empty (hash-table-empty? (make-hash-table (cmp))) #t)
(check make-mutable (hash-table-mutable? (make-hash-table (cmp))) #t)
;; A size argument is advice, not a limit.
(check make-sized (hash-table-size (make-hash-table (cmp) 64)) 0)
;; Weakness arguments are accepted and ignored.
(check make-weak (hash-table-size (make-hash-table (cmp) 'weak-keys)) 0)
;; The deprecated form still takes a bare equality predicate.
(check make-from-predicate
       (let ((t (make-hash-table equal?)))
         (hash-table-set! t "a" 1)
         (hash-table-ref t (string-copy "a")))
       1)

(check literal (hash-table-size (hash-table (cmp) 'a 1 'b 2)) 2)
(check literal-immutable (hash-table-mutable? (hash-table (cmp) 'a 1)) #f)
(check-raises literal-odd (hash-table (cmp) 'a))

(check unfold
       (sorted (hash-table-keys
                (hash-table-unfold (lambda (i) (= i 5))
                                   (lambda (i) (values i (* i i)))
                                   (lambda (i) (+ i 1))
                                   0 (cmp))))
       '(0 1 2 3 4))
(check unfold-values
       (hash-table-ref (hash-table-unfold (lambda (i) (= i 5))
                                          (lambda (i) (values i (* i i)))
                                          (lambda (i) (+ i 1))
                                          0 (cmp))
                       4)
       16)

(check alist->table
       (sorted (hash-table-values (alist->hash-table '((a . 1) (b . 2)) (cmp))))
       '(1 2))
;; Earlier entries win, as a lookup down the alist would give.
(check alist->table-duplicate
       (hash-table-ref (alist->hash-table '((a . 1) (a . 2)) (cmp)) 'a)
       1)

;; ── predicates and access ────────────────────────────────────────────
(define (t3) (alist->hash-table '((a . 1) (b . 2) (c . 3)) (cmp)))

(check contains (hash-table-contains? (t3) 'b) #t)
(check contains-no (hash-table-contains? (t3) 'z) #f)
(check exists-alias (hash-table-exists? (t3) 'b) #t)
(check size (hash-table-size (t3)) 3)
(check ref (hash-table-ref (t3) 'c) 3)
(check ref-failure (hash-table-ref (t3) 'z (lambda () 'none)) 'none)
(check ref-success (hash-table-ref (t3) 'c (lambda () 'none) (lambda (v) (* v 10))) 30)
(check-raises ref-missing-is-an-error (hash-table-ref (t3) 'z))
(check ref/default (hash-table-ref/default (t3) 'z 'none) 'none)

;; ── mutation ─────────────────────────────────────────────────────────
;; Several pairs at once, applied left to right.
(check set-many
       (let ((t (make-hash-table (cmp))))
         (hash-table-set! t 'a 1 'b 2 'a 3)
         (list (hash-table-ref t 'a) (hash-table-ref t 'b)))
       '(3 2))
(check-raises set-odd (hash-table-set! (make-hash-table (cmp)) 'a))

;; `delete!` answers how many of the keys were there.
(check delete (let ((t (t3))) (list (hash-table-delete! t 'a 'z 'b)
                                    (hash-table-size t)))
       '(2 1))

(check intern-absent
       (let ((t (make-hash-table (cmp))))
         (list (hash-table-intern! t 'a (lambda () 1)) (hash-table-size t)))
       '(1 1))
(check intern-present
       (let ((t (t3)))
         (list (hash-table-intern! t 'a (lambda () 99)) (hash-table-size t)))
       '(1 3))

(check update
       (let ((t (t3))) (hash-table-update! t 'a (lambda (v) (+ v 10)))
            (hash-table-ref t 'a))
       11)
(check update-absent-failure
       (let ((t (t3))) (hash-table-update! t 'z (lambda (v) (+ v 1)) (lambda () 100))
            (hash-table-ref t 'z))
       101)
(check update-success
       (let ((t (t3)))
         (hash-table-update! t 'a (lambda (v) (+ v 1)) (lambda () 0) (lambda (v) (* v 100)))
         (hash-table-ref t 'a))
       101)
(check-raises update-absent-is-an-error
              (hash-table-update! (t3) 'z (lambda (v) v)))
(check update/default
       (let ((t (t3)))
         (hash-table-update!/default t 'z (lambda (v) (+ v 1)) 7)
         (hash-table-ref t 'z))
       8)

;; `pop!` takes some entry out and returns both halves.
(check pop
       (let ((t (t3)))
         (call-with-values (lambda () (hash-table-pop! t))
           (lambda (k v) (list (hash-table-contains? t k) (hash-table-size t)
                               (hash-table-contains? (t3) k)))))
       '(#f 2 #t))
(check-raises pop-empty (hash-table-pop! (make-hash-table (cmp))))

(check clear (let ((t (t3))) (hash-table-clear! t) (hash-table-size t)) 0)

;; ── the whole table ──────────────────────────────────────────────────
(check keys (sorted-strings (map symbol->string (hash-table-keys (t3))))
       '("a" "b" "c"))
(check values- (sorted (hash-table-values (t3))) '(1 2 3))
;; Keys and values come back in matching order, whatever that order is.
(check entries
       (call-with-values (lambda () (hash-table-entries (t3)))
         (lambda (ks vs)
           (equal? (sorted vs)
                   (sorted (map (lambda (k) (hash-table-ref (t3) k)) ks)))))
       #t)
(check alist (sorted (map cdr (hash-table->alist (t3)))) '(1 2 3))

(check find (hash-table-find (lambda (k v) (and (= v 2) 'found)) (t3) (lambda () 'no))
       'found)
(check find-failure (hash-table-find (lambda (k v) (and (= v 9) 'found)) (t3)
                                     (lambda () 'no))
       'no)
(check count (hash-table-count (lambda (k v) (odd? v)) (t3)) 2)

;; ── mapping and folding ──────────────────────────────────────────────
(check map- (sorted (hash-table-values (hash-table-map (lambda (v) (* v 2)) (cmp) (t3))))
       '(2 4 6))
(check map-keys-kept (hash-table-ref (hash-table-map - (cmp) (t3)) 'b) -2)
(check map! (let ((t (t3))) (hash-table-map! (lambda (k v) (* v 10)) t)
                 (sorted (hash-table-values t)))
       '(10 20 30))
(check map->list (sorted (hash-table-map->list (lambda (k v) v) (t3))) '(1 2 3))
(check for-each
       (let ((n 0))
         (hash-table-for-each (lambda (k v) (set! n (+ n v))) (t3))
         n)
       6)
(check walk-alias
       (let ((n 0)) (hash-table-walk (t3) (lambda (k v) (set! n (+ n v)))) n)
       6)
(check fold (hash-table-fold (t3) (lambda (k v acc) (+ v acc)) 0) 6)
;; (srfi 69)'s argument order is accepted too, which the SRFI asks for.
(check fold-srfi-69-order (hash-table-fold (lambda (k v acc) (+ v acc)) 0 (t3)) 6)
(check prune!
       (let ((t (t3))) (hash-table-prune! (lambda (k v) (odd? v)) t)
            (hash-table-values t))
       '(2))

;; ── copying ──────────────────────────────────────────────────────────
;; A copy with no flag is immutable, which is this SRFI's choice.
(check copy-default-immutable (hash-table-mutable? (hash-table-copy (t3))) #f)
(check copy-contents (sorted (hash-table-values (hash-table-copy (t3)))) '(1 2 3))
(check copy-mutable (hash-table-mutable? (hash-table-copy (t3) #t)) #t)
(check-raises copy-immutable-rejects-mutation
              (hash-table-set! (hash-table-copy (t3)) 'z 9))
(check empty-copy (list (hash-table-size (hash-table-empty-copy (t3)))
                        (hash-table-mutable? (hash-table-empty-copy (t3))))
       '(0 #t))
;; An empty copy keeps the original's comparator, so a structural key works.
(check empty-copy-keeps-equality
       (let ((t (hash-table-empty-copy (alist->hash-table '(("a" . 1)) (cmp)))))
         (hash-table-set! t (string-copy "k") 5)
         (hash-table-ref t (string-copy "k")))
       5)

;; ── comparison and the set operations ────────────────────────────────
(check table=? (hash-table=? (cmp) (t3) (t3)) #t)
(check table=?-size (hash-table=? (cmp) (t3) (alist->hash-table '((a . 1)) (cmp))) #f)
(check table=?-values
       (hash-table=? (cmp) (t3) (alist->hash-table '((a . 9) (b . 2) (c . 3)) (cmp)))
       #f)
(check table=?-keys
       (hash-table=? (cmp) (t3) (alist->hash-table '((a . 1) (b . 2) (z . 3)) (cmp)))
       #f)

;; Union keeps the receiver's own values.
(check union
       (let ((t (alist->hash-table '((a . 1)) (cmp))))
         (hash-table-union! t (alist->hash-table '((a . 9) (b . 2)) (cmp)))
         (sorted (hash-table-values t)))
       '(1 2))
(check merge-alias
       (let ((t (alist->hash-table '((a . 1)) (cmp))))
         (hash-table-merge! t (alist->hash-table '((b . 2)) (cmp)))
         (hash-table-size t))
       2)
(check intersection
       (let ((t (t3)))
         (hash-table-intersection! t (alist->hash-table '((a . 9) (z . 9)) (cmp)))
         (hash-table->alist t))
       '((a . 1)))
(check difference
       (let ((t (t3)))
         (hash-table-difference! t (alist->hash-table '((a . 9) (b . 9)) (cmp)))
         (hash-table->alist t))
       '((c . 3)))
;; Keys in exactly one of the two.
(check xor
       (let ((t (alist->hash-table '((a . 1) (b . 2)) (cmp))))
         (hash-table-xor! t (alist->hash-table '((b . 9) (c . 3)) (cmp)))
         (sorted (hash-table-map->list (lambda (k v) v) t)))
       '(1 3))
(check xor-returns-the-receiver
       (let ((t (alist->hash-table '((a . 1)) (cmp))))
         (eq? t (hash-table-xor! t (alist->hash-table '((b . 2)) (cmp)))))
       #t)

;; ── one table, two interfaces ────────────────────────────────────────
;; `(srfi 69)` and this library are two views of one data type, so a
;; table made by one must work through the other.
(check shared-type
       (let ((t (make-hash-table (cmp))))
         (hash-table-set! t 'a 1)
         (list (hash-table-contains? t 'a) (hash-table-size t)))
       '(#t 1))

;; ── growth ───────────────────────────────────────────────────────────
;; Past the load factor the table doubles and rehashes; every key must
;; still be found afterwards.
(check growth
       (let ((t (make-hash-table (cmp))))
         (let loop ((i 0))
           (when (< i 500) (hash-table-set! t i (* i i)) (loop (+ i 1))))
         (let loop ((i 0) (bad '()))
           (cond ((= i 500) (list (hash-table-size t) (reverse bad)))
                 ((= (hash-table-ref t i) (* i i)) (loop (+ i 1) bad))
                 (else (loop (+ i 1) (cons i bad))))))
       '(500 ()))

(test-failures)
