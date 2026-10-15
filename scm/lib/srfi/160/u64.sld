;;; (srfi 160 u64) — the full operation set for u64vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `u64vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `u64?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 u64)
  (export
    make-u64vector u64vector u64vector? u64vector-length u64vector-ref
    u64vector-set! u64vector->list list->u64vector u64? u64vector-unfold
    u64vector-unfold-right u64vector-append u64vector-concatenate
    u64vector-append-subvectors reverse-list->u64vector vector->u64vector
    u64vector-comparator u64vector-copy u64vector-reverse-copy
    u64vector-empty? u64vector= u64vector-take u64vector-take-right
    u64vector-drop u64vector-drop-right u64vector-segment u64vector-fold
    u64vector-fold-right u64vector-map u64vector-map! u64vector-for-each
    u64vector-count u64vector-cumulate u64vector-take-while
    u64vector-take-while-right u64vector-drop-while
    u64vector-drop-while-right u64vector-index u64vector-index-right
    u64vector-skip u64vector-skip-right u64vector-any u64vector-every
    u64vector-partition u64vector-filter u64vector-remove u64vector-swap!
    u64vector-fill! u64vector-reverse! u64vector-copy!
    u64vector-reverse-copy! u64vector-unfold! u64vector-unfold-right!
    reverse-u64vector->list u64vector->vector make-u64vector-generator
    write-u64vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (u64? x) (numvector-element? 'u64 x))
    (define (u64vector-unfold f len seed) (numvector-unfold 'u64 f len seed))
    (define (u64vector-unfold-right f len seed)
      (numvector-unfold-right 'u64 f len seed))
    (define (u64vector-append . vs) (numvector-append 'u64 vs))
    (define (u64vector-concatenate vs) (numvector-concatenate 'u64 vs))
    (define (u64vector-append-subvectors . args)
      (numvector-append-subvectors 'u64 args))
    (define (reverse-list->u64vector lst) (reverse-list->numvector 'u64 lst))
    (define (vector->u64vector vec . opt)
      (apply vector->numvector 'u64 vec opt))
    (define u64vector-comparator (make-numvector-comparator 'u64))

    ;; The same procedure under this type's name.
    (define u64vector-copy numvector-copy)
    (define u64vector-reverse-copy numvector-reverse-copy)
    (define u64vector-empty? numvector-empty?)
    (define u64vector= numvector=)
    (define u64vector-take numvector-take)
    (define u64vector-take-right numvector-take-right)
    (define u64vector-drop numvector-drop)
    (define u64vector-drop-right numvector-drop-right)
    (define u64vector-segment numvector-segment)
    (define u64vector-fold numvector-fold)
    (define u64vector-fold-right numvector-fold-right)
    (define u64vector-map numvector-map)
    (define u64vector-map! numvector-map!)
    (define u64vector-for-each numvector-for-each)
    (define u64vector-count numvector-count)
    (define u64vector-cumulate numvector-cumulate)
    (define u64vector-take-while numvector-take-while)
    (define u64vector-take-while-right numvector-take-while-right)
    (define u64vector-drop-while numvector-drop-while)
    (define u64vector-drop-while-right numvector-drop-while-right)
    (define u64vector-index numvector-index)
    (define u64vector-index-right numvector-index-right)
    (define u64vector-skip numvector-skip)
    (define u64vector-skip-right numvector-skip-right)
    (define u64vector-any numvector-any)
    (define u64vector-every numvector-every)
    (define u64vector-partition numvector-partition)
    (define u64vector-filter numvector-filter)
    (define u64vector-remove numvector-remove)
    (define u64vector-swap! numvector-swap!)
    (define u64vector-fill! numvector-fill!)
    (define u64vector-reverse! numvector-reverse!)
    (define u64vector-copy! numvector-copy!)
    (define u64vector-reverse-copy! numvector-reverse-copy!)
    (define u64vector-unfold! numvector-unfold!)
    (define u64vector-unfold-right! numvector-unfold-right!)
    (define reverse-u64vector->list reverse-numvector->list)
    (define u64vector->vector numvector->vector)
    (define make-u64vector-generator make-numvector-generator)
    (define write-u64vector write-numvector)))
