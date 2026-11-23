;;; The machine's `map`, `for-each`, `filter`, `reduce` and `(srfi 1)`'s
;;; `fold` and `for-each` (S41, src/vm/iter.rs) against the Scheme they
;;; replaced.
;;;
;;; The originals are below, verbatim from the prelude and from
;;; lib/srfi/1.sld before S41, renamed ref-…. (srfi 1)'s used its own
;;; `map` for `(map car ls)`; here that is the native one, which this file
;;; holds to the prelude's.
;;;
;;; The claim is not only the same values but THE SAME CALLS IN THE SAME
;;; ORDER: every procedure here logs its arguments, and `kons` is not
;;; commutative. A procedure that mutates the pair it was handed sees what
;;; the Scheme loop saw, and an improper list fails after the same calls.
(import (scm test) (scheme base) (prefix (srfi 1) s1:))

;; ── the originals: the prelude ───────────────────────────────────────
(define (ref-map f lst . more)
  (if (null? more)
      (let loop ((lst lst) (acc '()))
        (if (null? lst)
            (reverse acc)
            (loop (cdr lst) (cons (f (car lst)) acc))))
      (let loop ((ls (cons lst more)) (acc '()))
        (if (ref-any-null? ls)
            (reverse acc)
            (loop (ref-map-cdr ls) (cons (apply f (ref-map-car ls)) acc))))))

(define (ref-any-null? ls)
  (cond [(null? ls) #f]
        [(null? (car ls)) #t]
        [else (ref-any-null? (cdr ls))]))

(define (ref-map-car ls) (if (null? ls) '() (cons (car (car ls)) (ref-map-car (cdr ls)))))
(define (ref-map-cdr ls) (if (null? ls) '() (cons (cdr (car ls)) (ref-map-cdr (cdr ls)))))

(define (ref-for-each f lst . more)
  (if (null? more)
      (let loop ((lst lst))
        (if (null? lst)
            (if #f #f)
            (begin (f (car lst)) (loop (cdr lst)))))
      (let loop ((ls (cons lst more)))
        (if (ref-any-null? ls)
            (if #f #f)
            (begin (apply f (ref-map-car ls)) (loop (ref-map-cdr ls)))))))

(define (ref-filter pred lst)
  (let loop ((lst lst) (acc '()))
    (cond [(null? lst) (reverse acc)]
          [(pred (car lst)) (loop (cdr lst) (cons (car lst) acc))]
          [else (loop (cdr lst) acc)])))

(define (ref-reduce f init lst)
  (let loop ((lst lst) (acc init))
    (if (null? lst) acc (loop (cdr lst) (f acc (car lst))))))

;; ── the originals: (srfi 1) ──────────────────────────────────────────
(define (ref-any-empty? ls)
  (let loop ((ls ls))
    (cond ((null? ls) #f)
          ((null? (car ls)) #t)
          (else (loop (cdr ls))))))

(define (ref-fold kons knil l . more)
  (if (null? more)
      (let loop ((l l) (acc knil))
        (if (null? l) acc (loop (cdr l) (kons (car l) acc))))
      (let loop ((ls (cons l more)) (acc knil))
        (if (ref-any-empty? ls)
            acc
            (loop (map cdr ls)
                  (apply kons (append (map car ls) (list acc))))))))

(define (ref-map-cars ls)
  (let loop ((ls ls) (acc '()))
    (if (null? ls) (reverse acc) (loop (cdr ls) (cons (caar ls) acc)))))
(define (ref-map-cdrs ls)
  (let loop ((ls ls) (acc '()))
    (if (null? ls) (reverse acc) (loop (cdr ls) (cons (cdar ls) acc)))))

(define (ref-s1-for-each f l . more)
  (if (null? more)
      (let loop ((l l))
        (if (null? l)
            #t
            (begin (f (car l)) (loop (cdr l)))))
      (let loop ((ls (cons l more)))
        (if (ref-any-empty? ls)
            #t
            (begin (apply f (ref-map-cars ls)) (loop (ref-map-cdrs ls)))))))

;; ── the harness ──────────────────────────────────────────────────────
;; A procedure that logs every call; `(run iter make-proc args)` is the
;; value, or the error's message, and the log, so two runs compare by
;; `equal?`.
(define log '())
(define (logging f) (lambda args (set! log (cons args log)) (apply f args)))
(define (outcome thunk)
  (set! log '())
  (let ((v (guard (e (#t (list 'raised))) (thunk))))
    (list v (reverse log))))

;; Unspecified prints as itself; compare it as a token.
(define (norm x) (if (eq? x (if #f #f)) 'unspecified x))

(define (agree name native ref)
  (check-value* name (norm-outcome (outcome native)) (norm-outcome (outcome ref))))
(define (norm-outcome o) (list (norm (car o)) (cadr o)))
(define failures-here '())
(define (check-value* name got want)
  (if (not (equal? got want))
      (set! failures-here (cons (list name 'got got 'wanted want) failures-here))))

(define lists
  (list '() '(1) '(1 2) '(1 2 3) '(5 4 3 2 1) '(a b c d e f g)))

;; Non-commutative, so an argument in the wrong place shows.
(define (kons . args) (cons 'k args))
(define (tag . args) (cons 't args))
(define (keep x) (and (number? x) (odd? x)))

;; One list, every shape.
(for-each
 (lambda (l)
   (agree (list 'map l) (lambda () (map (logging tag) l)) (lambda () (ref-map (logging tag) l)))
   (agree (list 'for-each l) (lambda () (for-each (logging tag) l)) (lambda () (ref-for-each (logging tag) l)))
   (agree (list 'filter l) (lambda () (filter (logging keep) l)) (lambda () (ref-filter (logging keep) l)))
   (agree (list 'reduce l) (lambda () (reduce (logging kons) 'z l)) (lambda () (ref-reduce (logging kons) 'z l)))
   (agree (list 'fold l) (lambda () (s1:fold (logging kons) 'z l)) (lambda () (ref-fold (logging kons) 'z l)))
   (agree (list 's1-for-each l) (lambda () (s1:for-each (logging tag) l)) (lambda () (ref-s1-for-each (logging tag) l)))
   (agree (list 's1-map l) (lambda () (s1:map (logging tag) l)) (lambda () (ref-map (logging tag) l)))
   (agree (list 's1-filter l) (lambda () (s1:filter (logging keep) l)) (lambda () (ref-filter (logging keep) l))))
 lists)

;; Two and three lists, of every pair of lengths: stops at the shortest.
(for-each
 (lambda (a)
   (for-each
    (lambda (b)
      (agree (list 'map2 a b) (lambda () (map (logging tag) a b)) (lambda () (ref-map (logging tag) a b)))
      (agree (list 'map3 a b) (lambda () (map (logging tag) a b a)) (lambda () (ref-map (logging tag) a b a)))
      (agree (list 'for-each2 a b) (lambda () (for-each (logging tag) a b)) (lambda () (ref-for-each (logging tag) a b)))
      (agree (list 'fold2 a b) (lambda () (s1:fold (logging kons) 'z a b)) (lambda () (ref-fold (logging kons) 'z a b)))
      (agree (list 'fold3 a b) (lambda () (s1:fold (logging kons) 'z b a b)) (lambda () (ref-fold (logging kons) 'z b a b)))
      (agree (list 's1-for-each2 a b) (lambda () (s1:for-each (logging tag) a b)) (lambda () (ref-s1-for-each (logging tag) a b))))
    lists))
 lists)

;; Improper lists and non-lists: the same calls, then an error.
(for-each
 (lambda (l)
   (agree (list 'map-bad l) (lambda () (map (logging tag) l)) (lambda () (ref-map (logging tag) l)))
   (agree (list 'for-each-bad l) (lambda () (for-each (logging tag) l)) (lambda () (ref-for-each (logging tag) l)))
   (agree (list 'filter-bad l) (lambda () (filter (logging keep) l)) (lambda () (ref-filter (logging keep) l)))
   (agree (list 'reduce-bad l) (lambda () (reduce (logging kons) 'z l)) (lambda () (ref-reduce (logging kons) 'z l)))
   (agree (list 'fold-bad l) (lambda () (s1:fold (logging kons) 'z l)) (lambda () (ref-fold (logging kons) 'z l)))
   (agree (list 'map2-bad l) (lambda () (map (logging tag) '(1 2 3) l)) (lambda () (ref-map (logging tag) '(1 2 3) l)))
   (agree (list 'fold2-bad l) (lambda () (s1:fold (logging kons) 'z l '(1 2 3))) (lambda () (ref-fold (logging kons) 'z l '(1 2 3)))))
 (list 5 '(1 . 2) '(1 2 . 3) "abc" #(1 2)))

;; A procedure that rewrites the list it is walking: `map` and `fold` had
;; taken the tail already, `for-each` and `filter` had not, and `filter`
;; reads the element again after the predicate.
(define (cut-after! p) (lambda (x) (set-cdr! p '()) x))
(let ((a (list 1 2 3)) (b (list 1 2 3)))
  (check native-map-mutating (map (lambda (x) (when (= x 1) (set-cdr! a '())) x) a)
         (ref-map (lambda (x) (when (= x 1) (set-cdr! b '())) x) b)))
(let ((a (list 1 2 3)) (b (list 1 2 3)) (la '()) (lb '()))
  (for-each (lambda (x) (set! la (cons x la)) (when (= x 1) (set-cdr! a '()))) a)
  (ref-for-each (lambda (x) (set! lb (cons x lb)) (when (= x 1) (set-cdr! b '()))) b)
  (check native-for-each-mutating la lb))
(let ((a (list 1 2 3)) (b (list 1 2 3)))
  (check native-filter-mutating
         (filter (lambda (x) (set-car! a 'changed) #t) a)
         (ref-filter (lambda (x) (set-car! b 'changed) #t) b)))

;; ── errors, escapes, unwinding ───────────────────────────────────────
(define (boom-at n) (lambda (x . _) (if (eqv? x n) (raise (list 'boom x)) x)))
(check native-map-raise-guarded
       (guard (e (#t e)) (map (boom-at 2) '(1 2 3)))
       '(boom 2))
(check native-for-each-raise-guarded
       (guard (e (#t e)) (for-each (boom-at 3) '(1 2 3)))
       '(boom 3))
(check native-fold-raise-guarded
       (guard (e (#t e)) (s1:fold (boom-at 1) 0 '(1 2 3)))
       '(boom 1))
(check native-filter-error-object
       (guard (e ((error-object? e) (error-object-message e)))
         (filter (lambda (x) (error "nope" x)) '(1)))
       "nope")
(check native-map-escape
       (call/cc (lambda (k) (map (lambda (x) (if (= x 2) (k 'escaped) x)) '(1 2 3))))
       'escaped)
(check native-fold-escape
       (call/cc (lambda (k) (s1:fold (lambda (x acc) (if (= x 3) (k acc) (+ x acc))) 0 '(1 2 3 4))))
       3)
;; An escape and an error each leave through a dynamic-wind inside the
;; procedure and one around the map, running both afters, innermost first.
(let ((trail '()))
  (define (note x) (set! trail (cons x trail)))
  (call/cc
   (lambda (k)
     (dynamic-wind
      (lambda () (note 'in))
      (lambda ()
        (map (lambda (x)
               (dynamic-wind (lambda () (note (list 'in x)))
                             (lambda () (if (= x 2) (k 'out) x))
                             (lambda () (note (list 'out x)))))
             '(1 2 3)))
      (lambda () (note 'out)))))
  (check native-map-escape-unwinds (reverse trail) '(in (in 1) (out 1) (in 2) (out 2) out)))
(let ((trail '()))
  (define (note x) (set! trail (cons x trail)))
  (guard (e (#t (note (list 'caught e))))
    (dynamic-wind
     (lambda () (note 'in))
     (lambda () (for-each (boom-at 2) '(1 2 3)))
     (lambda () (note 'out))))
  (check native-for-each-error-unwinds (reverse trail) '(in out (caught (boom 2)))))
;; A continuable raise returns into the map, which carries on.
(check native-map-continuable
       (with-exception-handler
        (lambda (e) (* e 100))
        (lambda () (map (lambda (x) (+ 1 (raise-continuable x))) '(1 2 3))))
       '(101 201 301))

;; The errors map itself raises name it.
(define (message-of thunk)
  (guard (e ((error-object? e) (error-object-message e)) (#t e)) (thunk) 'no-error))
(check native-map-names-itself (message-of (lambda () (map car 5))) "map: not a list: 5")
(check native-map-names-itself-2 (message-of (lambda () (map car '((1) . 7)))) "map: not a list: 7")
(check native-for-each-names-itself (message-of (lambda () (for-each values '(1 . 2)))) "for-each: not a list: 2")
(check native-fold-names-itself (message-of (lambda () (s1:fold cons '() 3))) "fold: not a list: 3")
(check native-filter-names-itself (message-of (lambda () (filter odd? 'x))) "filter: not a list: x")
(check native-reduce-names-itself (message-of (lambda () (reduce + 0 #t))) "reduce: not a list: #t")
(check native-map-arity (message-of (lambda () (map car))) "map expects 2 required arguments, got 1")
(check native-fold-arity (message-of (lambda () (s1:fold cons '()))) "fold expects 3 required arguments, got 2")
(check native-filter-arity (message-of (lambda () (filter odd? '(1) '(2)))) "filter got 3 arguments, which is more than it takes")
(check-raises native-map-not-applicable (map 5 '(1)))
(check native-map-not-applicable-empty (map 5 '()) '())

;; ── every kind of procedure, and long lists ─────────────────────────
(check native-map-prim (map car '((1) (2))) '(1 2))
(check native-map-prim-n (map + '(1 2) '(10 20) '(100 200)) '(111 222))
(check native-map-variadic-lambda (map (lambda args args) '(1 2) '(a b)) '((1 a) (2 b)))
(check native-map-optional (map (lambda (x &opt (y 7)) (list x y)) '(1 2)) '((1 7) (2 7)))
(check native-map-continuation (call/cc (lambda (k) (map k '(1 2)))) 1)
(check native-map-parameter
       (let ((p (make-parameter 4))) (map (lambda (x) (+ x (p))) '(1 2)))
       '(5 6))
(check native-map-nested (map (lambda (l) (map (lambda (x) (* x x)) l)) '((1 2) (3))) '((1 4) (9)))
(check native-map-apply (apply map list '((1 2) (3 4))) '((1 3) (2 4)))
(check native-map-as-value (let ((m map)) (m - '(1 2))) '(-1 -2))
(define big (let loop ((i 0) (acc '())) (if (= i 100000) acc (loop (+ i 1) (cons i acc)))))
(check native-map-long (length (map (lambda (x) (+ x 1)) big)) 100000)
(check native-fold-long (s1:fold + 0 big) 4999950000)
(check native-reduce-long (reduce + 0 big) 4999950000)
(check native-filter-long (length (filter even? big)) 50000)
(check native-for-each-long (let ((n 0)) (for-each (lambda (x) (set! n (+ n 1))) big) n) 100000)
;; What is built on them.
(check s1-reduce (s1:reduce - 0 '(1 2 3 4)) 2)
(check s1-append-map (s1:append-map (lambda (x) (list x x)) '(1 2)) '(1 1 2 2))
(check vector-map-n (vector-map + #(1 2 3) #(10 20)) #(11 22))
(check fold-left (fold-left - 0 '(1 2 3)) -6)
(check s1-for-each-value (s1:for-each car '()) #t)

(append (reverse failures-here) (test-failures))
