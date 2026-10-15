;;; (srfi 160 u16) — the full operation set for u16vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `u16vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `u16?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 u16)
  (export
    make-u16vector u16vector u16vector? u16vector-length u16vector-ref
    u16vector-set! u16vector->list list->u16vector u16? u16vector-unfold
    u16vector-unfold-right u16vector-append u16vector-concatenate
    u16vector-append-subvectors reverse-list->u16vector vector->u16vector
    u16vector-comparator u16vector-copy u16vector-reverse-copy
    u16vector-empty? u16vector= u16vector-take u16vector-take-right
    u16vector-drop u16vector-drop-right u16vector-segment u16vector-fold
    u16vector-fold-right u16vector-map u16vector-map! u16vector-for-each
    u16vector-count u16vector-cumulate u16vector-take-while
    u16vector-take-while-right u16vector-drop-while
    u16vector-drop-while-right u16vector-index u16vector-index-right
    u16vector-skip u16vector-skip-right u16vector-any u16vector-every
    u16vector-partition u16vector-filter u16vector-remove u16vector-swap!
    u16vector-fill! u16vector-reverse! u16vector-copy!
    u16vector-reverse-copy! u16vector-unfold! u16vector-unfold-right!
    reverse-u16vector->list u16vector->vector make-u16vector-generator
    write-u16vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (u16? x) (numvector-element? 'u16 x))
    (define (u16vector-unfold f len seed) (numvector-unfold 'u16 f len seed))
    (define (u16vector-unfold-right f len seed)
      (numvector-unfold-right 'u16 f len seed))
    (define (u16vector-append . vs) (numvector-append 'u16 vs))
    (define (u16vector-concatenate vs) (numvector-concatenate 'u16 vs))
    (define (u16vector-append-subvectors . args)
      (numvector-append-subvectors 'u16 args))
    (define (reverse-list->u16vector lst) (reverse-list->numvector 'u16 lst))
    (define (vector->u16vector vec . opt)
      (apply vector->numvector 'u16 vec opt))
    (define u16vector-comparator (make-numvector-comparator 'u16))

    ;; The same procedure under this type's name.
    (define u16vector-copy numvector-copy)
    (define u16vector-reverse-copy numvector-reverse-copy)
    (define u16vector-empty? numvector-empty?)
    (define u16vector= numvector=)
    (define u16vector-take numvector-take)
    (define u16vector-take-right numvector-take-right)
    (define u16vector-drop numvector-drop)
    (define u16vector-drop-right numvector-drop-right)
    (define u16vector-segment numvector-segment)
    (define u16vector-fold numvector-fold)
    (define u16vector-fold-right numvector-fold-right)
    (define u16vector-map numvector-map)
    (define u16vector-map! numvector-map!)
    (define u16vector-for-each numvector-for-each)
    (define u16vector-count numvector-count)
    (define u16vector-cumulate numvector-cumulate)
    (define u16vector-take-while numvector-take-while)
    (define u16vector-take-while-right numvector-take-while-right)
    (define u16vector-drop-while numvector-drop-while)
    (define u16vector-drop-while-right numvector-drop-while-right)
    (define u16vector-index numvector-index)
    (define u16vector-index-right numvector-index-right)
    (define u16vector-skip numvector-skip)
    (define u16vector-skip-right numvector-skip-right)
    (define u16vector-any numvector-any)
    (define u16vector-every numvector-every)
    (define u16vector-partition numvector-partition)
    (define u16vector-filter numvector-filter)
    (define u16vector-remove numvector-remove)
    (define u16vector-swap! numvector-swap!)
    (define u16vector-fill! numvector-fill!)
    (define u16vector-reverse! numvector-reverse!)
    (define u16vector-copy! numvector-copy!)
    (define u16vector-reverse-copy! numvector-reverse-copy!)
    (define u16vector-unfold! numvector-unfold!)
    (define u16vector-unfold-right! numvector-unfold-right!)
    (define reverse-u16vector->list reverse-numvector->list)
    (define u16vector->vector numvector->vector)
    (define make-u16vector-generator make-numvector-generator)
    (define write-u16vector write-numvector)))
