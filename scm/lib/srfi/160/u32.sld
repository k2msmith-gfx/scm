;;; (srfi 160 u32) — the full operation set for u32vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `u32vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `u32?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 u32)
  (export
    make-u32vector u32vector u32vector? u32vector-length u32vector-ref
    u32vector-set! u32vector->list list->u32vector u32? u32vector-unfold
    u32vector-unfold-right u32vector-append u32vector-concatenate
    u32vector-append-subvectors reverse-list->u32vector vector->u32vector
    u32vector-comparator u32vector-copy u32vector-reverse-copy
    u32vector-empty? u32vector= u32vector-take u32vector-take-right
    u32vector-drop u32vector-drop-right u32vector-segment u32vector-fold
    u32vector-fold-right u32vector-map u32vector-map! u32vector-for-each
    u32vector-count u32vector-cumulate u32vector-take-while
    u32vector-take-while-right u32vector-drop-while
    u32vector-drop-while-right u32vector-index u32vector-index-right
    u32vector-skip u32vector-skip-right u32vector-any u32vector-every
    u32vector-partition u32vector-filter u32vector-remove u32vector-swap!
    u32vector-fill! u32vector-reverse! u32vector-copy!
    u32vector-reverse-copy! u32vector-unfold! u32vector-unfold-right!
    reverse-u32vector->list u32vector->vector make-u32vector-generator
    write-u32vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (u32? x) (numvector-element? 'u32 x))
    (define (u32vector-unfold f len seed) (numvector-unfold 'u32 f len seed))
    (define (u32vector-unfold-right f len seed)
      (numvector-unfold-right 'u32 f len seed))
    (define (u32vector-append . vs) (numvector-append 'u32 vs))
    (define (u32vector-concatenate vs) (numvector-concatenate 'u32 vs))
    (define (u32vector-append-subvectors . args)
      (numvector-append-subvectors 'u32 args))
    (define (reverse-list->u32vector lst) (reverse-list->numvector 'u32 lst))
    (define (vector->u32vector vec . opt)
      (apply vector->numvector 'u32 vec opt))
    (define u32vector-comparator (make-numvector-comparator 'u32))

    ;; The same procedure under this type's name.
    (define u32vector-copy numvector-copy)
    (define u32vector-reverse-copy numvector-reverse-copy)
    (define u32vector-empty? numvector-empty?)
    (define u32vector= numvector=)
    (define u32vector-take numvector-take)
    (define u32vector-take-right numvector-take-right)
    (define u32vector-drop numvector-drop)
    (define u32vector-drop-right numvector-drop-right)
    (define u32vector-segment numvector-segment)
    (define u32vector-fold numvector-fold)
    (define u32vector-fold-right numvector-fold-right)
    (define u32vector-map numvector-map)
    (define u32vector-map! numvector-map!)
    (define u32vector-for-each numvector-for-each)
    (define u32vector-count numvector-count)
    (define u32vector-cumulate numvector-cumulate)
    (define u32vector-take-while numvector-take-while)
    (define u32vector-take-while-right numvector-take-while-right)
    (define u32vector-drop-while numvector-drop-while)
    (define u32vector-drop-while-right numvector-drop-while-right)
    (define u32vector-index numvector-index)
    (define u32vector-index-right numvector-index-right)
    (define u32vector-skip numvector-skip)
    (define u32vector-skip-right numvector-skip-right)
    (define u32vector-any numvector-any)
    (define u32vector-every numvector-every)
    (define u32vector-partition numvector-partition)
    (define u32vector-filter numvector-filter)
    (define u32vector-remove numvector-remove)
    (define u32vector-swap! numvector-swap!)
    (define u32vector-fill! numvector-fill!)
    (define u32vector-reverse! numvector-reverse!)
    (define u32vector-copy! numvector-copy!)
    (define u32vector-reverse-copy! numvector-reverse-copy!)
    (define u32vector-unfold! numvector-unfold!)
    (define u32vector-unfold-right! numvector-unfold-right!)
    (define reverse-u32vector->list reverse-numvector->list)
    (define u32vector->vector numvector->vector)
    (define make-u32vector-generator make-numvector-generator)
    (define write-u32vector write-numvector)))
