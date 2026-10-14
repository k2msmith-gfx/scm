;;; (srfi 8) — RECEIVE: binding multiple values.
(define-library (srfi 8)
  (export receive)
  (import (scheme base))
  (begin
    (define-syntax receive
      (syntax-rules ()
        ((_ formals expr body ...)
         (call-with-values (lambda () expr) (lambda formals body ...)))))))
