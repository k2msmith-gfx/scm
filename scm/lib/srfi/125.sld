;;; (srfi 125) — Intermediate Hash Tables.
;;;
;;; The larger hash-table interface: keyed on a `(srfi 128)` comparator
;;; rather than a loose pair of procedures, with the whole-table
;;; operations (fold, map, prune, the four set operations) that `(srfi 69)`
;;; leaves out.
;;;
;;; The table itself is `(scm hash-table)`, shared with `(srfi 69)`, so a
;;; table made through either interface works through the other -- which
;;; is what this SRFI's compatibility clause with 69 requires.
;;;
;;; Two defaults here surprise people, so they are worth stating: a table
;;; from `hash-table` or from `hash-table-copy` with no second argument is
;;; IMMUTABLE, and mutating it is an error. `make-hash-table`,
;;; `alist->hash-table`, `hash-table-unfold` and `hash-table-empty-copy`
;;; all give mutable tables. This follows the SRFI, whose flag is named
;;; `mutable` because it is an opt-in.
(define-library (srfi 125)
  (export make-hash-table hash-table hash-table-unfold alist->hash-table
          hash-table? hash-table-contains? hash-table-exists?
          hash-table-empty? hash-table=? hash-table-mutable?
          hash-table-ref hash-table-ref/default
          hash-table-set! hash-table-delete! hash-table-intern!
          hash-table-update! hash-table-update!/default
          hash-table-pop! hash-table-clear!
          hash-table-size hash-table-keys hash-table-values
          hash-table-entries hash-table-find hash-table-count
          hash-table-map hash-table-for-each hash-table-walk
          hash-table-map! hash-table-map->list hash-table-fold
          hash-table-prune!
          hash-table-copy hash-table-empty-copy hash-table->alist
          hash-table-union! hash-table-merge!
          hash-table-intersection! hash-table-difference! hash-table-xor!
          hash hash-by-identity
          hash-table-equivalence-function hash-table-hash-function)
  (import (scheme base) (scm hash-table) (srfi 128))
  (begin

    (define hash equal-hash)
    (define hash-by-identity eqv-hash)

    (define hash-table? table?)
    (define hash-table-size table-count)
    (define hash-table-ref/default table-ref/default)
    (define hash-table-mutable? table-mutable?)
    (define hash-table-equivalence-function table-equiv)
    (define hash-table-hash-function table-hashf)
    (define hash-table-clear! table-clear!)

    (define (hash-table-contains? table key)
      (if (table-entry table key) #t #f))

    (define hash-table-exists? hash-table-contains?)

    (define (hash-table-empty? table) (= 0 (table-count table)))

    ;; A comparator's hash function answers over the whole integer range;
    ;; the table wants a bucket index, so it is reduced here. Everything
    ;; below builds its tables through this one place.
    (define (%bucket-hash h) (lambda (key n) (modulo (h key) n)))

    ;; `(make-hash-table comparator-or-equality arg ...)`. The trailing
    ;; args are advice: an integer is a starting size, a procedure after
    ;; an equality predicate is its hash function, and weakness arguments
    ;; are accepted and ignored -- this GC holds no weak references, and
    ;; silently keeping entries alive is the safe direction.
    (define (make-hash-table first . args)
      (if (comparator? first)
          (%make-mutable (comparator-equality-predicate first)
                         (%bucket-hash (comparator-hash-function first))
                         first args)
          (let* ((hashf (if (and (pair? args) (procedure? (car args)))
                            (%bucket-hash (car args))
                            (hash-for-equality first)))
                 (args (if (and (pair? args) (procedure? (car args)))
                           (cdr args) args)))
            (%make-mutable first hashf #f args))))

    (define (%make-mutable equiv hashf cmp args)
      (make-table equiv hashf cmp (%size args) #t))

    (define (%size args)
      (cond ((null? args) 8)
            ((and (integer? (car args)) (exact? (car args))) (car args))
            (else (%size (cdr args)))))

    ;; A table with the same equality, hash and comparator as `t`.
    (define (%like t mutable)
      (make-table (table-equiv t) (table-hashf t) (table-comparator t) 8 mutable))

    ;; `(hash-table comparator key value ...)` -- immutable, per the SRFI.
    (define (hash-table comparator . kvs)
      (let ((t (make-hash-table comparator)))
        (%set-pairs! t kvs 'hash-table)
        (%freeze t)))

    ;; The table is built mutable and then copied into an immutable one,
    ;; since the mutability flag is fixed when the record is made.
    (define (%freeze t) (table-copy t #f))

    (define (%set-pairs! t kvs who)
      (let loop ((kvs kvs))
        (cond ((null? kvs) (if #f #f))
              ((null? (cdr kvs)) (error "odd number of key/value arguments" who))
              (else (table-set! t (car kvs) (cadr kvs))
                    (loop (cddr kvs))))))

    (define (hash-table-unfold stop? mapper successor seed comparator . args)
      (let ((t (apply make-hash-table comparator args)))
        (let loop ((seed seed))
          (if (stop? seed)
              t
              (call-with-values (lambda () (mapper seed))
                (lambda (key value)
                  (table-set! t key value)
                  (loop (successor seed))))))))

    (define (alist->hash-table alist first . args)
      (let ((t (apply make-hash-table first args)))
        ;; Earlier entries win, as a lookup down the alist would give.
        (let loop ((as (reverse alist)))
          (unless (null? as)
            (table-set! t (caar as) (cdar as))
            (loop (cdr as))))
        t))

    ;; `(hash-table-ref table key [failure [success]])`
    (define (hash-table-ref table key . opt)
      (let ((e (table-entry table key)))
        (cond (e (if (and (pair? opt) (pair? (cdr opt)))
                     ((cadr opt) (cdr e))
                     (cdr e)))
              ((pair? opt) ((car opt)))
              (else (error "hash-table-ref: no such key" key)))))

    ;; Alternating keys and values, applied left to right, so a repeated
    ;; key ends with its last value.
    (define (hash-table-set! table . kvs)
      (%set-pairs! table kvs 'hash-table-set!))

    ;; Returns how many of the given keys were actually present.
    (define (hash-table-delete! table . keys)
      (let loop ((ks keys) (n 0))
        (if (null? ks)
            n
            (loop (cdr ks) (if (table-delete! table (car ks)) (+ n 1) n)))))

    (define (hash-table-intern! table key failure)
      (let ((e (table-entry table key)))
        (if e
            (cdr e)
            (let ((v (failure)))
              (table-set! table key v)
              v))))

    ;; `(hash-table-update! table key updater [failure [success]])`
    (define (hash-table-update! table key updater . opt)
      (let ((e (table-entry table key))
            (failure (if (pair? opt) (car opt) #f))
            (success (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) #f)))
        (cond (e (table-check-mutable table 'update!)
                 (set-cdr! e (updater (if success (success (cdr e)) (cdr e)))))
              (failure (table-set! table key (updater (failure))))
              (else (error "hash-table-update!: no such key" key)))))

    (define (hash-table-update!/default table key updater default)
      (hash-table-update! table key updater (lambda () default)))

    ;; Removes and returns some entry, as two values. Which entry is not
    ;; specified, so this takes the first the buckets offer.
    (define (hash-table-pop! table)
      (table-check-mutable table 'pop!)
      (let ((e (table-fold table (lambda (k v acc) (or acc (cons k v))) #f)))
        (if (not e)
            (error "hash-table-pop!: the table is empty")
            (begin (table-delete! table (car e))
                   (values (car e) (cdr e))))))

    (define (hash-table-keys table)
      (table-fold table (lambda (k v acc) (cons k acc)) '()))

    (define (hash-table-values table)
      (table-fold table (lambda (k v acc) (cons v acc)) '()))

    ;; Keys and values as two lists in matching order.
    (define (hash-table-entries table)
      (let ((kvs (table-fold table (lambda (k v acc) (cons (cons k v) acc)) '())))
        (values (map car kvs) (map cdr kvs))))

    (define (hash-table->alist table)
      (table-fold table (lambda (k v acc) (cons (cons k v) acc)) '()))

    ;; The first true result of `(proc key value)`, else `(failure)`.
    ;; The fold cannot stop early, so the result is carried once found.
    (define (hash-table-find proc table failure)
      (let ((r (table-fold table
                           (lambda (k v acc) (or acc (proc k v)))
                           #f)))
        (or r (failure))))

    (define (hash-table-count pred table)
      (table-fold table (lambda (k v n) (if (pred k v) (+ n 1) n)) 0))

    (define (hash-table-for-each proc table)
      (table-fold table (lambda (k v acc) (proc k v) acc) #f)
      (if #f #f))

    ;; Deprecated, and it keeps `(srfi 69)`'s argument order -- the table
    ;; first -- which is the whole reason it is a separate name.
    (define (hash-table-walk table proc) (hash-table-for-each proc table))

    (define (hash-table-map->list proc table)
      (table-fold table (lambda (k v acc) (cons (proc k v) acc)) '()))

    ;; `(hash-table-map proc comparator table)` -- same keys, each value
    ;; replaced by `(proc value)`, in a new table under `comparator`.
    (define (hash-table-map proc comparator table)
      (let ((t (make-hash-table comparator)))
        (table-fold table (lambda (k v acc) (table-set! t k (proc v)) acc) #f)
        t))

    (define (hash-table-map! proc table)
      (table-check-mutable table 'map!)
      (table-fold table
                  (lambda (k v acc)
                    (set-cdr! (table-entry table k) (proc k v))
                    acc)
                  #f)
      (if #f #f))

    ;; `(hash-table-fold table proc seed)`. When the first argument is a
    ;; procedure the arguments are read in SRFI 69's order instead, which
    ;; the SRFI asks for by name.
    (define (hash-table-fold a b c)
      (if (procedure? a)
          (table-fold c a b)
          (table-fold a b c)))

    (define (hash-table-prune! proc table)
      (table-check-mutable table 'prune!)
      (let ((doomed (table-fold table
                                (lambda (k v acc) (if (proc k v) (cons k acc) acc))
                                '())))
        ;; Collected first: deleting during the walk would mutate the
        ;; buckets the walk is standing on.
        (let loop ((ks doomed))
          (unless (null? ks)
            (table-delete! table (car ks))
            (loop (cdr ks))))
        (if #f #f)))

    ;; `(hash-table-copy table [mutable])` -- immutable unless asked.
    (define (hash-table-copy table . opt)
      (table-copy table (and (pair? opt) (car opt) #t)))

    (define (hash-table-empty-copy table) (%like table #t))

    ;; Same keys under ht1's equality, and values equal under the given
    ;; comparator.
    (define (hash-table=? value-comparator ht1 ht2)
      (let ((same? (comparator-equality-predicate value-comparator))
            (missing (list 'missing)))
        (and (= (table-count ht1) (table-count ht2))
             (table-fold ht1
                         (lambda (k v acc)
                           (and acc
                                (let ((other (table-ref/default ht2 k missing)))
                                  (and (not (eq? other missing))
                                       (same? v other)))))
                         #t))))

    ;; ht2's entries for keys ht1 lacks; ht1's own values are kept.
    (define (hash-table-union! ht1 ht2)
      (table-check-mutable ht1 'union!)
      (table-fold ht2
                  (lambda (k v acc)
                    (unless (table-entry ht1 k) (table-set! ht1 k v))
                    acc)
                  #f)
      ht1)

    (define hash-table-merge! hash-table-union!)

    (define (hash-table-intersection! ht1 ht2)
      (hash-table-prune! (lambda (k v) (not (table-entry ht2 k))) ht1)
      ht1)

    (define (hash-table-difference! ht1 ht2)
      (hash-table-prune! (lambda (k v) (if (table-entry ht2 k) #t #f)) ht1)
      ht1)

    ;; Keys in exactly one of the two: ht2's additions go in, then the
    ;; keys both had come out. The additions are gathered before anything
    ;; changes, so a key added from ht2 is not then pruned as "shared".
    (define (hash-table-xor! ht1 ht2)
      (table-check-mutable ht1 'xor!)
      (let ((adds (table-fold ht2
                              (lambda (k v acc)
                                (if (table-entry ht1 k) acc (cons (cons k v) acc)))
                              '()))
            (drops (table-fold ht2
                               (lambda (k v acc)
                                 (if (table-entry ht1 k) (cons k acc) acc))
                               '())))
        (let loop ((ks drops))
          (unless (null? ks) (table-delete! ht1 (car ks)) (loop (cdr ks))))
        (let loop ((as adds))
          (unless (null? as) (table-set! ht1 (caar as) (cdar as)) (loop (cdr as))))
        ht1))))
