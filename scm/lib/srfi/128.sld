;;; (srfi 128) — Comparators.
;;;
;;; A comparator bundles the four things a container needs to know about
;;; its elements: whether an object is one, when two are equal, how they
;;; order, and how to hash one. It is what `(srfi 125)` hash tables and
;;; any sorted container take instead of a loose pair of procedures.
;;;
;;; Two decisions worth stating.
;;;
;;; **The `eq?` and `eqv?` comparators are unordered.** A comparator's
;;; four parts have to agree with each other, and the default total order
;;; is consistent with `equal?` rather than with identity: two distinct
;;; but equal strings are unordered by it and *not* `eq?`. Offering an
;;; ordering there would be offering a contradiction. Their hash is by
;;; identity for the same reason -- a structural hash of a key that is
;;; later mutated would lose the entry that `eq?` can still find.
;;;
;;; **`comparator-register-default!` is absent.** It mutates a global
;;; ordering from anywhere in a program, and nothing here needs it.
(define-library (srfi 128)
  (export comparator? comparator-ordered? comparator-hashable?
          make-comparator make-pair-comparator make-list-comparator
          make-vector-comparator make-eq-comparator make-eqv-comparator
          make-equal-comparator
          boolean-hash char-hash char-ci-hash string-hash string-ci-hash
          symbol-hash number-hash default-hash
          comparator-type-test-predicate comparator-equality-predicate
          comparator-ordering-predicate comparator-hash-function
          comparator-test-type comparator-check-type comparator-hash
          make-default-comparator
          =? <? >? <=? >=? comparator-if<=> hash-bound hash-salt)
  (import (scheme base))
  (begin

    (define-record-type <comparator>
      (%comparator type-test equality ordering hash) comparator?
      (type-test comparator-type-test-predicate)
      (equality comparator-equality-predicate)
      (ordering comparator-ordering-predicate)
      (hash comparator-hash-function))

    ;; `type-test` may be `#t` for "every object", and `equality` may be
    ;; `#t` to be derived from the ordering -- both of which the SRFI
    ;; allows and real code uses.
    (define (make-comparator type-test equality ordering hash)
      (%comparator
       (if (eq? type-test #t) (lambda (x) #t) type-test)
       (if (eq? equality #t)
           (lambda (a b) (not (or (ordering a b) (ordering b a))))
           equality)
       ordering
       hash))

    (define (comparator-ordered? c)
      (if (comparator-ordering-predicate c) #t #f))
    (define (comparator-hashable? c)
      (if (comparator-hash-function c) #t #f))

    (define (comparator-test-type c obj) ((comparator-type-test-predicate c) obj))

    (define (comparator-check-type c obj)
      (if (comparator-test-type c obj)
          #t
          (error "comparator: this object is not of the right type" obj)))

    (define (comparator-hash c obj) ((comparator-hash-function c) obj))

    ;; ── the hash functions ────────────────────────────────────────────
    ;;
    ;; `string-hash`, `string-ci-hash`, `symbol-hash` and `number-hash`
    ;; are the native primitives; the other two are one line each on top
    ;; of them. Every one takes the SRFI's optional second argument.
    (define default-hash equal-hash)
    (define (boolean-hash b . opt) (apply number-hash (if b 1 0) opt))
    (define (char-hash c . opt) (apply number-hash (char->integer c) opt))
    (define (char-ci-hash c . opt)
      (apply number-hash (char->integer (char-downcase c)) opt))

    ;; ── the default total order ───────────────────────────────────────
    ;;
    ;; Objects of different types order by the SRFI's list: the empty
    ;; list, pairs, booleans, characters, strings, symbols, numbers,
    ;; vectors, bytevectors, then everything else -- which is unordered
    ;; among itself, so two procedures compare neither less nor greater.
    (define (%type-rank x)
      (cond ((null? x) 0)
            ((pair? x) 1)
            ((boolean? x) 2)
            ((char? x) 3)
            ((string? x) 4)
            ((symbol? x) 5)
            ((number? x) 6)
            ((vector? x) 7)
            ((bytevector? x) 8)
            (else 9)))

    (define (default-ordering a b)
      (let ((ra (%type-rank a)) (rb (%type-rank b)))
        (cond
         ((not (= ra rb)) (< ra rb))
         ((= ra 0) #f)
         ((= ra 1) (%list-ordering a b))
         ((= ra 2) (and (not a) b))
         ((= ra 3) (char<? a b))
         ((= ra 4) (string<? a b))
         ((= ra 5) (string<? (symbol->string a) (symbol->string b)))
         ((= ra 6) (< a b))
         ((= ra 7) (%sequence-ordering a b vector-length vector-ref))
         ((= ra 8) (%sequence-ordering a b bytevector-length bytevector-u8-ref))
         (else #f))))

    ;; Lexicographic, and a shorter list comes first. An improper tail is
    ;; compared as an element, which is what makes this total over pairs.
    (define (%list-ordering a b)
      (cond ((and (null? a) (null? b)) #f)
            ((null? a) #t)
            ((null? b) #f)
            ((not (pair? a)) (if (pair? b) #t (default-ordering a b)))
            ((not (pair? b)) #f)
            ((default-ordering (car a) (car b)) #t)
            ((default-ordering (car b) (car a)) #f)
            (else (%list-ordering (cdr a) (cdr b)))))

    (define (%sequence-ordering a b len ref)
      (let ((la (len a)) (lb (len b)))
        (cond ((not (= la lb)) (< la lb))
              (else
               (let loop ((i 0))
                 (cond ((>= i la) #f)
                       ((default-ordering (ref a i) (ref b i)) #t)
                       ((default-ordering (ref b i) (ref a i)) #f)
                       (else (loop (+ i 1)))))))))

    (define (make-default-comparator)
      (make-comparator #t equal? default-ordering default-hash))

    ;; See the header for why these two are unordered and hash by
    ;; identity.
    (define (make-eq-comparator) (make-comparator #t eq? #f eqv-hash))
    (define (make-eqv-comparator) (make-comparator #t eqv? #f eqv-hash))
    (define (make-equal-comparator)
      (make-comparator #t equal? default-ordering default-hash))

    ;; ── comparators built from comparators ────────────────────────────
    (define (make-pair-comparator car-c cdr-c)
      (make-comparator
       (lambda (x)
         (and (pair? x)
              (comparator-test-type car-c (car x))
              (comparator-test-type cdr-c (cdr x))))
       (lambda (a b)
         (and ((comparator-equality-predicate car-c) (car a) (car b))
              ((comparator-equality-predicate cdr-c) (cdr a) (cdr b))))
       (and (comparator-ordered? car-c)
            (comparator-ordered? cdr-c)
            (lambda (a b)
              (cond (((comparator-ordering-predicate car-c) (car a) (car b)) #t)
                    (((comparator-ordering-predicate car-c) (car b) (car a)) #f)
                    (else ((comparator-ordering-predicate cdr-c) (cdr a) (cdr b))))))
       (and (comparator-hashable? car-c)
            (comparator-hashable? cdr-c)
            (lambda (x . opt)
              (%combine (comparator-hash car-c (car x))
                        (comparator-hash cdr-c (cdr x))
                        opt)))))

    ;; ── combining hashes ──────────────────────────────────────────────
    ;;
    ;; Every step is reduced modulo `%limit` first, and **this is the
    ;; right way to write it, not a workaround.** The earlier comment here
    ;; blamed the absent bignums (plan §2.3); S18 claims `exact-closed`,
    ;; so the unreduced form would now *work* -- and would be worse. An
    ;; unreduced `(* 31 h)` grows about five bits per element, so folding
    ;; over a long list builds a many-thousand-bit integer, allocates all
    ;; the way, and then reduces it at the end to the same answer. A
    ;; bounded hash stays in a range it can multiply within. 2^25 is
    ;; `(hash-bound)`, and 31 times it is nowhere near the word.
    (define %limit 33554432)

    (define (%mix h x) (modulo (+ (* 31 (modulo h %limit)) (modulo x %limit)) %limit))

    (define (%bounded h opt) (if (pair? opt) (modulo h (car opt)) h))

    ;; Two hashes into one, honouring an optional bound.
    (define (%combine h1 h2 opt) (%bounded (%mix h1 h2) opt))

    (define (make-list-comparator element-c type-test empty? head tail)
      (make-comparator
       (lambda (x)
         (and (type-test x)
              (let loop ((x x))
                (cond ((empty? x) #t)
                      ((comparator-test-type element-c (head x)) (loop (tail x)))
                      (else #f)))))
       (lambda (a b)
         (let loop ((a a) (b b))
           (cond ((empty? a) (empty? b))
                 ((empty? b) #f)
                 (((comparator-equality-predicate element-c) (head a) (head b))
                  (loop (tail a) (tail b)))
                 (else #f))))
       (and (comparator-ordered? element-c)
            (lambda (a b)
              (let loop ((a a) (b b))
                (cond ((empty? a) (not (empty? b)))
                      ((empty? b) #f)
                      (((comparator-ordering-predicate element-c) (head a) (head b)) #t)
                      (((comparator-ordering-predicate element-c) (head b) (head a)) #f)
                      (else (loop (tail a) (tail b)))))))
       (and (comparator-hashable? element-c)
            (lambda (x . opt)
              (let loop ((x x) (h 17))
                (if (empty? x)
                    (%bounded h opt)
                    (loop (tail x)
                          (%mix h (comparator-hash element-c (head x))))))))))

    (define (make-vector-comparator element-c type-test length ref)
      (make-comparator
       (lambda (x)
         (and (type-test x)
              (let loop ((i 0))
                (cond ((>= i (length x)) #t)
                      ((comparator-test-type element-c (ref x i)) (loop (+ i 1)))
                      (else #f)))))
       (lambda (a b)
         (and (= (length a) (length b))
              (let loop ((i 0))
                (cond ((>= i (length a)) #t)
                      (((comparator-equality-predicate element-c) (ref a i) (ref b i))
                       (loop (+ i 1)))
                      (else #f)))))
       (and (comparator-ordered? element-c)
            (lambda (a b)
              (cond ((not (= (length a) (length b))) (< (length a) (length b)))
                    (else
                     (let loop ((i 0))
                       (cond ((>= i (length a)) #f)
                             (((comparator-ordering-predicate element-c) (ref a i) (ref b i)) #t)
                             (((comparator-ordering-predicate element-c) (ref b i) (ref a i)) #f)
                             (else (loop (+ i 1)))))))))
       (and (comparator-hashable? element-c)
            (lambda (x . opt)
              (let loop ((i 0) (h 19))
                (if (>= i (length x))
                    (%bounded h opt)
                    (loop (+ i 1)
                          (%mix h (comparator-hash element-c (ref x i))))))))))

    ;; ── the comparison operators ──────────────────────────────────────
    ;;
    ;; Variadic and chained: `(<? c a b c)` is `a < b` and `b < c`.
    (define (%chain c rel args)
      (let loop ((as args))
        (cond ((null? as) #t)
              ((null? (cdr as)) #t)
              ((rel c (car as) (cadr as)) (loop (cdr as)))
              (else #f))))

    (define (%eq c a b) ((comparator-equality-predicate c) a b))
    (define (%lt c a b) ((comparator-ordering-predicate c) a b))
    (define (%gt c a b) ((comparator-ordering-predicate c) b a))
    (define (%le c a b) (not (%gt c a b)))
    (define (%ge c a b) (not (%lt c a b)))

    (define (=? c . args) (%chain c %eq args))
    (define (<? c . args) (%chain c %lt args))
    (define (>? c . args) (%chain c %gt args))
    (define (<=? c . args) (%chain c %le args))
    (define (>=? c . args) (%chain c %ge args))

    ;; `(comparator-if<=> [comparator] a b less equal greater)`
    (define-syntax comparator-if<=>
      (syntax-rules ()
        ((_ a b less equal greater)
         (comparator-if<=> (make-default-comparator) a b less equal greater))
        ((_ comparator a b less equal greater)
         (let ((cmp comparator) (x a) (y b))
           (cond ((=? cmp x y) equal)
                 ((<? cmp x y) less)
                 (else greater))))))

    ;; The largest bound a hash function may be asked for, and a salt that
    ;; is a constant here: a per-run random salt would make a hash that
    ;; cannot be written down, and nothing in this implementation needs
    ;; one.
    (define-syntax hash-bound
      (syntax-rules () ((_) 33554432)))
    (define-syntax hash-salt
      (syntax-rules () ((_) 16064047)))))
