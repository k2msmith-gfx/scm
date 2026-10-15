;;; (srfi 160 f64) — the full operation set for f64vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `f64vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `f64?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 f64)
  (export
    make-f64vector f64vector f64vector? f64vector-length f64vector-ref
    f64vector-set! f64vector->list list->f64vector f64? f64vector-unfold
    f64vector-unfold-right f64vector-append f64vector-concatenate
    f64vector-append-subvectors reverse-list->f64vector vector->f64vector
    f64vector-comparator f64vector-copy f64vector-reverse-copy
    f64vector-empty? f64vector= f64vector-take f64vector-take-right
    f64vector-drop f64vector-drop-right f64vector-segment f64vector-fold
    f64vector-fold-right f64vector-map f64vector-map! f64vector-for-each
    f64vector-count f64vector-cumulate f64vector-take-while
    f64vector-take-while-right f64vector-drop-while
    f64vector-drop-while-right f64vector-index f64vector-index-right
    f64vector-skip f64vector-skip-right f64vector-any f64vector-every
    f64vector-partition f64vector-filter f64vector-remove f64vector-swap!
    f64vector-fill! f64vector-reverse! f64vector-copy!
    f64vector-reverse-copy! f64vector-unfold! f64vector-unfold-right!
    reverse-f64vector->list f64vector->vector make-f64vector-generator
    write-f64vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (f64? x) (numvector-element? 'f64 x))
    (define (f64vector-unfold f len seed) (numvector-unfold 'f64 f len seed))
    (define (f64vector-unfold-right f len seed)
      (numvector-unfold-right 'f64 f len seed))
    (define (f64vector-append . vs) (numvector-append 'f64 vs))
    (define (f64vector-concatenate vs) (numvector-concatenate 'f64 vs))
    (define (f64vector-append-subvectors . args)
      (numvector-append-subvectors 'f64 args))
    (define (reverse-list->f64vector lst) (reverse-list->numvector 'f64 lst))
    (define (vector->f64vector vec . opt)
      (apply vector->numvector 'f64 vec opt))
    (define f64vector-comparator (make-numvector-comparator 'f64))

    ;; The same procedure under this type's name.
    (define f64vector-copy numvector-copy)
    (define f64vector-reverse-copy numvector-reverse-copy)
    (define f64vector-empty? numvector-empty?)
    (define f64vector= numvector=)
    (define f64vector-take numvector-take)
    (define f64vector-take-right numvector-take-right)
    (define f64vector-drop numvector-drop)
    (define f64vector-drop-right numvector-drop-right)
    (define f64vector-segment numvector-segment)
    (define f64vector-fold numvector-fold)
    (define f64vector-fold-right numvector-fold-right)
    (define f64vector-map numvector-map)
    (define f64vector-map! numvector-map!)
    (define f64vector-for-each numvector-for-each)
    (define f64vector-count numvector-count)
    (define f64vector-cumulate numvector-cumulate)
    (define f64vector-take-while numvector-take-while)
    (define f64vector-take-while-right numvector-take-while-right)
    (define f64vector-drop-while numvector-drop-while)
    (define f64vector-drop-while-right numvector-drop-while-right)
    (define f64vector-index numvector-index)
    (define f64vector-index-right numvector-index-right)
    (define f64vector-skip numvector-skip)
    (define f64vector-skip-right numvector-skip-right)
    (define f64vector-any numvector-any)
    (define f64vector-every numvector-every)
    (define f64vector-partition numvector-partition)
    (define f64vector-filter numvector-filter)
    (define f64vector-remove numvector-remove)
    (define f64vector-swap! numvector-swap!)
    (define f64vector-fill! numvector-fill!)
    (define f64vector-reverse! numvector-reverse!)
    (define f64vector-copy! numvector-copy!)
    (define f64vector-reverse-copy! numvector-reverse-copy!)
    (define f64vector-unfold! numvector-unfold!)
    (define f64vector-unfold-right! numvector-unfold-right!)
    (define reverse-f64vector->list reverse-numvector->list)
    (define f64vector->vector numvector->vector)
    (define make-f64vector-generator make-numvector-generator)
    (define write-f64vector write-numvector)))
