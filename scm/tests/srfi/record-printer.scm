;;; `set-record-type-printer!` (S42), Guile's extension and spelling: a
;;; record type's printer, `(lambda (record port) ...)`, is what `write`,
;;; `display`, `write-shared` and `write-simple` print for its records,
;;; wherever in a value they are, where they printed `#<type>`.
(import (scm test) (scheme base) (scheme write))

(define-record-type point (make-point x y) point? (x point-x) (y point-y set-point-y!))
(define-record-type plain (make-plain a) plain? (a plain-a))

(define (written x) (let ([p (open-output-string)]) (write x p) (get-output-string p)))
(define (displayed x) (let ([p (open-output-string)]) (display x p) (get-output-string p)))

(check before-a-printer (written (make-point 1 2)) "#<point>")
;; Defined before the first printer, called after it.
(define (shown-early x) (let ([p (open-output-string)]) (display x p) (get-output-string p)))

(set-record-type-printer! point
  (lambda (r port)
    (display "#<point " port) (write (point-x r) port) (display " " port) (write (point-y r) port) (display ">" port)))

(define p (make-point 1 "two"))
(check written (written p) "#<point 1 \"two\">")
(check defined-before-the-printer (shown-early p) "#<point 1 \"two\">")
;; The printer chose write for its fields, so display does not change them.
(check displayed (displayed p) "#<point 1 \"two\">")
(check in-a-list (written (list p 3)) "(#<point 1 \"two\"> 3)")
(check in-a-vector (written (vector 'a p)) "#(a #<point 1 \"two\">)")
(check in-an-improper-list (written (cons 1 p)) "(1 . #<point 1 \"two\">)")
(check twice (written (list p p)) "(#<point 1 \"two\"> #<point 1 \"two\">)")
(check nested (written (make-point (make-point 5 6) 7)) "#<point #<point 5 6> 7>")
(check other-types-as-they-were (written (make-plain 1)) "#<plain>")
;; write-shared labels a shared record as it labels any shared object.
(check write-shared (let ([o (open-output-string)]) (write-shared (list p p) o) (get-output-string o))
       "(#0=#<point 1 \"two\"> #0#)")
(check write-simple (let ([o (open-output-string)]) (write-simple (vector p) o) (get-output-string o))
       "#(#<point 1 \"two\">)")

;; A record that holds itself prints by name inside, not forever.
(define q (make-point 3 #f))
(set-point-y! q q)
(check holds-itself (written q) "#<point 3 #<point>>")
(define r1 (make-point 1 #f))
(define r2 (make-point 2 r1))
(set-point-y! r1 r2)
(check a-cycle-of-two (written r1) "#<point 1 #<point 2 #<point>>>")

;; Setting it again replaces it.
(set-record-type-printer! point (lambda (r port) (display "P" port)))
(check replaced (written (list p q)) "(P P)")

;; A failing printer fails the write, and leaves nothing behind.
(set-record-type-printer! plain (lambda (r port) (error "plain: no")))
(check-raises a-failing-printer (written (make-plain 1)))
(check after-a-failure (written (list 1 "x")) "(1 \"x\")")
(check others-after-a-failure (written p) "P")

;; Output to the current port, and the other printing procedures, unchanged.
(check current-port (let ([o (open-output-string)])
                      (parameterize ([current-output-port o]) (write p) (display " ") (display "s"))
                      (get-output-string o))
       "P s")
(check-raises not-a-procedure (set-record-type-printer! point 5))

(test-failures)
