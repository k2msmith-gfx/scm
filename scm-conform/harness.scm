;;; harness.scm — enough of `(chibi test)` to run r7rs-tests.scm.
;;;
;;; The suite is 2,530 lines written by other people against the spec, with no
;;; knowledge of this implementation. That is the entire value of it, so
;;; nothing here adapts the suite — this file adapts to the suite.
;;;
;;; Every assertion runs inside `%try`, so a test that raises is a recorded
;;; failure rather than the end of the run. The host's own stops (interrupt,
;;; fuel, heap cap) are deliberately NOT catchable, so a runaway test still
;;; stops the runner rather than being swallowed.

(define *passed* 0)
(define *failed* 0)
(define *failures* '())
(define *group* "")

(define (%pass!) (set! *passed* (+ *passed* 1)))

(define (%fail! label why)
  (set! *failed* (+ *failed* 1))
  (set! *failures* (cons (list *group* label why) *failures*)))

;; `(test expected expr)` and `(test name expected expr)`.
;;
;; The *expected* expression is wrapped too: several suite entries compute it,
;; and one that raises would otherwise abort the file rather than fail a test.
(define-syntax test
  (syntax-rules ()
    [(_ expected expr) (%test-1 'expr (%try expected) (%try expr))]
    [(_ name expected expr) (%test-1 name (%try expected) (%try expr))]))

(define (%test-1 label expected actual)
  (cond
    [(not (car expected)) (%fail! label (format "expected-expr raised: ~a" (cdr expected)))]
    [(not (car actual)) (%fail! label (format "raised: ~a" (cdr actual)))]
    [(%same? (cdr expected) (cdr actual)) (%pass!)]
    [else (%fail! label (format "want ~s, got ~s" (cdr expected) (cdr actual)))]))

;; chibi's `test` compares inexact numbers APPROXIMATELY, and the suite relies
;; on it: its expected values are written to about 15 significant digits, so an
;; exact `equal?` fails every trigonometric entry on any implementation. This
;; is the harness adapting to the suite, which is the right direction -- the
;; suite is the oracle and does not get edited.
(define %epsilon 1e-12)

(define (%same? a b)
  (if (and (number? a) (number? b) (or (not (exact? a)) (not (exact? b))))
      (let ([d (abs (- a b))])
        (or (< d %epsilon)
            (< (/ d (max (abs a) (abs b) %epsilon)) 1e-9)))
      (equal? a b)))

(define-syntax test-assert
  (syntax-rules ()
    [(_ expr) (%test-assert 'expr (%try expr))]
    [(_ name expr) (%test-assert name (%try expr))]))

(define (%test-assert label r)
  (cond [(not (car r)) (%fail! label (format "raised: ~a" (cdr r)))]
        [(cdr r) (%pass!)]
        [else (%fail! label "expected a true value")]))

;; `test-error` passes when the expression *does* raise. The condition is now
;; an object rather than a string, so the report formats it with `~a`.
(define-syntax test-error
  (syntax-rules ()
    [(_ expr) (%test-error 'expr (%try expr))]
    [(_ name expr) (%test-error name (%try expr))]
    [(_ name expected expr) (%test-error name (%try expr))]))

(define (%test-error label r)
  (if (car r) (%fail! label "expected an error, got a value") (%pass!)))

;; `test-values` compares the whole value lists, which is what makes a
;; zero-value or three-value producer distinguishable from a one-value one.
(define-syntax test-values
  (syntax-rules ()
    [(_ expected expr) (%test-1 'expr (%try (%vals expected)) (%try (%vals expr)))]
    [(_ name expected expr) (%test-1 name (%try (%vals expected)) (%try (%vals expr)))]))

(define-syntax test-begin
  (syntax-rules ()
    [(_ name) (set! *group* name)]
    [(_ name x ...) (set! *group* name)]))

(define-syntax test-end
  (syntax-rules ()
    [(_) (set! *group* "")]
    [(_ x ...) (set! *group* "")]))

;; The library system is S5's remaining slice. Until then an import is noted
;; and ignored, so the suite's ten imports do not stop it at line 3.
(define *imports* '())
(define-syntax import
  (syntax-rules ()
    [(_ lib ...) (set! *imports* (cons '(lib ...) *imports*))]))

(define (report)
  (list *passed* *failed* (reverse *failures*)))
