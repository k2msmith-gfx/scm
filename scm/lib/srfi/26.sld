;;; (srfi 26) — CUT and CUTE: notation for specialising parameters.
;;;
;;; `(cut + 1 <>)` is `(lambda (x) (+ 1 x))`; `<...>` takes the rest. `cut`
;;; evaluates the non-slot expressions each time the procedure is called,
;;; `cute` once when the procedure is made.
;;;
;;; Both accumulate through a helper macro, and both depend on hygiene for
;;; their introduced names: every recursive expansion introduces another
;;; `x` (or `tmp`), and they must be distinct bindings. They are, because
;;; `syntax-rules` renames a template identifier per expansion -- which
;;; makes this library a test of that as much as a use of it.
(define-library (srfi 26)
  (export cut cute)
  (import (scheme base))
  (begin
    (define-syntax %cut
      (syntax-rules (<> <...>)
        ((_ (slot ...) (pos ...))
         (lambda (slot ...) (pos ...)))
        ((_ (slot ...) (pos ...) <> . rest)
         (%cut (slot ... x) (pos ... x) . rest))
        ((_ (slot ...) (pos ...) <...>)
         (lambda (slot ... . rest-slot) (apply pos ... rest-slot)))
        ((_ (slot ...) (pos ...) e . rest)
         (%cut (slot ...) (pos ... e) . rest))))

    (define-syntax cut
      (syntax-rules ()
        ((_ . slots) (%cut () () . slots))))

    (define-syntax %cute
      (syntax-rules (<> <...>)
        ((_ (slot ...) (pos ...) (binding ...))
         (let (binding ...) (lambda (slot ...) (pos ...))))
        ((_ (slot ...) (pos ...) (binding ...) <> . rest)
         (%cute (slot ... x) (pos ... x) (binding ...) . rest))
        ((_ (slot ...) (pos ...) (binding ...) <...>)
         (let (binding ...)
           (lambda (slot ... . rest-slot) (apply pos ... rest-slot))))
        ((_ (slot ...) (pos ...) (binding ...) e . rest)
         (%cute (slot ...) (pos ... tmp) (binding ... (tmp e)) . rest))))

    (define-syntax cute
      (syntax-rules ()
        ((_ . slots) (%cute () () () . slots))))))
