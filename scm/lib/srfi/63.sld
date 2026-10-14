;;; (srfi 63) — Homogeneous and Heterogeneous Arrays.
;;;
;;; An array here is a **view**: dimensions, a stride per dimension, an
;;; offset and a store, with the index arithmetic native. So
;;; `make-shared-array` is a row, a column, a transpose or a subrectangle
;;; that *shares* the store rather than copying it, and an image or a
;;; volume over an `f64vector` costs eight bytes a sample with no
;;; indirection.
;;;
;;; `equal?` is extended to arrays, as the SRFI requires: same shape and
;;; `equal?` elements, whatever either one's store or strides -- so a
;;; transpose compares equal to a copy of itself.
;;;
;;; Three things to know.
;;;
;;; **`array-set!` takes the value second**: `(array-set! a obj k …)`, which
;;; is SRFI 63's order, inherited from SLIB. SRFI 25 puts it last. Getting
;;; this backwards is silent, so it is worth saying twice.
;;;
;;; **The prototype procedures are the subset that has real storage.**
;;; `a:floc*` are complex and scm has no complex numbers (plan §2.3);
;;; `a:flor128b` and `a:flor16b` want 128- and 16-bit floats, which there
;;; are none of. Those five are absent rather than silently widened to
;;; something else. `a:bool` is backed by a general vector, so it holds
;;; real `#t`/`#f` rather than being bit-packed.
;;;
;;; **SRFI 47 is not implemented and will not be**: it is withdrawn, and
;;; this SRFI supersedes it and SRFI 25.
(define-library (srfi 63)
  (export array? array-rank array-dimensions array-in-bounds?
          array-ref array-set! make-array make-shared-array
          array->vector array->list
          a:flor64b a:flor32b
          a:fixz64b a:fixz32b a:fixz16b a:fixz8b
          a:fixn64b a:fixn32b a:fixn16b a:fixn8b
          a:bool)
  (import (scheme base) (srfi 4))
  (begin

    ;; `array-set!`'s argument order is the SRFI's, and the primitive's.
    (define (array-set! a obj . subs) (apply %array-set! a obj subs))

    ;; ── prototypes ────────────────────────────────────────────────────
    ;;
    ;; A prototype is a procedure that returns an empty vector of its
    ;; element type; `make-array` looks at what it returns to decide what
    ;; store to build. Each also accepts the SRFI's optional arguments and
    ;; ignores them, since the type is the whole message.
    (define (a:flor64b . _) (f64vector))
    (define (a:flor32b . _) (f32vector))
    (define (a:fixz64b . _) (s64vector))
    (define (a:fixz32b . _) (s32vector))
    (define (a:fixz16b . _) (s16vector))
    (define (a:fixz8b . _) (s8vector))
    (define (a:fixn64b . _) (u64vector))
    (define (a:fixn32b . _) (u32vector))
    (define (a:fixn16b . _) (u16vector))
    (define (a:fixn8b . _) (u8vector))
    (define (a:bool . _) (vector))

    ;; A store of `n` elements shaped like `proto`. `proto` may be the
    ;; prototype procedure, or any vector/string/bytevector to copy the
    ;; type of -- SRFI 63 allows `vector` and `string` themselves.
    (define (%store-like proto n)
      (let ((sample (cond ((procedure? proto) (proto))
                          (else proto))))
        (cond ((numvector? sample)
               (make-numvector (numvector-tag sample) n))
              ((bytevector? sample) (make-bytevector n 0))
              ((string? sample) (make-string n #\space))
              ((vector? sample) (make-vector n #f))
              (else (error "make-array: not a prototype" proto)))))

    (define (make-array proto . dims)
      (let ((n (let loop ((d dims) (acc 1))
                 (if (null? d) acc (loop (cdr d) (* acc (car d)))))))
        (%make-array (%store-like proto n) dims)))

    ;; ── shared arrays ─────────────────────────────────────────────────
    ;;
    ;; SRFI 63 requires the mapper to be affine, so it is solved rather
    ;; than called per access: the image of the origin is the offset, and
    ;; the image of each unit step is that dimension's stride. One call per
    ;; dimension plus one, at view-construction time, instead of a Scheme
    ;; call on every read.
    ;;
    ;; A mapper that is *not* affine is an error the SRFI already names,
    ;; and this will quietly take its linearisation -- but a view that
    ;; reaches outside the store is refused by the primitive.
    (define (make-shared-array array mapper . dims)
      (let* ((rank (length dims))
             (zeros (let loop ((i rank) (acc '()))
                      (if (= i 0) acc (loop (- i 1) (cons 0 acc)))))
             (origin (%store-offset array (apply mapper zeros)))
             (strides
              (let loop ((i 0) (acc '()))
                (if (= i rank)
                    (reverse acc)
                    (let* ((probe (%unit-at zeros i))
                           (there (%store-offset array (apply mapper probe))))
                      (loop (+ i 1) (cons (- there origin) acc)))))))
        (%make-array-view (%array-store array) dims strides origin)))

    ;; `(0 0 … 1 … 0)`: a one in position `i`.
    (define (%unit-at zeros i)
      (let loop ((k 0) (z zeros) (acc '()))
        (if (null? z)
            (reverse acc)
            (loop (+ k 1) (cdr z) (cons (if (= k i) 1 0) acc)))))

    ;; Where the old array's own subscripts land in its store. The mapper
    ;; returns a list of subscripts (or a single one for rank 1).
    (define (%store-offset array subs)
      (let ((subs (if (list? subs) subs (list subs)))
            (strides (%array-strides array)))
        (let loop ((s subs) (st strides) (acc (%array-offset array)))
          (if (or (null? s) (null? st))
              acc
              (loop (cdr s) (cdr st) (+ acc (* (car s) (car st))))))))

    ;; ── conversions, which the SRFI does not define but every user wants ──
    (define (array->list a)
      (let* ((dims (array-dimensions a))
             (n (let loop ((d dims) (acc 1))
                  (if (null? d) acc (loop (cdr d) (* acc (car d)))))))
        (let loop ((k (- n 1)) (acc '()))
          (if (< k 0)
              acc
              (loop (- k 1) (cons (apply array-ref a (%unflatten dims k)) acc))))))

    (define (array->vector a) (list->vector (array->list a)))

    ;; The `k`-th subscript list in row-major order.
    (define (%unflatten dims k)
      (let loop ((d (reverse dims)) (k k) (acc '()))
        (if (null? d)
            acc
            (loop (cdr d) (quotient k (car d)) (cons (remainder k (car d)) acc)))))))
