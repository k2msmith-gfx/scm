;;; (srfi 69) — Basic Hash Tables.
;;;
;;; One interface onto `(scm hash-table)`; `(srfi 125)` is the other, and a
;;; table made here works there and the other way round, which is what
;;; SRFI 125's compatibility clause asks for.
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
  (import (scheme base) (scm hash-table))
  (begin

    (define hash equal-hash)
    (define hash-by-identity eqv-hash)

    (define hash-table? table?)
    (define hash-table-equivalence-function table-equiv)
    (define hash-table-hash-function table-hashf)
    (define hash-table-size table-count)
    (define hash-table-ref/default table-ref/default)
    (define hash-table-set! table-set!)

    ;; `(make-hash-table [equal [hash [size]]])`. The default equality is
    ;; `equal?`, whose hash must agree with it -- the contract
    ;; `equal-hash` is written to satisfy.
    (define (make-hash-table . opt)
      (let* ((equiv (if (pair? opt) (car opt) equal?))
             (opt (if (pair? opt) (cdr opt) '()))
             (hashf (if (pair? opt) (car opt) (hash-for-equality equiv)))
             (opt (if (pair? opt) (cdr opt) '()))
             (size (if (pair? opt) (car opt) 8)))
        (make-table equiv hashf #f size #t)))

    ;; `(hash-table-ref table key [failure [success]])`. With no failure
    ;; thunk a missing key is an error, which is this SRFI's choice and
    ;; the reason `/default` exists beside it.
    (define (hash-table-ref table key . opt)
      (let ((e (table-entry table key)))
        (cond (e (if (and (pair? opt) (pair? (cdr opt)))
                     ((cadr opt) (cdr e))
                     (cdr e)))
              ((pair? opt) ((car opt)))
              (else (error "hash-table-ref: no such key" key)))))

    (define (hash-table-exists? table key)
      (if (table-entry table key) #t #f))

    (define (hash-table-delete! table key)
      (table-delete! table key)
      #t)

    ;; `(hash-table-update! table key proc [default-thunk])`
    (define (hash-table-update! table key proc . opt)
      (let ((e (table-entry table key)))
        (cond (e (table-check-mutable table 'update!) (set-cdr! e (proc (cdr e))))
              ((pair? opt) (table-set! table key (proc ((car opt)))))
              (else (error "hash-table-update!: no such key" key)))))

    (define (hash-table-update!/default table key proc default)
      (hash-table-update! table key proc (lambda () default)))

    (define (hash-table-fold table f init) (table-fold table f init))

    (define (hash-table-walk table proc)
      (table-fold table (lambda (k v acc) (proc k v) acc) #f)
      #t)

    (define (hash-table-keys table)
      (table-fold table (lambda (k v acc) (cons k acc)) '()))

    (define (hash-table-values table)
      (table-fold table (lambda (k v acc) (cons v acc)) '()))

    (define (hash-table->alist table)
      (table-fold table (lambda (k v acc) (cons (cons k v) acc)) '()))

    (define (hash-table-copy table) (table-copy table #t))

    ;; Entries of `from` that `to` does not already have.
    (define (hash-table-merge! to from)
      (hash-table-walk from
                       (lambda (k v)
                         (unless (hash-table-exists? to k)
                           (table-set! to k v))))
      to)

    (define (alist->hash-table alist . opt)
      (let ((t (apply make-hash-table opt)))
        ;; Earlier entries win, as an alist's own lookup would.
        (let loop ((as (reverse alist)))
          (unless (null? as)
            (table-set! t (caar as) (cdar as))
            (loop (cdr as))))
        t))))
