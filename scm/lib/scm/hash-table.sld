;;; (scm hash-table) — the table itself, which two SRFIs present.
;;;
;;; `(srfi 69)` and `(srfi 125)` are two interfaces to one data type, so
;;; there is one implementation here and each of them exports a view of it.
;;; That is not only tidier: SRFI 125 says it is compatible with SRFI 69,
;;; and a table made through either API has to work through the other,
;;; which separate implementations could not give.
;;;
;;; A table is a vector of buckets, each an association list. A lookup
;;; hashes the key to a bucket and walks it with the table's own equality;
;;; the table doubles when it holds more entries than buckets, so a walk
;;; stays short and the cost is amortised constant. The hash functions are
;;; native -- `equal-hash` walks structure in Rust -- which is where the
;;; work actually is.
(define-library (scm hash-table)
  (export table? make-table table-equiv table-hashf table-comparator
          table-mutable? table-count table-buckets
          table-entry table-ref/default table-set! table-delete!
          table-fold table-clear! table-copy table-copy-into
          table-check-mutable
          hash-for-equality)
  (import (scheme base))
  (begin

    (define-record-type <hash-table>
      (%table buckets count equiv hashf comparator mutable) table?
      (buckets table-buckets set-buckets!)
      (count table-count set-count!)
      (equiv table-equiv)
      (hashf table-hashf)
      (comparator table-comparator)
      (mutable table-mutable?))

    ;; A caller who named an equality but no hash gets one that agrees
    ;; with it. `eq?` and `eqv?` need only identity, which is cheaper and
    ;; cycle-proof; anything else is given the structural hash, the only
    ;; safe default.
    (define (hash-for-equality equiv)
      (cond ((eq? equiv eq?) eqv-hash)
            ((eq? equiv eqv?) eqv-hash)
            ((eq? equiv string=?) string-hash)
            ((eq? equiv string-ci=?) string-ci-hash)
            (else equal-hash)))

    (define (make-table equiv hashf comparator size mutable)
      (%table (make-vector (max 8 size) '()) 0 equiv hashf comparator mutable))

    (define (table-check-mutable t who)
      (unless (table-mutable? t)
        (error "this hash table is immutable" who)))

    (define (%index t key) ((table-hashf t) key (vector-length (table-buckets t))))

    ;; The `(key . value)` pair for `key`, or #f.
    (define (table-entry t key)
      (let ((equiv (table-equiv t)))
        (let loop ((b (vector-ref (table-buckets t) (%index t key))))
          (cond ((null? b) #f)
                ((equiv key (caar b)) (car b))
                (else (loop (cdr b)))))))

    (define (table-ref/default t key default)
      (let ((e (table-entry t key)))
        (if e (cdr e) default)))

    (define (table-set! t key value)
      (table-check-mutable t 'set!)
      (let ((e (table-entry t key)))
        (if e
            (set-cdr! e value)
            (let ((i (%index t key)))
              (vector-set! (table-buckets t) i
                           (cons (cons key value) (vector-ref (table-buckets t) i)))
              (set-count! t (+ 1 (table-count t)))
              (when (> (table-count t) (vector-length (table-buckets t)))
                (%grow! t))))))

    ;; Double and rehash. One entry per bucket is the load factor, so a
    ;; walk is a couple of comparisons on average.
    (define (%grow! t)
      (let* ((old (table-buckets t))
             (new (make-vector (* 2 (vector-length old)) '())))
        (set-buckets! t new)
        (let loop ((i 0))
          (when (< i (vector-length old))
            (let inner ((b (vector-ref old i)))
              (unless (null? b)
                (let ((j ((table-hashf t) (caar b) (vector-length new))))
                  (vector-set! new j (cons (car b) (vector-ref new j))))
                (inner (cdr b))))
            (loop (+ i 1))))))

    ;; Returns whether anything was removed, which both interfaces want.
    (define (table-delete! t key)
      (table-check-mutable t 'delete!)
      (let* ((equiv (table-equiv t))
             (i (%index t key))
             (b (vector-ref (table-buckets t) i)))
        (let loop ((b b) (acc '()) (found #f))
          (cond ((null? b)
                 (when found
                   (vector-set! (table-buckets t) i (reverse acc))
                   (set-count! t (- (table-count t) 1)))
                 found)
                ((equiv key (caar b)) (loop (cdr b) acc #t))
                (else (loop (cdr b) (cons (car b) acc) found))))))

    (define (table-fold t f init)
      (let ((bs (table-buckets t)))
        (let loop ((i 0) (acc init))
          (if (>= i (vector-length bs))
              acc
              (let inner ((b (vector-ref bs i)) (acc acc))
                (if (null? b)
                    (loop (+ i 1) acc)
                    (inner (cdr b) (f (caar b) (cdar b) acc))))))))

    (define (table-clear! t)
      (table-check-mutable t 'clear!)
      (set-buckets! t (make-vector 8 '()))
      (set-count! t 0))

    ;; A fresh table with the same equality, hash, comparator and
    ;; contents, and the mutability asked for. This does not go through
    ;; `table-set!`, because the result may be immutable from birth --
    ;; which is the only way to make one, the flag being fixed when the
    ;; record is made. The bucket count and hash are unchanged, so no
    ;; rehashing is needed; each entry pair is rebuilt so that updating
    ;; one table cannot be seen through the other.
    (define (table-copy t mutable)
      (let* ((old (table-buckets t))
             (new (make-vector (vector-length old) '()))
             (c (%table new 0 (table-equiv t) (table-hashf t)
                        (table-comparator t) mutable)))
        (let loop ((i 0))
          (when (< i (vector-length old))
            (vector-set! new i
                         (map (lambda (p) (cons (car p) (cdr p)))
                              (vector-ref old i)))
            (loop (+ i 1))))
        (set-count! c (table-count t))
        c))

    ;; Every entry of `from` into `to`, newest winning -- which is what
    ;; a union wants.
    (define (table-copy-into from to)
      (table-fold from (lambda (k v acc) (table-set! to k v) acc) #f)
      to)))
