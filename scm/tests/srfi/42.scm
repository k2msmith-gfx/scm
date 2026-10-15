;;; (srfi 42) eager comprehensions.
;;;
;;; The generators are spelled `:list`, `:range` and so on, which scm's
;;; reader turns into KEYWORDS rather than identifiers (plan §4). They work
;;; anyway, because a keyword in a macro pattern matches as a constant and
;;; a generator is only ever a qualifier -- so this file is also the check
;;; that the two decisions coexist.
(import (scm test) (srfi 42) (scheme base) (scheme inexact))

;; ── the specification's own examples ─────────────────────────────────
(check spec-squares (list-ec (: i 5) (* i i)) '(0 1 4 9 16))
(check spec-nested (list-ec (: n 1 4) (: i n) (list n i))
       '((1 0) (2 0) (2 1) (3 0) (3 1) (3 2)))
(check spec-sum (sum-ec (: i 1 5) i) 10)

;; ── generators ───────────────────────────────────────────────────────
(check range-stop (list-ec (:range i 4) i) '(0 1 2 3))
(check range-start-stop (list-ec (:range i 2 5) i) '(2 3 4))
(check range-step (list-ec (:range i 0 10 3) i) '(0 3 6 9))
(check range-negative-step (list-ec (:range i 5 0 -2) i) '(5 3 1))
(check range-empty (list-ec (:range i 3 3) i) '())
(check range-rejects-a-zero-step
       (guard (e (#t 'raised)) (list-ec (:range i 0 5 0) i)) 'raised)

(check real-range (list-ec (:real-range x 0.0 1.0 0.25) x) '(0.0 0.25 0.5 0.75))
;; Computed as start + i*step, so the values do not drift.
(check real-range-does-not-drift
       (let ((xs (list-ec (:real-range x 0.0 1.0 0.1) x)))
         (list (length xs) (< (abs (- (list-ref xs 9) 0.9)) 1e-12)))
       '(10 #t))
(check real-range-exact-stays-exact (list-ec (:real-range x 0 3 1) x) '(0 1 2))

(check char-range (string-ec (:char-range c #\a #\e) c) "abcde")
(check char-range-is-inclusive (string-length (string-ec (:char-range c #\a #\a) c)) 1)

(check list-generator (list-ec (:list x '(1 2 3)) x) '(1 2 3))
(check list-generator-many (list-ec (:list x '(1 2) '(3)) x) '(1 2 3))
(check string-generator (list-ec (:string c "ab") c) '(#\a #\b))
(check string-generator-many (string-ec (:string c "ab" "c") c) "abc")
(check vector-generator (list-ec (:vector x (vector 1 2)) x) '(1 2))
(check vector-generator-many (list-ec (:vector x (vector 1) (vector 2 3)) x) '(1 2 3))

;; :integers is unbounded, so something else must stop it.
(check integers-with-while (list-ec (:while (:integers i) (< i 4)) i) '(0 1 2 3))

;; :let binds a local without leaving the qualifier list.
(check let-generator (list-ec (: i 3) (:let sq (* i i)) sq) '(0 1 4))
(check let-is-one-iteration (list-ec (:let x 5) x) '(5))

;; :do is the low-level form.
(check do-generator (list-ec (:do ((i 0)) (< i 4) ((+ i 1))) i) '(0 1 2 3))
(check do-two-variables
       (list-ec (:do ((i 0) (j 10)) (< i 3) ((+ i 1) (- j 1))) (list i j))
       '((0 10) (1 9) (2 8)))

(check port-generator
       (list-ec (:port x (open-input-string "1 2 3")) x) '(1 2 3))
(check port-with-a-reader
       (list-ec (:port c (open-input-string "abc") read-char) c) '(#\a #\b #\c))

;; ── the dispatching `:` ──────────────────────────────────────────────
(check dispatch-list (list-ec (: x '(1 2 3)) x) '(1 2 3))
(check dispatch-string (list-ec (: c "abc") c) '(#\a #\b #\c))
(check dispatch-vector (list-ec (: x (vector 7 8)) x) '(7 8))
(check dispatch-one-integer (list-ec (: i 3) i) '(0 1 2))
(check dispatch-two-integers (list-ec (: i 2 5) i) '(2 3 4))
(check dispatch-three-integers (list-ec (: i 0 10 4) i) '(0 4 8))
(check dispatch-reals (list-ec (: x 0.0 1.0 0.5) x) '(0.0 0.5))
(check dispatch-chars (list-ec (: c #\a #\c) c) '(#\a #\b #\c))
(check dispatch-port (list-ec (: x (open-input-string "4 5")) x) '(4 5))
(check dispatch-port-with-reader
       (list-ec (: c (open-input-string "hi") read-char) c) '(#\h #\i))
(check dispatch-rejects-the-unknown
       (guard (e (#t 'raised)) (list-ec (: x 'nonsense) x)) 'raised)
;; Several lists concatenate, as the typed generator does.
(check dispatch-two-lists (list-ec (: x '(1) '(2 3)) x) '(1 2 3))

;; ── index variables ──────────────────────────────────────────────────
(check index-on-a-list
       (list-ec (:list (x (index i)) '(a b c)) (list i x))
       '((0 a) (1 b) (2 c)))
(check index-on-a-range
       (list-ec (:range (v (index i)) 10 13) (list i v))
       '((0 10) (1 11) (2 12)))
(check index-on-dispatch
       (list-ec (: (c (index i)) "ab") (list i c)) '((0 #\a) (1 #\b)))
(check index-on-a-string
       (list-ec (:string (c (index i)) "xy") (list i c)) '((0 #\x) (1 #\y)))
;; The index counts that generator's own iterations, not the total.
(check index-is-per-generator
       (list-ec (: a 2) (:list (b (index i)) '(p q)) (list a i b))
       '((0 0 p) (0 1 q) (1 0 p) (1 1 q)))

;; ── filters and side effects ─────────────────────────────────────────
(check filter-if (list-ec (: i 10) (if (even? i)) i) '(0 2 4 6 8))
(check filter-not (list-ec (: i 6) (not (even? i)) i) '(1 3 5))
(check filter-and (list-ec (: i 10) (and (even? i) (> i 4)) i) '(6 8))
(check filter-or (list-ec (: i 6) (or (= i 0) (> i 4)) i) '(0 5))
;; `begin` runs between iterations rather than per value.
(check qualifier-begin
       (let ((seen '()))
         (list-ec (: i 3) (begin (set! seen (cons i seen))) i)
         (reverse seen))
       '(0 1 2))
;; `nested` is only grouping.
(check qualifier-nested
       (list-ec (nested (: i 2) (: j 2)) (list i j))
       '((0 0) (0 1) (1 0) (1 1)))
(check filters-compose
       (list-ec (: i 20) (if (even? i)) (if (> i 10)) i) '(12 14 16 18))

;; ── :parallel, which nesting cannot express ──────────────────────────
(check parallel-lockstep
       (list-ec (:parallel (: i 10) (:list x '(a b c))) (list i x))
       '((0 a) (1 b) (2 c)))
;; It stops at the shortest, where nesting would give the cross product.
(check parallel-stops-at-the-shortest
       (list-ec (:parallel (: i 2) (: j 100)) (list i j)) '((0 0) (1 1)))
(check parallel-three
       (list-ec (:parallel (: i 2) (:list x '(a b)) (:string c "xy"))
                (list i x c))
       '((0 a #\x) (1 b #\y)))
(check parallel-of-one (list-ec (:parallel (: i 3)) i) '(0 1 2))
(check parallel-with-an-index
       (list-ec (:parallel (:list (x (index i)) '(p q)) (: n 10 12)) (list i x n))
       '((0 p 10) (1 q 11)))

;; ── :while and :until ────────────────────────────────────────────────
(check while-stops-early (list-ec (:while (: i 10) (< i 3)) i) '(0 1 2))
;; `:until` stops AFTER the iteration where the test holds.
(check until-includes-the-last (list-ec (:until (: i 10) (>= i 3)) i) '(0 1 2 3))
;; Each escapes only its own loop, so the outer generator keeps going.
(check while-is-per-loop
       (list-ec (: n 3) (:while (: i 10) (< i n)) (list n i))
       '((1 0) (2 0) (2 1)))
(check until-is-per-loop
       (list-ec (: n 2) (:until (: i 10) (>= i 1)) (list n i))
       '((0 0) (0 1) (1 0) (1 1)))
(check while-that-never-runs (list-ec (:while (: i 10) #f) i) '())

;; ── all seventeen comprehensions ─────────────────────────────────────
(check do-ec-is-for-effect
       (let ((n 0)) (do-ec (: i 5) (set! n (+ n i))) n) 10)
(check do-ec-with-no-qualifiers (let ((n 0)) (do-ec (set! n 7)) n) 7)
(check list-ec- (list-ec (: i 3) i) '(0 1 2))
(check append-ec- (append-ec (: i 3) (list i i)) '(0 0 1 1 2 2))
(check append-ec-empty (append-ec (: i 0) (list i)) '())
(check string-ec- (string-ec (: c "abc") (char-upcase c)) "ABC")
(check string-append-ec- (string-append-ec (: i 3) (number->string i)) "012")
(check vector-ec- (vector-ec (: i 3) (* i 2)) #(0 2 4))
(check vector-of-length-ec- (vector-of-length-ec 3 (: i 3) i) #(0 1 2))
(check vector-of-length-rejects-too-many
       (guard (e (#t 'raised)) (vector-of-length-ec 2 (: i 5) i)) 'raised)
(check vector-of-length-rejects-too-few
       (guard (e (#t 'raised)) (vector-of-length-ec 5 (: i 2) i)) 'raised)
(check sum-ec- (sum-ec (: i 5) i) 10)
(check sum-ec-empty (sum-ec (: i 0) i) 0)
(check product-ec- (product-ec (: i 1 5) i) 24)
(check product-ec-empty (product-ec (: i 0) i) 1)
(check min-ec- (min-ec (: x '(3 1 2)) x) 1)
(check max-ec- (max-ec (: x '(3 1 2)) x) 3)
;; No value means no answer, rather than an infinity that does not exist
;; here as an exact number.
(check min-ec-empty-raises (guard (e (#t 'raised)) (min-ec (: i 0) i)) 'raised)
(check max-ec-empty-raises (guard (e (#t 'raised)) (max-ec (: i 0) i)) 'raised)
(check any?-ec- (any?-ec (: i 10) (> i 8)) #t)
(check any?-ec-no (any?-ec (: i 10) (> i 100)) #f)
(check any?-ec-empty (any?-ec (: i 0) #t) #f)
(check every?-ec- (every?-ec (: i 10) (< i 100)) #t)
(check every?-ec-no (every?-ec (: i 10) (< i 5)) #f)
(check every?-ec-empty (every?-ec (: i 0) #f) #t)
(check first-ec- (first-ec 'none (: i 10) (if (> i 5)) i) 6)
(check first-ec-default (first-ec 'none (: i 0) i) 'none)
(check last-ec- (last-ec 'none (: i 5) i) 4)
(check last-ec-default (last-ec 'none (: i 0) i) 'none)
(check fold-ec- (fold-ec 0 (: i 5) i +) 10)
(check fold-ec-order (fold-ec '() (: i 3) i cons) '(2 1 0))
(check fold3-ec- (fold3-ec 'empty (: i 5) i (lambda (x) x) +) 10)
(check fold3-ec-empty (fold3-ec 'empty (: i 0) i (lambda (x) x) +) 'empty)
;; `f1` makes the seed from the first value, so this is a max without one.
(check fold3-ec-uses-f1
       (fold3-ec 'empty (: x '(3 1 4)) x (lambda (x) (list x)) cons)
       '(4 1 3))

;; ── early exit really is early ───────────────────────────────────────
;; `any?-ec` and `first-ec` must stop, not merely report. Counting the
;; iterations is the only way to tell.
(check any?-ec-stops-early
       (let ((n 0))
         (any?-ec (: i 100) (begin (set! n (+ n 1)) (> i 2)))
         n)
       4)
(check first-ec-stops-early
       (let ((n 0))
         (first-ec 'none (: i 100) (begin (set! n (+ n 1)) i))
         n)
       1)
(check every?-ec-stops-early
       (let ((n 0))
         (every?-ec (: i 100) (begin (set! n (+ n 1)) (< i 2)))
         n)
       3)
;; And an unbounded generator proves it, since this could not terminate
;; otherwise.
(check early-exit-tames-an-infinite-generator
       (first-ec 'none (:integers i) (if (> i 50)) i) 51)

;; ── the dispatcher is extensible ─────────────────────────────────────
;; `:-dispatch-ref` and `:-dispatch-set!` are spelled without the leading
;; colon here, because a keyword cannot be an operator (see the library).
(check dispatch-ref-is-a-procedure (procedure? (dispatch-ref)) #t)
(check a-custom-dispatcher
       (let* ((old (dispatch-ref))
              (mine (lambda (args)
                      (if (and (= 1 (length args)) (symbol? (car args)))
                          (let ((cs (string->list (symbol->string (car args)))))
                            (lambda ()
                              (if (pair? cs)
                                  (let ((c (car cs))) (set! cs (cdr cs)) c)
                                  (eof-object))))
                          #f))))
         (dispatch-set! (dispatch-union mine old))
         (let ((r (list (list-ec (: c 'abc) c) (list-ec (: i 3) i))))
           (dispatch-set! old)
           r))
       '((#\a #\b #\c) (0 1 2)))
(check dispatch-set-returns-the-old
       (let* ((old (dispatch-ref))
              (back (dispatch-set! old)))
         (eq? old back))
       #t)
(check initial-dispatch-is-fresh
       (let ((d (make-initial-:-dispatch)))
         (list (procedure? d) (not (eq? d (dispatch-ref)))))
       '(#t #t))

;; ── scm's own two decisions still hold ───────────────────────────────
;; A keyword is still a keyword: nothing here bound `:list`.
(check keywords-are-untouched (list (keyword? ':list) (symbol? ':list)) '(#t #f))
;; And a bracket in an expression position is still a vector.
(check brackets-still-work (list-ec (:vector x [1 2 3]) x) '(1 2 3))

(test-failures)
