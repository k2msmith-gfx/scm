;;; (scm test) — the smallest thing that can gate a library.
;;;
;;; A test file imports this, calls `check` with a name, an expression's
;;; value and the expected value, and ends with `(test-failures)`, which is
;;; `()` when everything passed and a list of reports when it did not. The
;;; Rust gate asserts `()`; a person running `scm srfi/1-test.scm` reads the
;;; same list. `check` compares with `equal?`.
(define-library (scm test)
  (export check check-raises test-failures test-count)
  (import (scheme base))
  (begin
    (define failures '())
    (define count 0)

    (define (record! name got want)
      (set! failures
            (cons (list name 'got got 'wanted want) failures)))

    (define (check-value name got want)
      (set! count (+ count 1))
      (if (equal? got want) #t (begin (record! name got want) #f)))

    ;; `(check NAME expr want)` — NAME is a datum, not a string, so a
    ;; failure report reads as the form that produced it.
    (define-syntax check
      (syntax-rules ()
        ((_ name expr want)
         (check-value 'name
                      (guard (e (#t (list 'raised (if (error-object? e)
                                                      (error-object-message e)
                                                      e))))
                        expr)
                      want))))

    ;; `(check-raises NAME expr)` — passes when `expr` raises anything.
    (define-syntax check-raises
      (syntax-rules ()
        ((_ name expr)
         (check-value 'name (guard (e (#t 'raised)) (begin expr 'no-raise)) 'raised))))

    (define (test-failures) (reverse failures))
    (define (test-count) count)))
