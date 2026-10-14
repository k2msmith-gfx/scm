;;; (srfi 2) — AND-LET*: an AND with local bindings, a guarded LET*.
(define-library (srfi 2)
  (export and-let*)
  (import (scheme base))
  (begin
    ;; A claw is `(variable expression)`, `(expression)`, or a bound
    ;; variable. The order of the rules is the disambiguation: a two-element
    ;; list binds, a one-element list only tests, a bare identifier tests
    ;; what it is already bound to. With no body, the value is the last
    ;; claw's.
    (define-syntax and-let*
      (syntax-rules ()
        ((_ ()) #t)
        ((_ () body body* ...) (begin body body* ...))
        ;; "The value is that of the last expression evaluated", so with an
        ;; empty body the last claw's own value comes back rather than `#t`.
        ;; These three must precede the general rules, which would match
        ;; with their `rest ...` and `body ...` both empty.
        ((_ ((var expr))) expr)
        ((_ ((expr))) expr)
        ((_ (var)) var)
        ((_ ((var expr) rest ...) body ...)
         (let ((var expr)) (and var (and-let* (rest ...) body ...))))
        ((_ ((expr) rest ...) body ...)
         (and expr (and-let* (rest ...) body ...)))
        ((_ (var rest ...) body ...)
         (and var (and-let* (rest ...) body ...)))))))
