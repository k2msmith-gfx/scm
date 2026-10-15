;;; (srfi 27) — Sources of Random Bits.
;;;
;;; Most of this is native, because a generator is a mutable word of state
;;; and the useful part of this SRFI is the half that makes a run
;;; *reproducible*: save the state, generate, restore the state, and get
;;; the same numbers again. For procedural modelling that is the whole
;;; game. ray's scene corpus mentions `math/random` in six files but
;;; `:seed` in seventeen, and `grow-branches` can only be replayed because
;;; a seed is threaded through it.
;;;
;;; The generator is SplitMix64: fast, one word of state, good enough for
;;; scattering points and sampling directions. It is **not** cryptographic
;;; and nothing here pretends otherwise.
;;;
;;; Two deviations from the SRFI, both deliberate.
;;;
;;; **`random-integer` and `random-real` take an optional source.** The SRFI
;;; gives them no argument and ties them to `default-random-source`. scm
;;; makes the source a *parameter*, `current-random-source`, so
;;; `parameterize` can scope a seed to one build the way ray's `:ray-rng`
;;; dynamic variable does. Called with no source they read that parameter,
;;; so SRFI code works unchanged; passing one explicitly is the extension.
;;;
;;; **`default-random-source` is the source that parameter starts with.**
;;; So `(random-source-randomize! default-random-source)` does affect a
;;; later bare `(random-integer 10)`, as the SRFI expects -- unless
;;; something has parameterized `current-random-source` to another source,
;;; in which case the parameter wins, which is the point of having it.
(define-library (srfi 27)
  (export default-random-source
          random-integer random-real
          make-random-source random-source?
          random-source-state-ref random-source-state-set!
          random-source-randomize! random-source-pseudo-randomize!
          random-source-make-integers random-source-make-reals)
  (import (scheme base))
  (begin

    (define default-random-source (current-random-source))

    ;; The SRFI's reals are in the OPEN interval (0,1), where the native
    ;; `random-real` gives [0,1). Rather than shadow the native one -- a
    ;; library body's definitions are mutually recursive, so naming it here
    ;; would call *this* -- the value is built from an integer draw over the
    ;; 53 bits a double actually has. Zero is redrawn, which happens with
    ;; probability 2^-53, so the result is exact rather than nudged.
    (define %mantissa 9007199254740992) ; 2^53

    (define (random-real . opt)
      (let loop ()
        (let ((n (apply random-integer %mantissa opt)))
          (if (= n 0) (loop) (/ n 9007199254740992.0)))))

    (define (random-source-make-integers s)
      (lambda (n) (random-integer n s)))

    ;; `unit`, when given, is the resolution: every result is a multiple of
    ;; it, still inside the open interval.
    (define (random-source-make-reals s . opt)
      (if (null? opt)
          (lambda () (random-real s))
          (let* ((unit (car opt))
                 (steps (exact (floor (/ 1 unit)))))
            (unless (and (> unit 0) (< unit 1))
              (error "random-source-make-reals: unit must be between 0 and 1" unit))
            (lambda () (* unit (+ 1 (random-integer (- steps 1) s)))))))))
