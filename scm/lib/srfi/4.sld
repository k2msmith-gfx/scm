;;; (srfi 4) — Homogeneous numeric vectors.
;;;
;;; The storage is native and **unboxed**: an `f64vector` of a million
;;; samples is eight megabytes of `f64`, where a `vector` of the same
;;; flonums is sixteen bytes and a tag check per element. That is the whole
;;; reason this library exists, and the reason ray wants it -- an image, a
;;; height field or a volume is exactly this shape.
;;;
;;; The reader and printer know the literals: `#f64(1.0 2.5)`, `#s8(-1 2)`,
;;; `#u16(65535)`, and `#u8(…)` which is R7RS's bytevector.
;;;
;;; Two things to know.
;;;
;;; **`u8vector` *is* `bytevector`.** R7RS's bytevector is the vector of
;;; octets, so these names are its operations rather than a second
;;; representation of the same thing.
;;;
;;; **The type-agnostic operations are aliases, not wrappers.**
;;; `f64vector-ref` is the primitive `numvector-ref`, which asks the vector
;;; what type it is -- so a hot loop pays nothing for the name, and
;;; `(f64vector-ref some-s32vector i)` returns the element rather than
;;; complaining. SRFI 4 says that case "is an error", which leaves it
;;; unspecified; the alternative was a wrapper call per element.
(define-library (srfi 4)
  (export
    make-s8vector s8vector s8vector? s8vector-length s8vector-ref
    s8vector-set! s8vector->list list->s8vector
    make-u8vector u8vector u8vector? u8vector-length u8vector-ref
    u8vector-set! u8vector->list list->u8vector
    make-s16vector s16vector s16vector? s16vector-length s16vector-ref
    s16vector-set! s16vector->list list->s16vector
    make-u16vector u16vector u16vector? u16vector-length u16vector-ref
    u16vector-set! u16vector->list list->u16vector
    make-s32vector s32vector s32vector? s32vector-length s32vector-ref
    s32vector-set! s32vector->list list->s32vector
    make-u32vector u32vector u32vector? u32vector-length u32vector-ref
    u32vector-set! u32vector->list list->u32vector
    make-s64vector s64vector s64vector? s64vector-length s64vector-ref
    s64vector-set! s64vector->list list->s64vector
    make-u64vector u64vector u64vector? u64vector-length u64vector-ref
    u64vector-set! u64vector->list list->u64vector
    make-f32vector f32vector f32vector? f32vector-length f32vector-ref
    f32vector-set! f32vector->list list->f32vector
    make-f64vector f64vector f64vector? f64vector-length f64vector-ref
    f64vector-set! f64vector->list list->f64vector)
  (import (scheme base))
  (begin

    ;; Nine of the ten types differ only in the element-type symbol, so
    ;; they are written once. `maker`, `ctor`, `pred` and `from-list` need
    ;; to be told the type; the rest are the generic primitives under
    ;; another name.
    (define-syntax define-numvector-type
      (syntax-rules ()
        ((_ tag maker ctor pred len ref setter to-list from-list)
         (begin
           (define (maker k . fill)
             (if (pair? fill)
                 (make-numvector 'tag k (car fill))
                 (make-numvector 'tag k)))
           (define (ctor . xs) (list->numvector 'tag xs))
           (define (pred x) (numvector? x 'tag))
           (define len numvector-length)
           (define ref numvector-ref)
           (define setter numvector-set!)
           (define to-list numvector->list)
           (define (from-list l) (list->numvector 'tag l))))))

    (define-numvector-type s8 make-s8vector s8vector s8vector?
      s8vector-length s8vector-ref s8vector-set! s8vector->list
      list->s8vector)
    (define-numvector-type s16 make-s16vector s16vector s16vector?
      s16vector-length s16vector-ref s16vector-set! s16vector->list
      list->s16vector)
    (define-numvector-type u16 make-u16vector u16vector u16vector?
      u16vector-length u16vector-ref u16vector-set! u16vector->list
      list->u16vector)
    (define-numvector-type s32 make-s32vector s32vector s32vector?
      s32vector-length s32vector-ref s32vector-set! s32vector->list
      list->s32vector)
    (define-numvector-type u32 make-u32vector u32vector u32vector?
      u32vector-length u32vector-ref u32vector-set! u32vector->list
      list->u32vector)
    (define-numvector-type s64 make-s64vector s64vector s64vector?
      s64vector-length s64vector-ref s64vector-set! s64vector->list
      list->s64vector)
    (define-numvector-type u64 make-u64vector u64vector u64vector?
      u64vector-length u64vector-ref u64vector-set! u64vector->list
      list->u64vector)
    (define-numvector-type f32 make-f32vector f32vector f32vector?
      f32vector-length f32vector-ref f32vector-set! f32vector->list
      list->f32vector)
    (define-numvector-type f64 make-f64vector f64vector f64vector?
      f64vector-length f64vector-ref f64vector-set! f64vector->list
      list->f64vector)

    ;; ── u8, which is the bytevector ───────────────────────────────────
    (define make-u8vector make-bytevector)
    (define u8vector bytevector)
    (define u8vector? bytevector?)
    (define u8vector-length bytevector-length)
    (define u8vector-ref bytevector-u8-ref)
    (define u8vector-set! bytevector-u8-set!)
    (define (list->u8vector l) (apply bytevector l))
    (define (u8vector->list v . se)
      (let* ((start (if (pair? se) (car se) 0))
             (end (if (and (pair? se) (pair? (cdr se)))
                      (cadr se)
                      (bytevector-length v))))
        (let loop ((i (- end 1)) (acc '()))
          (if (< i start) acc (loop (- i 1) (cons (bytevector-u8-ref v i) acc))))))))
