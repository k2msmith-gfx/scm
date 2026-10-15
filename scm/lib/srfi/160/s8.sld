;;; (srfi 160 s8) — the full operation set for s8vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `s8vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `s8?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 s8)
  (export
    make-s8vector s8vector s8vector? s8vector-length s8vector-ref
    s8vector-set! s8vector->list list->s8vector s8? s8vector-unfold
    s8vector-unfold-right s8vector-append s8vector-concatenate
    s8vector-append-subvectors reverse-list->s8vector vector->s8vector
    s8vector-comparator s8vector-copy s8vector-reverse-copy s8vector-empty?
    s8vector= s8vector-take s8vector-take-right s8vector-drop
    s8vector-drop-right s8vector-segment s8vector-fold s8vector-fold-right
    s8vector-map s8vector-map! s8vector-for-each s8vector-count
    s8vector-cumulate s8vector-take-while s8vector-take-while-right
    s8vector-drop-while s8vector-drop-while-right s8vector-index
    s8vector-index-right s8vector-skip s8vector-skip-right s8vector-any
    s8vector-every s8vector-partition s8vector-filter s8vector-remove
    s8vector-swap! s8vector-fill! s8vector-reverse! s8vector-copy!
    s8vector-reverse-copy! s8vector-unfold! s8vector-unfold-right!
    reverse-s8vector->list s8vector->vector make-s8vector-generator
    write-s8vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (s8? x) (numvector-element? 's8 x))
    (define (s8vector-unfold f len seed) (numvector-unfold 's8 f len seed))
    (define (s8vector-unfold-right f len seed)
      (numvector-unfold-right 's8 f len seed))
    (define (s8vector-append . vs) (numvector-append 's8 vs))
    (define (s8vector-concatenate vs) (numvector-concatenate 's8 vs))
    (define (s8vector-append-subvectors . args)
      (numvector-append-subvectors 's8 args))
    (define (reverse-list->s8vector lst) (reverse-list->numvector 's8 lst))
    (define (vector->s8vector vec . opt)
      (apply vector->numvector 's8 vec opt))
    (define s8vector-comparator (make-numvector-comparator 's8))

    ;; The same procedure under this type's name.
    (define s8vector-copy numvector-copy)
    (define s8vector-reverse-copy numvector-reverse-copy)
    (define s8vector-empty? numvector-empty?)
    (define s8vector= numvector=)
    (define s8vector-take numvector-take)
    (define s8vector-take-right numvector-take-right)
    (define s8vector-drop numvector-drop)
    (define s8vector-drop-right numvector-drop-right)
    (define s8vector-segment numvector-segment)
    (define s8vector-fold numvector-fold)
    (define s8vector-fold-right numvector-fold-right)
    (define s8vector-map numvector-map)
    (define s8vector-map! numvector-map!)
    (define s8vector-for-each numvector-for-each)
    (define s8vector-count numvector-count)
    (define s8vector-cumulate numvector-cumulate)
    (define s8vector-take-while numvector-take-while)
    (define s8vector-take-while-right numvector-take-while-right)
    (define s8vector-drop-while numvector-drop-while)
    (define s8vector-drop-while-right numvector-drop-while-right)
    (define s8vector-index numvector-index)
    (define s8vector-index-right numvector-index-right)
    (define s8vector-skip numvector-skip)
    (define s8vector-skip-right numvector-skip-right)
    (define s8vector-any numvector-any)
    (define s8vector-every numvector-every)
    (define s8vector-partition numvector-partition)
    (define s8vector-filter numvector-filter)
    (define s8vector-remove numvector-remove)
    (define s8vector-swap! numvector-swap!)
    (define s8vector-fill! numvector-fill!)
    (define s8vector-reverse! numvector-reverse!)
    (define s8vector-copy! numvector-copy!)
    (define s8vector-reverse-copy! numvector-reverse-copy!)
    (define s8vector-unfold! numvector-unfold!)
    (define s8vector-unfold-right! numvector-unfold-right!)
    (define reverse-s8vector->list reverse-numvector->list)
    (define s8vector->vector numvector->vector)
    (define make-s8vector-generator make-numvector-generator)
    (define write-s8vector write-numvector)))
