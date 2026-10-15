;;; (srfi 160 u8) — the full operation set for u8vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `u8vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `u8?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
;;;
;;; `u8vector` **is** R7RS's bytevector, so these are operations on
;;; bytevectors -- which is what an 8-bit image buffer is. The generic
;;; primitives accept either representation for exactly this reason.
(define-library (srfi 160 u8)
  (export
    make-u8vector u8vector u8vector? u8vector-length u8vector-ref
    u8vector-set! u8vector->list list->u8vector u8? u8vector-unfold
    u8vector-unfold-right u8vector-append u8vector-concatenate
    u8vector-append-subvectors reverse-list->u8vector vector->u8vector
    u8vector-comparator u8vector-copy u8vector-reverse-copy u8vector-empty?
    u8vector= u8vector-take u8vector-take-right u8vector-drop
    u8vector-drop-right u8vector-segment u8vector-fold u8vector-fold-right
    u8vector-map u8vector-map! u8vector-for-each u8vector-count
    u8vector-cumulate u8vector-take-while u8vector-take-while-right
    u8vector-drop-while u8vector-drop-while-right u8vector-index
    u8vector-index-right u8vector-skip u8vector-skip-right u8vector-any
    u8vector-every u8vector-partition u8vector-filter u8vector-remove
    u8vector-swap! u8vector-fill! u8vector-reverse! u8vector-copy!
    u8vector-reverse-copy! u8vector-unfold! u8vector-unfold-right!
    reverse-u8vector->list u8vector->vector make-u8vector-generator
    write-u8vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (u8? x) (numvector-element? 'u8 x))
    (define (u8vector-unfold f len seed) (numvector-unfold 'u8 f len seed))
    (define (u8vector-unfold-right f len seed)
      (numvector-unfold-right 'u8 f len seed))
    (define (u8vector-append . vs) (numvector-append 'u8 vs))
    (define (u8vector-concatenate vs) (numvector-concatenate 'u8 vs))
    (define (u8vector-append-subvectors . args)
      (numvector-append-subvectors 'u8 args))
    (define (reverse-list->u8vector lst) (reverse-list->numvector 'u8 lst))
    (define (vector->u8vector vec . opt)
      (apply vector->numvector 'u8 vec opt))
    (define u8vector-comparator (make-numvector-comparator 'u8))

    ;; The same procedure under this type's name.
    (define u8vector-copy numvector-copy)
    (define u8vector-reverse-copy numvector-reverse-copy)
    (define u8vector-empty? numvector-empty?)
    (define u8vector= numvector=)
    (define u8vector-take numvector-take)
    (define u8vector-take-right numvector-take-right)
    (define u8vector-drop numvector-drop)
    (define u8vector-drop-right numvector-drop-right)
    (define u8vector-segment numvector-segment)
    (define u8vector-fold numvector-fold)
    (define u8vector-fold-right numvector-fold-right)
    (define u8vector-map numvector-map)
    (define u8vector-map! numvector-map!)
    (define u8vector-for-each numvector-for-each)
    (define u8vector-count numvector-count)
    (define u8vector-cumulate numvector-cumulate)
    (define u8vector-take-while numvector-take-while)
    (define u8vector-take-while-right numvector-take-while-right)
    (define u8vector-drop-while numvector-drop-while)
    (define u8vector-drop-while-right numvector-drop-while-right)
    (define u8vector-index numvector-index)
    (define u8vector-index-right numvector-index-right)
    (define u8vector-skip numvector-skip)
    (define u8vector-skip-right numvector-skip-right)
    (define u8vector-any numvector-any)
    (define u8vector-every numvector-every)
    (define u8vector-partition numvector-partition)
    (define u8vector-filter numvector-filter)
    (define u8vector-remove numvector-remove)
    (define u8vector-swap! numvector-swap!)
    (define u8vector-fill! numvector-fill!)
    (define u8vector-reverse! numvector-reverse!)
    (define u8vector-copy! numvector-copy!)
    (define u8vector-reverse-copy! numvector-reverse-copy!)
    (define u8vector-unfold! numvector-unfold!)
    (define u8vector-unfold-right! numvector-unfold-right!)
    (define reverse-u8vector->list reverse-numvector->list)
    (define u8vector->vector numvector->vector)
    (define make-u8vector-generator make-numvector-generator)
    (define write-u8vector write-numvector)))
