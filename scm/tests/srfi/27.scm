;;; (srfi 27) sources of random bits.
;;;
;;; The property that matters here is not the distribution but the
;;; REPRODUCIBILITY: a procedural model has to be replayable, so the state
;;; must be savable, restorable and derivable from a seed.
(import (scm test) (srfi 27) (srfi 132) (scheme base))

(define (draws s n)
  (let loop ((i 0) (acc '()))
    (if (= i n) (reverse acc) (loop (+ i 1) (cons (random-integer 1000 s) acc)))))

;; ── sources ──────────────────────────────────────────────────────────
(check source? (random-source? (make-random-source 1)) #t)
(check source?-no (random-source? 1) #f)
(check default-is-a-source (random-source? default-random-source) #t)

;; ── a seed determines the whole sequence ─────────────────────────────
(check same-seed-same-draws
       (equal? (draws (make-random-source 42) 20) (draws (make-random-source 42) 20))
       #t)
(check different-seed-different-draws
       (equal? (draws (make-random-source 42) 20) (draws (make-random-source 43) 20))
       #f)
;; A fresh source with no seed is also fixed, so a program that forgets to
;; seed is still replayable rather than quietly varying per run.
(check no-seed-is-still-deterministic
       (equal? (draws (make-random-source) 20) (draws (make-random-source) 20))
       #t)

;; ── save, draw, restore, draw again ──────────────────────────────────
;; This is the operation procedural modelling actually needs: regenerate
;; one branch without regenerating everything that came before it.
(check state-round-trip
       (let* ((s (make-random-source 7))
              (st (random-source-state-ref s))
              (first (draws s 10)))
         (random-source-state-set! s st)
         (equal? first (draws s 10)))
       #t)
;; The state is a datum, so it can be written and read back.
(check state-is-a-datum
       (let ((st (random-source-state-ref (make-random-source 7))))
         (list (vector? st) (vector-length st) (vector-ref st 0)))
       '(#t 2 splitmix64))
(check state-reads-back
       (let* ((s (make-random-source 7))
              (text (let ((p (open-output-string)))
                      (write (random-source-state-ref s) p)
                      (get-output-string p)))
              (back (read (open-input-string text)))
              (first (draws s 5)))
         (random-source-state-set! s back)
         (equal? first (draws s 5)))
       #t)
;; Restoring a state taken mid-sequence resumes from exactly there.
(check state-mid-sequence
       (let* ((s (make-random-source 7))
              (ignore (draws s 5))
              (st (random-source-state-ref s))
              (next (draws s 5)))
         (random-source-state-set! s st)
         (equal? next (draws s 5)))
       #t)
(check-raises state-set-rejects-a-foreign-state
              (random-source-state-set! (make-random-source 1) (vector 'mt19937 0)))
(check-raises state-set-rejects-a-non-state
              (random-source-state-set! (make-random-source 1) 42))

;; ── substreams ───────────────────────────────────────────────────────
;; The same pair always gives the same stream, which is what lets a tile
;; or a frame own its own stream and stay replayable.
(check pseudo-randomize-is-deterministic
       (let ((a (make-random-source 1)) (b (make-random-source 999)))
         (random-source-pseudo-randomize! a 3 4)
         (random-source-pseudo-randomize! b 3 4)
         (equal? (draws a 10) (draws b 10)))
       #t)
;; Different pairs give different streams, in both coordinates.
(check pseudo-randomize-i-matters
       (let ((a (make-random-source 1)) (b (make-random-source 1)))
         (random-source-pseudo-randomize! a 3 4)
         (random-source-pseudo-randomize! b 5 4)
         (equal? (draws a 10) (draws b 10)))
       #f)
(check pseudo-randomize-j-matters
       (let ((a (make-random-source 1)) (b (make-random-source 1)))
         (random-source-pseudo-randomize! a 3 4)
         (random-source-pseudo-randomize! b 3 6)
         (equal? (draws a 10) (draws b 10)))
       #f)
;; It overrides whatever the source held, seed included.
(check pseudo-randomize-overrides-the-seed
       (let ((a (make-random-source 1)) (b (make-random-source 2)))
         (random-source-pseudo-randomize! a 0 0)
         (random-source-pseudo-randomize! b 0 0)
         (equal? (draws a 10) (draws b 10)))
       #t)

;; `randomize!` is the one operation that gives up reproducibility, so all
;; that can be checked is that it moved the state off the seed.
(check randomize-changes-the-state
       (let* ((s (make-random-source 7))
              (before (random-source-state-ref s)))
         (random-source-randomize! s)
         (equal? before (random-source-state-ref s)))
       #f)

;; ── the generator procedures ─────────────────────────────────────────
(check make-integers
       (let ((g (random-source-make-integers (make-random-source 5))))
         (let loop ((i 0) (ok #t))
           (if (= i 200)
               ok
               (let ((n (g 10)))
                 (loop (+ i 1) (and ok (exact? n) (>= n 0) (< n 10)))))))
       #t)
(check make-integers-is-the-source
       (let ((s (make-random-source 5)))
         (equal? ((random-source-make-integers s) 1000)
                 (begin (random-source-state-set!
                         s (random-source-state-ref (make-random-source 5)))
                        (random-integer 1000 s))))
       #t)
(check make-reals
       (let ((g (random-source-make-reals (make-random-source 5))))
         (let loop ((i 0) (ok #t))
           (if (= i 200)
               ok
               (let ((x (g)))
                 (loop (+ i 1) (and ok (inexact? x) (> x 0.0) (< x 1.0)))))))
       #t)
;; With a unit, every value is a multiple of it and still inside (0,1).
(check make-reals-with-a-unit
       (let ((g (random-source-make-reals (make-random-source 5) 0.25)))
         (let loop ((i 0) (seen '()))
           (if (= i 200)
               (list-sort < seen)
               (let ((x (g)))
                 (loop (+ i 1) (if (member x seen) seen (cons x seen)))))))
       '(0.25 0.5 0.75))
(check-raises make-reals-rejects-a-bad-unit
              (random-source-make-reals (make-random-source 1) 2.0))

;; ── the bare procedures and the parameter ────────────────────────────
(check random-integer-range
       (let loop ((i 0) (ok #t))
         (if (= i 200)
             ok
             (let ((n (random-integer 7)))
               (loop (+ i 1) (and ok (exact? n) (>= n 0) (< n 7))))))
       #t)
;; The SRFI's interval is OPEN, so zero never comes back.
(check random-real-is-open
       (let loop ((i 0) (ok #t))
         (if (= i 500)
             ok
             (let ((x (random-real)))
               (loop (+ i 1) (and ok (> x 0.0) (< x 1.0))))))
       #t)
(check-raises random-integer-rejects-zero
              (random-integer 0))

;; scm's extension: the source is a parameter, so a seed can be scoped to
;; one build the way ray's `:ray-rng` dynamic variable is.
(check parameterized-source-is-reproducible
       (equal? (parameterize ((current-random-source (make-random-source 99)))
                 (list (random-integer 1000) (random-integer 1000)))
               (parameterize ((current-random-source (make-random-source 99)))
                 (list (random-integer 1000) (random-integer 1000))))
       #t)

(test-failures)
