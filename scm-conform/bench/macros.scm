;;; What a macro use costs per call, against the code it stands for.
;;;
;;;     cargo build --release
;;;     target/release/scm scm-conform/bench/macros.scm            # T1
;;;     SCM_TIER=t0 target/release/scm scm-conform/bench/macros.scm
;;;
;;; By hand, never in CI: a timing assert would be flaky, so the suite pins
;;; the same thing by counting instead (scm/tests/local_macros.rs). The
;;; numbers and their history are in docs/s29-local-macros.md. Each row is
;;; the best of three runs of N calls, in microseconds per call.
(import (scheme base) (scheme write) (scheme time) (srfi 204))

(define-syntax my-if (syntax-rules () ((_ c a b) (cond (c a) (else b)))))

(define (f-cond x) (cond ((eq? x :a) 1) ((eq? x :b) 2) (else 3)))
(define (f-case x) (case x ((:a) 1) ((:b) 2) (else 3)))
(define (f-macro x) (my-if (eq? x :a) 1 2))
(define (f-letsyn x)
  (let-syntax ((probe (syntax-rules () ((_ y) (eq? y :a)))))
    (if (probe x) 1 2)))
(define (f-match-var x) (match x (y y)))
(define (f-match-2 x) (match x (:a 1) (_ 3)))
(define (f-match-3 x) (match x (:a 1) (:b 2) (_ 3)))
(define (g-ref v) (+ (vector-ref v 0) (vector-ref v 1) (vector-ref v 2) (vector-ref v 3)))
(define (g-match v) (match v (#(a b c d) (+ a b c d))))
(define (h-cdr l) (if (and (pair? l) (pair? (cdr l))) (+ (car l) (cadr l)) 0))
(define (h-match l) (match l ((a b . _) (+ a b)) (_ 0)))

(define n 100000)

(define (per-call f arg)
  (let loop ((run 0) (best #f))
    (if (= run 3)
        best
        (let ((t0 (current-jiffy)))
          (let spin ((i 0)) (when (< i n) (f arg) (spin (+ i 1))))
          (let ((us (/ (* 1e6 (/ (- (current-jiffy) t0) (jiffies-per-second))) n)))
            (loop (+ run 1) (if (or (not best) (< us best)) us best)))))))

(define (show x)
  (let ((r (/ (round (* x 1000)) 1000))) (display r)))

(define (row name f arg)
  (display "| ") (display name) (display " | ") (show (per-call f arg)) (display " |") (newline))

(display "| case | us/call |") (newline)
(display "|---|---|") (newline)
(row "`cond` on a keyword" f-cond :b)
(row "`case` on a keyword" f-case :b)
(row "a global syntax-rules macro" f-macro :b)
(row "one `let-syntax` in the body" f-letsyn :b)
(row "`(match x (y y))`" f-match-var :b)
(row "`(match x (:a 1) (_ 3))`" f-match-2 :b)
(row "`(match x (:a 1) (:b 2) (_ 3))`" f-match-3 :b)
(row "four `vector-ref`s" g-ref #(1 2 3 4))
(row "`(match v (#(a b c d) …))`" g-match #(1 2 3 4))
(row "`car`/`cadr` with checks" h-cdr '(1 2 3))
(row "`(match l ((a b . _) …) (_ 0))`" h-match '(1 2 3))
