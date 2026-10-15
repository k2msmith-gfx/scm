;;; (srfi 69) — Basic Hash Tables.
;;;
;;; A table is a record holding a vector of buckets, each bucket an
;;; association list. A lookup hashes the key to a bucket and walks it with
;;; the table's own equality predicate; the table doubles when it has more
;;; entries than buckets, so the walk stays short and the cost is amortised
;;; constant.
;;;
;;; **Why Scheme and not a native table.** A native one would be perhaps
;;; five times faster, but SRFI 69 lets a caller supply *any* equality
;;; predicate, and a native table would have to re-enter the interpreter
;;; for each probe -- so the fast path would be fast only for the four
;;; standard equivalences, and the general path slower than this. The
;;; hash functions themselves *are* native, which is where the real work
;;; is: `equal-hash` walks structure in Rust. If profiling ever asks for
;;; more, a native table for the standard equivalences is the upgrade, and
;;; nothing in this interface would change.
(define-library (srfi 69)
  (export make-hash-table hash-table? hash-table-equivalence-function
          hash-table-hash-function
          hash-table-ref hash-table-ref/default hash-table-set!
          hash-table-delete! hash-table-exists? hash-table-update!
          hash-table-update!/default hash-table-size
          hash-table-keys hash-table-values hash-table-walk
          hash-table-fold hash-table->alist hash-table-copy
          hash-table-merge! alist->hash-table
          hash string-hash string-ci-hash hash-by-identity)
  (import (scheme base))
  (begin

    (define hash equal-hash)
    (define hash-by-identity eqv-hash)

    (define-record-type <hash-table>
      (%table buckets count equiv hashf) hash-table?
      (buckets %buckets %set-buckets!)
      (count %count %set-count!)
      (equiv hash-table-equivalence-function)
      (hashf hash-table-hash-function))

    ;; `(make-hash-table [equal [hash [size]]])`. The default equality is
    ;; `equal?`, whose hash must agree with it -- which is the whole
    ;; contract `equal-hash` is written to.
    (define (make-hash-table . opt)
      (let* ((equiv (if (pair? opt) (car opt) equal?))
             (opt (if (pair? opt) (cdr opt) '()))
             (hashf (if (pair? opt) (car opt) (%hash-for equiv)))
             (opt (if (pair? opt) (cdr opt) '()))
             (size (max 8 (if (pair? opt) (car opt) 8))))
        (%table (make-vector size '()) 0 equiv hashf)))

    ;; A caller who named an equality but no hash gets one that agrees
    ;; with it. `eq?` and `eqv?` need only identity, which is cheaper and
    ;; cycle-proof; anything else is given the structural hash, which is
    ;; the only safe default.
    (define (%hash-for equiv)
      (cond ((eq? equiv eq?) eqv-hash)
            ((eq? equiv eqv?) eqv-hash)
            ((eq? equiv string=?) string-hash)
            ((eq? equiv string-ci=?) string-ci-hash)
            (else equal-hash)))

    (define (%index table key)
      ((hash-table-hash-function table) key (vector-length (%buckets table))))

    ;; The bucket entry for `key`, or #f.
    (define (%entry table key)
      (let ((equiv (hash-table-equivalence-function table)))
        (let loop ((b (vector-ref (%buckets table) (%index table key))))
          (cond ((null? b) #f)
                ((equiv key (caar b)) (car b))
                (else (loop (cdr b)))))))

    ;; `(hash-table-ref table key [failure [success]])`. With no failure
    ;; thunk a missing key is an error, which is this SRFI's choice and
    ;; the reason `/default` exists beside it.
    (define (hash-table-ref table key . opt)
      (let ((e (%entry table key)))
        (cond (e (if (and (pair? opt) (pair? (cdr opt)))
                     ((cadr opt) (cdr e))
                     (cdr e)))
              ((pair? opt) ((car opt)))
              (else (error "hash-table-ref: no such key" key)))))

    (define (hash-table-ref/default table key default)
      (let ((e (%entry table key)))
        (if e (cdr e) default)))

    (define (hash-table-exists? table key)
      (if (%entry table key) #t #f))

    (define (hash-table-set! table key value)
      (let ((e (%entry table key)))
        (if e
            (set-cdr! e value)
            (let ((i (%index table key)))
              (vector-set! (%buckets table) i
                           (cons (cons key value)
                                 (vector-ref (%buckets table) i)))
              (%set-count! table (+ 1 (%count table)))
              (when (> (%count table) (vector-length (%buckets table)))
                (%grow! table))))))

    ;; Double the buckets and rehash. The load factor is one entry per
    ;; bucket, so a walk is a couple of comparisons on average.
    (define (%grow! table)
      (let* ((old (%buckets table))
             (new (make-vector (* 2 (vector-length old)) '())))
        (%set-buckets! table new)
        (let loop ((i 0))
          (when (< i (vector-length old))
            (let inner ((b (vector-ref old i)))
              (unless (null? b)
                (let ((j ((hash-table-hash-function table)
                          (caar b) (vector-length new))))
                  (vector-set! new j (cons (car b) (vector-ref new j))))
                (inner (cdr b))))
            (loop (+ i 1))))))

    (define (hash-table-delete! table key)
      (let* ((equiv (hash-table-equivalence-function table))
             (i (%index table key))
             (b (vector-ref (%buckets table) i)))
        (let loop ((b b) (acc '()) (found #f))
          (cond ((null? b)
                 (when found
                   (vector-set! (%buckets table) i (reverse acc))
                   (%set-count! table (- (%count table) 1))))
                ((equiv key (caar b)) (loop (cdr b) acc #t))
                (else (loop (cdr b) (cons (car b) acc) found))))))

    ;; `(hash-table-update! table key proc [default-thunk])`
    (define (hash-table-update! table key proc . opt)
      (let ((e (%entry table key)))
        (cond (e (set-cdr! e (proc (cdr e))))
              ((pair? opt) (hash-table-set! table key (proc ((car opt)))))
              (else (error "hash-table-update!: no such key" key)))))

    (define (hash-table-update!/default table key proc default)
      (hash-table-update! table key proc (lambda () default)))

    (define (hash-table-size table) (%count table))

    (define (hash-table-fold table f init)
      (let ((bs (%buckets table)))
        (let loop ((i 0) (acc init))
          (if (>= i (vector-length bs))
              acc
              (let inner ((b (vector-ref bs i)) (acc acc))
                (if (null? b)
                    (loop (+ i 1) acc)
                    (inner (cdr b) (f (caar b) (cdar b) acc))))))))

    (define (hash-table-walk table proc)
      (hash-table-fold table (lambda (k v acc) (proc k v) acc) #f)
      #t)

    (define (hash-table-keys table)
      (hash-table-fold table (lambda (k v acc) (cons k acc)) '()))

    (define (hash-table-values table)
      (hash-table-fold table (lambda (k v acc) (cons v acc)) '()))

    (define (hash-table->alist table)
      (hash-table-fold table (lambda (k v acc) (cons (cons k v) acc)) '()))

    (define (hash-table-copy table)
      (let ((new (make-hash-table (hash-table-equivalence-function table)
                                  (hash-table-hash-function table)
                                  (vector-length (%buckets table)))))
        (hash-table-walk table (lambda (k v) (hash-table-set! new k v)))
        new))

    ;; Entries of `from` that `to` does not already have.
    (define (hash-table-merge! to from)
      (hash-table-walk from
                       (lambda (k v)
                         (unless (hash-table-exists? to k)
                           (hash-table-set! to k v))))
      to)

    (define (alist->hash-table alist . opt)
      (let ((t (apply make-hash-table opt)))
        ;; Earlier entries win, as an alist's own lookup would.
        (let loop ((as (reverse alist)))
          (unless (null? as)
            (hash-table-set! t (caar as) (cdar as))
            (loop (cdr as))))
        t))))
