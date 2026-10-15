;;; (srfi 160 f32) — the full operation set for f32vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `f32vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `f32?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 f32)
  (export
    make-f32vector f32vector f32vector? f32vector-length f32vector-ref
    f32vector-set! f32vector->list list->f32vector f32? f32vector-unfold
    f32vector-unfold-right f32vector-append f32vector-concatenate
    f32vector-append-subvectors reverse-list->f32vector vector->f32vector
    f32vector-comparator f32vector-copy f32vector-reverse-copy
    f32vector-empty? f32vector= f32vector-take f32vector-take-right
    f32vector-drop f32vector-drop-right f32vector-segment f32vector-fold
    f32vector-fold-right f32vector-map f32vector-map! f32vector-for-each
    f32vector-count f32vector-cumulate f32vector-take-while
    f32vector-take-while-right f32vector-drop-while
    f32vector-drop-while-right f32vector-index f32vector-index-right
    f32vector-skip f32vector-skip-right f32vector-any f32vector-every
    f32vector-partition f32vector-filter f32vector-remove f32vector-swap!
    f32vector-fill! f32vector-reverse! f32vector-copy!
    f32vector-reverse-copy! f32vector-unfold! f32vector-unfold-right!
    reverse-f32vector->list f32vector->vector make-f32vector-generator
    write-f32vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (f32? x) (numvector-element? 'f32 x))
    (define (f32vector-unfold f len seed) (numvector-unfold 'f32 f len seed))
    (define (f32vector-unfold-right f len seed)
      (numvector-unfold-right 'f32 f len seed))
    (define (f32vector-append . vs) (numvector-append 'f32 vs))
    (define (f32vector-concatenate vs) (numvector-concatenate 'f32 vs))
    (define (f32vector-append-subvectors . args)
      (numvector-append-subvectors 'f32 args))
    (define (reverse-list->f32vector lst) (reverse-list->numvector 'f32 lst))
    (define (vector->f32vector vec . opt)
      (apply vector->numvector 'f32 vec opt))
    (define f32vector-comparator (make-numvector-comparator 'f32))

    ;; The same procedure under this type's name.
    (define f32vector-copy numvector-copy)
    (define f32vector-reverse-copy numvector-reverse-copy)
    (define f32vector-empty? numvector-empty?)
    (define f32vector= numvector=)
    (define f32vector-take numvector-take)
    (define f32vector-take-right numvector-take-right)
    (define f32vector-drop numvector-drop)
    (define f32vector-drop-right numvector-drop-right)
    (define f32vector-segment numvector-segment)
    (define f32vector-fold numvector-fold)
    (define f32vector-fold-right numvector-fold-right)
    (define f32vector-map numvector-map)
    (define f32vector-map! numvector-map!)
    (define f32vector-for-each numvector-for-each)
    (define f32vector-count numvector-count)
    (define f32vector-cumulate numvector-cumulate)
    (define f32vector-take-while numvector-take-while)
    (define f32vector-take-while-right numvector-take-while-right)
    (define f32vector-drop-while numvector-drop-while)
    (define f32vector-drop-while-right numvector-drop-while-right)
    (define f32vector-index numvector-index)
    (define f32vector-index-right numvector-index-right)
    (define f32vector-skip numvector-skip)
    (define f32vector-skip-right numvector-skip-right)
    (define f32vector-any numvector-any)
    (define f32vector-every numvector-every)
    (define f32vector-partition numvector-partition)
    (define f32vector-filter numvector-filter)
    (define f32vector-remove numvector-remove)
    (define f32vector-swap! numvector-swap!)
    (define f32vector-fill! numvector-fill!)
    (define f32vector-reverse! numvector-reverse!)
    (define f32vector-copy! numvector-copy!)
    (define f32vector-reverse-copy! numvector-reverse-copy!)
    (define f32vector-unfold! numvector-unfold!)
    (define f32vector-unfold-right! numvector-unfold-right!)
    (define reverse-f32vector->list reverse-numvector->list)
    (define f32vector->vector numvector->vector)
    (define make-f32vector-generator make-numvector-generator)
    (define write-f32vector write-numvector)))
