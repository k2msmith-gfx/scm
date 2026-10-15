;;; (srfi 160 base) — the core of the homogeneous numeric vectors.
;;;
;;; SRFI 160 is the successor to `(srfi 4)`, split into this base library --
;;; the eight operations per element type that SRFI 4 itself defines -- and
;;; one library per type carrying the full sixty-odd operations.
;;;
;;; These are `(srfi 4)`'s own bindings under this name, so the two are
;;; interchangeable and there is one implementation. The storage is native
;;; and unboxed; see `(srfi 4)` for what that buys, and chiefly for the fact
;;; that `u8vector` **is** R7RS's bytevector.
;;;
;;; Two absences worth stating. **`c64` and `c128` are not here**: they are
;;; vectors of complex numbers, and plan §2.3 declines the numeric tower, so
;;; there is no element to store. **`@?`, the element predicate, lives in
;;; the per-type libraries** rather than here, so that importing a per-type
;;; library beside this one cannot bring the same name in twice.
(define-library (srfi 160 base)
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
  (import (srfi 4)))
