;;; (srfi 204) — the Wright-Cartwright-Shinn pattern matcher.
(import (scm test) (srfi 204))

;; ── the leaves ───────────────────────────────────────────────────────
(check patvar (match 5 (x x)) 5)
(check wildcard (match 5 (_ 'any)) 'any)
;; The wildcard binds nothing, which is the half that a literals-list `_`
;; silently got wrong while this library was being written.
(check wildcard-binds-nothing
       (let ((_ 'outer)) (match 5 (_ _)))
       'outer)
(check number-literal (match 2 (1 'one) (2 'two)) 'two)
(check string-literal (match "s" ("t" 'no) ("s" 'yes)) 'yes)
(check char-literal (match #\a (#\b 'no) (#\a 'yes)) 'yes)
(check boolean-literal (match #f (#t 'no) (#f 'yes)) 'yes)
(check empty-list (match '() (() 'empty) (_ 'other)) 'empty)
(check quoted-symbol (match 'a ('b 'no) ('a 'yes)) 'yes)
(check quoted-datum (match '(1 (2)) ('(1 (2)) 'yes) (_ 'no)) 'yes)
;; A keyword is a literal here, not a variable: scm's own addition, and the
;; one leaf kind the SRFI's grammar does not list.
(check keyword-literal (match ':at (':at 'yes) (_ 'no)) 'yes)

;; ── lists ────────────────────────────────────────────────────────────
(check list-two (match '(1 2) ((a b) (list b a))) '(2 1))
(check list-nested (match '(1 (2 3)) ((a (b c)) (list a b c))) '(1 2 3))
(check improper (match (cons 1 2) ((a . b) (list a b))) '(1 2))
(check rest (match '(1 2 3) ((a . rest) (list a rest))) '(1 (2 3)))
(check length-mismatch (match '(1 2 3) ((a b) 'two) ((a b c) 'three)) 'three)
(check list-is-not-an-atom (match 5 ((a b) 'list) (_ 'atom)) 'atom)

;; ── ellipsis ─────────────────────────────────────────────────────────
;; The SRFI's own example, verbatim.
(check spec-example
       (match '((a time) (stitch saves) (in nine)) (((x y) ...) (list x y)))
       '((a stitch in) (time saves nine)))
(check ellipsis-all (match '(1 2 3) ((a ...) a)) '(1 2 3))
(check ellipsis-none (match '() ((a ...) a)) '())
(check ellipsis-after-fixed (match '(1 2 3) ((a b ...) (list a b))) '(1 (2 3)))
;; Fixed patterns AFTER the ellipsis: the split is decided by their count,
;; which is what makes this unambiguous without backtracking.
(check ellipsis-before-fixed
       (match '(1 2 3 4) ((a ... b c) (list a b c)))
       '((1 2) 3 4))
(check ellipsis-too-short
       (match '(1) ((a ... b c) 'matched) (_ 'no))
       'no)
(check ellipsis-nested (match '((1 2) (3 4)) (((a ...) ...) a)) '((1 2) (3 4)))
(check ellipsis-binds-nothing (match '(1 2) ((_ ...) 'any)) 'any)
(check ellipsis-alt-spelling (match '(1 2) ((a ___) a)) '(1 2))
(check ellipsis-over-pairs
       (match '((1 . a) (2 . b)) (((k . v) ...) (list k v)))
       '((1 2) (a b)))
(check ellipsis-literal-element
       (match '(1 1 1) ((1 ...) 'all-ones) (_ 'no))
       'all-ones)
(check ellipsis-literal-element-fails
       (match '(1 2) ((1 ...) 'all-ones) (_ 'no))
       'no)
(check ellipsis-on-an-improper-list
       (match (cons 1 2) ((a ...) 'matched) (_ 'no))
       'no)

;; ── vectors ──────────────────────────────────────────────────────────
;; These are what found the expander gap: a `#(…)` PATTERN in syntax-rules
;; matched nothing at all before, so none of these could have passed.
(check vector-fixed (match #(1 2) (#(a b) (list b a))) '(2 1))
(check vector-ellipsis (match #(1 2 3) (#(a ...) a)) '(1 2 3))
(check vector-mixed (match #(1 2 3) (#(a b ...) (list a b))) '(1 (2 3)))
(check vector-length-mismatch
       (match #(1 2 3) (#(a b) 'two) (#(a b c) 'three))
       'three)
(check vector-is-not-a-list (match '(1 2) (#(a b) 'vector) (_ 'list)) 'list)
(check list-is-not-a-vector (match #(1 2) ((a b) 'list) (_ 'vector)) 'vector)
(check vector-nested (match #(#(1 2)) (#(#(a b)) (list a b))) '(1 2))

;; ── the operators ────────────────────────────────────────────────────
(check predicate (match 5 ((? number? n) (* n 2))) 10)
(check predicate-fails (match "s" ((? number? n) 'num) (_ 'other)) 'other)
(check predicate-bare (match 5 ((? number?) 'num)) 'num)
(check predicate-extra-patterns
       (match '(1 2) ((? pair? (a b)) (list b a)))
       '(2 1))
(check accessor (match '(1 2 3) ((= length n) n)) 3)
(check accessor-then-pattern (match '(1 2) ((= length 2) 'two) (_ 'no)) 'two)
(check and-all (match 5 ((and (? number?) x) x)) 5)
(check and-empty (match 5 ((and) 'yes)) 'yes)
(check and-fails (match "s" ((and (? number?) x) 'num) (_ 'other)) 'other)
(check or-first (match 'a ((or 'a 'b) 'found) (_ 'no)) 'found)
(check or-second (match 'b ((or 'a 'b) 'found) (_ 'no)) 'found)
(check or-none (match 'c ((or 'a 'b) 'found) (_ 'no)) 'no)
(check or-binds (match 2 ((or 1 x) x)) 2)
(check not-pattern (match 5 ((not (? string?)) 'notstring) (_ 'no)) 'notstring)
(check not-pattern-fails (match "s" ((not (? string?)) 'notstring) (_ 'no)) 'no)

;; ── quasipatterns ────────────────────────────────────────────────────
(check quasi-unquote (match '(1 x 3) (`(1 ,v 3) v)) 'x)
(check quasi-literal-symbol (match '(a b) (`(a b) 'yes) (_ 'no)) 'yes)
(check quasi-nested (match '(1 (2 3)) (`(1 (,a ,b)) (list a b))) '(2 3))
(check quasi-fails (match '(1 2) (`(1 3) 'yes) (_ 'no)) 'no)
(check quasi-vector (match #(1 2) (`#(1 ,b) b)) 2)

;; ── the clause forms ─────────────────────────────────────────────────
(check first-matching-clause-wins
       (match 1 (x 'first) (1 'second))
       'first)
(check no-clause-raises
       (guard (e (#t 'raised)) (match 1 (2 'two)))
       'raised)
;; `(=> id)` names the thunk that restarts matching at the next clause, so a
;; body can decide after the fact that the match was wrong. The SRFI's own
;; example.
(check failure-restart
       (match #f ((and x) (=> fail) (if x #t (fail))) (_ 'fell-through))
       'fell-through)
(check failure-not-called
       (match 7 ((and x) (=> fail) (if x x (fail))) (_ 'no))
       7)

;; ── the derived forms ────────────────────────────────────────────────
(check match-lambda ((match-lambda ((a b) (+ a b))) '(1 2)) 3)
(check match-lambda-clauses
       (map (match-lambda ((a b) (+ a b)) ((a) a)) '((1 2) (7)))
       '(3 7))
(check match-lambda* ((match-lambda* ((a b) (+ a b))) 1 2) 3)
(check match-let (match-let (((a b) '(1 2)) ((c) '(3))) (list a b c)) '(1 2 3))
;; `match-let` binds in PARALLEL, like `let`: the right-hand sides are
;; evaluated before any pattern binds, so the inner `x` here is 1 and not 9.
(check match-let-is-parallel
       (let ((x 1)) (match-let (((a) (list x)) ((x) '(9))) (list a x)))
       '(1 9))
(check match-let* (match-let* (((a) '(1)) ((b) (list (+ a 1)))) (list a b)) '(1 2))
(check match-let-named
       (match-let loop (((n acc) '(4 1)))
         (if (= n 0) acc (loop (list (- n 1) (* acc n)))))
       24)
(check match-letrec (match-letrec (((a) '(1))) a) 1)

;; ── what a ray scene actually needs it for ───────────────────────────
;; The shape that motivated the library: ray's Janet destructures a position
;; or an option pair about a thousand times across its sources.
(check destructure-a-vec3
       (match #(1.5 2.4 14.5) (#(x y z) (list z y x)))
       '(14.5 2.4 1.5))
(check destructure-wave-specs
       (map (match-lambda ((dx dz lam amp ph) (list lam amp)))
            '((0.80 0.60 3.4 0.068 0.0) (-0.55 0.83 1.9 0.044 1.7)))
       '((3.4 0.068) (1.9 0.044)))
(check destructure-an-alist
       (match '((:albedo . 0.8) (:shader . :lambertian))
         (((k . v) ...) (list k v)))
       '((:albedo :shader) (0.8 :lambertian)))

;; ── scm's own two decisions still hold ───────────────────────────────
(check keywords-are-untouched (list (keyword? ':at) (symbol? ':at)) '(#t #f))
(check brackets-still-work (match (let ([x 1]) [x 2]) (#(a b) (list a b))) '(1 2))

(test-failures)
