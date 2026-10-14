;;; (srfi 111) — Boxes: a single-slot mutable container.
(define-library (srfi 111)
  (export box box? unbox set-box!)
  (import (scheme base))
  (begin
    (define-record-type <box> (box value) box? (value unbox set-box!))))
