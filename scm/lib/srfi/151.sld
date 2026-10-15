;;; (srfi 151) — Bitwise Operations.
;;;
;;; An `i64` *is* two's complement, so `and`, `ior`, `xor` and `not` give
;;; this SRFI's infinite-precision answers directly for every value that
;;; fits, and the derived operations below are the ordinary identities.
;;;
;;; What does not fit is a left shift off the end of the word.
;;; `(arithmetic-shift 1 63)` raises rather than returning 2^63, because
;;; plan §2.3 declines bignums and §4.5 makes an integer overflow an error
;;; rather than a silent widening -- and bit twiddling is the one place a
;;; caller is usually counting on exactness. A right shift saturates
;;; instead: shifting a negative number right far enough gives -1, which is
;;; the infinite model's answer and needs no bignum.
(define-library (srfi 151)
  (export bitwise-not bitwise-and bitwise-ior bitwise-xor bitwise-eqv
          bitwise-nand bitwise-nor bitwise-andc1 bitwise-andc2
          bitwise-orc1 bitwise-orc2
          arithmetic-shift bit-count integer-length bitwise-if
          bit-set? copy-bit bit-swap any-bit-set? every-bit-set?
          first-set-bit
          bit-field bit-field-any? bit-field-every? bit-field-clear
          bit-field-set bit-field-replace bit-field-replace-same
          bit-field-rotate bit-field-reverse
          bits->list list->bits bits->vector vector->bits bits
          bitwise-fold bitwise-for-each bitwise-unfold
          make-bitwise-generator)
  (import (scheme base))
  (begin

    ;; ── the derived logical operations ────────────────────────────────
    (define (bitwise-eqv . args)
      (let loop ((acc -1) (as args))
        (if (null? as)
            acc
            (loop (bitwise-not (bitwise-xor acc (car as))) (cdr as)))))
    (define (bitwise-nand a b) (bitwise-not (bitwise-and a b)))
    (define (bitwise-nor a b) (bitwise-not (bitwise-ior a b)))
    (define (bitwise-andc1 a b) (bitwise-and (bitwise-not a) b))
    (define (bitwise-andc2 a b) (bitwise-and a (bitwise-not b)))
    (define (bitwise-orc1 a b) (bitwise-ior (bitwise-not a) b))
    (define (bitwise-orc2 a b) (bitwise-ior a (bitwise-not b)))

    ;; Bits of `a` where `mask` is 1, of `b` where it is 0.
    (define (bitwise-if mask a b)
      (bitwise-ior (bitwise-and mask a) (bitwise-and (bitwise-not mask) b)))

    ;; ── single bits ───────────────────────────────────────────────────
    ;;
    ;; The index comes first in this SRFI, which is worth noticing once.
    (define (bit-set? index i) (odd? (arithmetic-shift i (- index))))

    (define (copy-bit index i boolean)
      (if boolean
          (bitwise-ior i (arithmetic-shift 1 index))
          (bitwise-and i (bitwise-not (arithmetic-shift 1 index)))))

    (define (bit-swap n1 n2 i)
      (let ((b1 (bit-set? n1 i)) (b2 (bit-set? n2 i)))
        (copy-bit n2 (copy-bit n1 i b2) b1)))

    (define (any-bit-set? test i) (not (zero? (bitwise-and test i))))
    (define (every-bit-set? test i) (= test (bitwise-and test i)))

    ;; ── bit fields ────────────────────────────────────────────────────
    (define (%mask start end) (- (arithmetic-shift 1 (- end start)) 1))

    (define (bit-field i start end)
      (bitwise-and (arithmetic-shift i (- start)) (%mask start end)))

    (define (bit-field-any? i start end) (not (zero? (bit-field i start end))))
    (define (bit-field-every? i start end)
      (= (bit-field i start end) (%mask start end)))

    (define (bit-field-clear i start end) (bit-field-replace i 0 start end))
    (define (bit-field-set i start end)
      (bit-field-replace i (%mask start end) start end))

    ;; `source`'s low bits land in `dest`'s `start`..`end`.
    (define (bit-field-replace dest source start end)
      (let ((m (arithmetic-shift (%mask start end) start)))
        (bitwise-ior (bitwise-and dest (bitwise-not m))
                     (bitwise-and (arithmetic-shift source start) m))))

    ;; The same field of `source`, in place.
    (define (bit-field-replace-same dest source start end)
      (bitwise-if (arithmetic-shift (%mask start end) start) source dest))

    (define (bit-field-rotate i count start end)
      (let* ((width (- end start))
             (field (bit-field i start end)))
        (if (= width 0)
            i
            (let* ((c (modulo count width))
                   (rotated (bitwise-and
                             (bitwise-ior (arithmetic-shift field c)
                                          (arithmetic-shift field (- (- width c))))
                             (%mask start end))))
              (bit-field-replace i rotated start end)))))

    (define (bit-field-reverse i start end)
      (let loop ((k 0) (acc 0))
        (if (>= k (- end start))
            (bit-field-replace i acc start end)
            (loop (+ k 1)
                  (copy-bit (- (- end start) k 1) acc (bit-set? (+ start k) i))))))

    ;; ── bits as sequences ─────────────────────────────────────────────
    ;;
    ;; Least significant first, in every direction.
    (define (bits->list i . opt)
      (let ((len (if (pair? opt) (car opt) (integer-length i))))
        (let loop ((k (- len 1)) (acc '()))
          (if (< k 0) acc (loop (- k 1) (cons (bit-set? k i) acc))))))

    (define (list->bits bools)
      (let loop ((bs bools) (k 0) (acc 0))
        (if (null? bs)
            acc
            (loop (cdr bs) (+ k 1) (if (car bs) (copy-bit k acc #t) acc)))))

    (define (bits->vector i . opt) (list->vector (apply bits->list i opt)))
    (define (vector->bits v) (list->bits (vector->list v)))
    (define (bits . bools) (list->bits bools))

    ;; ── folding over the bits ─────────────────────────────────────────
    (define (bitwise-fold proc seed i)
      (let loop ((k 0) (acc seed))
        (if (>= k (integer-length i))
            acc
            (loop (+ k 1) (proc (bit-set? k i) acc)))))

    (define (bitwise-for-each proc i)
      (let loop ((k 0))
        (when (< k (integer-length i))
          (proc (bit-set? k i))
          (loop (+ k 1)))))

    (define (bitwise-unfold stop? mapper successor seed)
      (let loop ((k 0) (state seed) (acc 0))
        (if (stop? state)
            acc
            (loop (+ k 1)
                  (successor state)
                  (if (mapper state) (copy-bit k acc #t) acc)))))

    (define (make-bitwise-generator i)
      (let ((k 0))
        (lambda ()
          (let ((b (bit-set? k i)))
            (set! k (+ k 1))
            b))))))
