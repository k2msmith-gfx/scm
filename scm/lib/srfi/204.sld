;;; (srfi 204) — the Wright-Cartwright-Shinn pattern matcher.
;;;
;;; Written against SRFI 204's grammar. The SRFI itself was **withdrawn**
;;; (2022-02-02), which means its document will not change again — it does not
;;; mean the matcher is unusual: this is `match` as Chibi, Gauche and Guile's
;;; `(ice-9 match)` ship it and as Racket's own `match` is shaped, so it is
;;; the destructuring form a Scheme or Racket reader already knows. See
;;; docs/srfi.md for what is implemented and what is not.
;;;
;;; Two expander facts this rests on, both established by running them rather
;;; than assuming:
;;;
;;;   * an identifier can be told from a literal inside `syntax-rules`, by
;;;     substituting it into a nested rule's pattern: an identifier becomes a
;;;     pattern VARIABLE there and matches the probe, a number or string
;;;     becomes a literal and does not. `%if-id` below.
;;;   * `(syntax-rules <ellipsis> (<literal> …) …)` renames the ellipsis, which
;;;     is what makes `...` itself available as a literal to match. Every macro
;;;     here that has to recognise an ellipsis IN a pattern uses `eee` as its
;;;     own ellipsis for that reason.
(define-library (srfi 204)
  (export match match-lambda match-lambda* match-let match-let* match-letrec)
  (import (scheme base))
  (begin

    ;; ── is this an identifier? ──────────────────────────────────────────
    ;; `%id-probe` is a symbol the caller cannot have written as a literal
    ;; pattern leaf, so the two cases cannot be confused. If x happens to BE
    ;; `%id-probe`, it is an identifier, and the answer is still right.
    (define-syntax %if-id
      (syntax-rules ()
        ((_ x kt kf)
         (let-syntax ((probe (syntax-rules ()
                               ((_ x k1 k2) k1)
                               ((_ other k1 k2) k2))))
           (probe %id-probe kt kf)))))

    ;; Is this identifier the wildcard? It cannot be asked with `_` in a
    ;; literals list: scm matches `_` in a pattern as a WILDCARD even when it
    ;; is declared a literal, so a rule written `((_ v _ sk fk) sk)` quietly
    ;; matches every leaf and binds nothing. That cost this library its first
    ;; version. Asked the other way round — x as the literal, `_` as the
    ;; input — it answers correctly. x must already be known to be an
    ;; identifier, or the literals list would be malformed.
    (define-syntax %if-wild
      (syntax-rules ()
        ((_ x kt kf)
         (let-syntax ((probe (syntax-rules (x)
                               ((_ x k1 k2) k1)
                               ((_ other k1 k2) k2))))
           (probe _ kt kf)))))

    ;; ── the pattern variables of a pattern ──────────────────────────────
    ;; Continuation-passing, because a macro cannot return: `(%vars pat (k
    ;; arg …) acc)` ends in `(k arg … vars)`. Needed only for the ellipsis
    ;; case, which has to name what it collects.

    (define-syntax %call
      (syntax-rules ()
        ((_ (k a ...) acc) (k a ... acc))))

    (define-syntax %vars-then
      (syntax-rules ()
        ((_ q k acc) (%vars q k acc))))

    (define-syntax %vars-list-then
      (syntax-rules ()
        ((_ ps k acc) (%vars-list ps k acc))))

    (define-syntax %vars-list
      (syntax-rules ()
        ((_ () k acc) (%call k acc))
        ((_ (p rest ...) k acc) (%vars p (%vars-list-then (rest ...) k) acc))))

    (define-syntax %vars
      (syntax-rules eee (quote quasiquote unquote and or not ? = ... ___)
        ((_ (quote d) (k a eee) acc) (k a eee acc))
        ((_ (and p eee) k acc) (%vars-list (p eee) k acc))
        ((_ (or p eee) k acc) (%vars-list (p eee) k acc))
        ((_ (not p eee) k acc) (%vars-list (p eee) k acc))
        ((_ (? pred p eee) k acc) (%vars-list (p eee) k acc))
        ((_ (= proc p) k acc) (%vars p k acc))
        ((_ (quasiquote qp) k acc) (%qvars qp k acc))
        ;; An ellipsis is punctuation, not a variable.
        ((_ (... . rest) k acc) (%vars rest k acc))
        ((_ (___ . rest) k acc) (%vars rest k acc))
        ((_ #(p eee) k acc) (%vars-list (p eee) k acc))
        ((_ (p . q) k acc) (%vars p (%vars-then q k) acc))
        ((_ leaf (k a eee) acc)
         (%if-id leaf
                 (%if-wild leaf (k a eee acc) (k a eee (leaf . acc)))
                 (k a eee acc)))))

    (define-syntax %qvars-then
      (syntax-rules ()
        ((_ q k acc) (%qvars q k acc))))

    (define-syntax %qvars
      (syntax-rules eee (unquote)
        ((_ (unquote p) k acc) (%vars p k acc))
        ((_ (a . b) k acc) (%qvars a (%qvars-then b k) acc))
        ((_ #(x eee) k acc) (%qvars-list (x eee) k acc))
        ((_ other (k a eee) acc) (k a eee acc))))

    (define-syntax %qvars-list-then
      (syntax-rules ()
        ((_ ps k acc) (%qvars-list ps k acc))))

    (define-syntax %qvars-list
      (syntax-rules ()
        ((_ () k acc) (%call k acc))
        ((_ (p rest ...) k acc)
         (%qvars p (%qvars-list-then (rest ...) k) acc))))

    ;; ── the two runtime helpers ─────────────────────────────────────────

    ;; All but the last N elements, and the last N. `#f` for anything that is
    ;; not a list of at least N elements, which is how the match fails.
    (define (%split lst n)
      (let loop ((xs lst) (len 0))
        (cond ((pair? xs) (loop (cdr xs) (+ len 1)))
              ((not (null? xs)) #f)         ; improper: no match
              ((< len n) #f)
              (else
               (let take ((i (- len n)) (xs lst) (acc '()))
                 (if (= i 0)
                     (cons (reverse acc) xs)
                     (take (- i 1) (cdr xs) (cons (car xs) acc))))))))

    ;; Tuples of K bindings, one per matched element, become K lists of
    ;; bindings — which is what the body of an ellipsis pattern sees.
    (define (%columns tuples k)
      (if (null? tuples)
          (make-list k '())
          (apply map list tuples)))

    ;; ── the matcher ─────────────────────────────────────────────────────
    ;; `(%m v pat sk fk)`: sk and fk are expressions, and fk is always a call
    ;; to a thunk the clause bound, so duplicating it costs nothing.

    (define-syntax %mell
      (syntax-rules ()
        ((_ v p rest sk fk) (%vars p (%mell2 v p rest sk fk) ()))))

    (define-syntax %mell2
      (syntax-rules ()
        ((_ v p rest sk fk (var ...))
         (let ((parts (%split v (length (quote rest)))))
           (if parts
               (let scan ((xs (car parts)) (tuples '()))
                 (if (null? xs)
                     (let ((cols (%columns (reverse tuples)
                                           (length (quote (var ...))))))
                       (%m cols (var ...) (%m (cdr parts) rest sk fk) fk))
                     (%m (car xs) p
                         (scan (cdr xs) (cons (list var ...) tuples))
                         fk)))
               fk)))))

    (define-syntax %m
      (syntax-rules eee (quote quasiquote unquote and or not ? = ... ___)
        ((_ v (quote d) sk fk) (if (equal? v (quote d)) sk fk))
        ((_ v (and) sk fk) sk)
        ((_ v (and p rest eee) sk fk) (%m v p (%m v (and rest eee) sk fk) fk))
        ((_ v (or) sk fk) fk)
        ((_ v (or p rest eee) sk fk)
         (let ((next (lambda () (%m v (or rest eee) sk fk))))
           (%m v p sk (next))))
        ((_ v (not p eee) sk fk) (%m v (and p eee) fk sk))
        ((_ v (? pred p eee) sk fk) (if (pred v) (%m v (and p eee) sk fk) fk))
        ((_ v (= proc p) sk fk) (let ((got (proc v))) (%m got p sk fk)))
        ((_ v (quasiquote qp) sk fk) (%mq v qp sk fk))
        ;; One ellipsis per list, with any number of fixed patterns after it.
        ((_ v (p ... . rest) sk fk) (%mell v p rest sk fk))
        ((_ v (p ___ . rest) sk fk) (%mell v p rest sk fk))
        ((_ v #(p eee) sk fk)
         (if (vector? v)
             (let ((as-list (vector->list v))) (%m as-list (p eee) sk fk))
             fk))
        ((_ v () sk fk) (if (null? v) sk fk))
        ((_ v (p . q) sk fk)
         (if (pair? v)
             (let ((head (car v)) (tail (cdr v)))
               (%m head p (%m tail q sk fk) fk))
             fk))
        ((_ v leaf sk fk)
         (%if-id leaf
                 (%if-wild leaf sk (let ((leaf v)) sk))
                 (if (equal? v leaf) sk fk)))))

    ;; A quasipattern is data except where it is unquoted.
    (define-syntax %mq
      (syntax-rules eee (unquote)
        ((_ v (unquote p) sk fk) (%m v p sk fk))
        ((_ v (a . b) sk fk)
         (if (pair? v)
             (let ((head (car v)) (tail (cdr v)))
               (%mq head a (%mq tail b sk fk) fk))
             fk))
        ((_ v #(x eee) sk fk)
         (if (vector? v)
             (let ((as-list (vector->list v))) (%mq as-list (x eee) sk fk))
             fk))
        ((_ v d sk fk) (if (equal? v (quote d)) sk fk))))

    ;; ── the forms a user writes ─────────────────────────────────────────

    (define-syntax %clauses
      (syntax-rules (=>)
        ((_ v) (error "match: no matching clause" v))
        ;; `(=> fail)` names the thunk that restarts matching at the next
        ;; clause, so a body that discovers the match was wrong can say so.
        ((_ v (pat (=> fail) body ...) rest ...)
         (let ((fail (lambda () (%clauses v rest ...))))
           (%m v pat (begin body ...) (fail))))
        ((_ v (pat body ...) rest ...)
         (let ((next (lambda () (%clauses v rest ...))))
           (%m v pat (begin body ...) (next))))))

    (define-syntax match
      (syntax-rules ()
        ((_ e clause ...) (let ((v e)) (%clauses v clause ...)))))

    (define-syntax match-lambda
      (syntax-rules ()
        ((_ clause ...) (lambda (x) (match x clause ...)))))

    (define-syntax match-lambda*
      (syntax-rules ()
        ((_ clause ...) (lambda args (match args clause ...)))))

    ;; The list form binds in PARALLEL — every expression is evaluated, then
    ;; the patterns see the values — which is `let`'s rule, and is why this is
    ;; one match against a list rather than a fold of match-let*.
    ;; The named form is `let`'s other one: a loop whose parameters are
    ;; patterns.
    (define-syntax match-let
      (syntax-rules ()
        ((_ ((pat expr) ...) body ...)
         (match (list expr ...) ((pat ...) body ...)))
        ((_ loop ((pat expr) ...) body ...)
         (letrec ((loop (match-lambda* ((pat ...) body ...))))
           (loop expr ...)))))

    (define-syntax match-let*
      (syntax-rules ()
        ((_ () body ...) (let () body ...))
        ((_ ((pat expr) rest ...) body ...)
         (match expr (pat (match-let* (rest ...) body ...))))))

    ;; Sequential, not recursive: see docs/srfi.md. A pattern here cannot
    ;; refer to a binding made later in the same form.
    (define-syntax match-letrec
      (syntax-rules ()
        ((_ bindings body ...) (match-let* bindings body ...))))))
