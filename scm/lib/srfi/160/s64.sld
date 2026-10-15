;;; (srfi 160 s64) — the full operation set for s64vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `s64vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `s64?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 s64)
  (export
    make-s64vector s64vector s64vector? s64vector-length s64vector-ref
    s64vector-set! s64vector->list list->s64vector s64? s64vector-unfold
    s64vector-unfold-right s64vector-append s64vector-concatenate
    s64vector-append-subvectors reverse-list->s64vector vector->s64vector
    s64vector-comparator s64vector-copy s64vector-reverse-copy
    s64vector-empty? s64vector= s64vector-take s64vector-take-right
    s64vector-drop s64vector-drop-right s64vector-segment s64vector-fold
    s64vector-fold-right s64vector-map s64vector-map! s64vector-for-each
    s64vector-count s64vector-cumulate s64vector-take-while
    s64vector-take-while-right s64vector-drop-while
    s64vector-drop-while-right s64vector-index s64vector-index-right
    s64vector-skip s64vector-skip-right s64vector-any s64vector-every
    s64vector-partition s64vector-filter s64vector-remove s64vector-swap!
    s64vector-fill! s64vector-reverse! s64vector-copy!
    s64vector-reverse-copy! s64vector-unfold! s64vector-unfold-right!
    reverse-s64vector->list s64vector->vector make-s64vector-generator
    write-s64vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (s64? x) (numvector-element? 's64 x))
    (define (s64vector-unfold f len seed) (numvector-unfold 's64 f len seed))
    (define (s64vector-unfold-right f len seed)
      (numvector-unfold-right 's64 f len seed))
    (define (s64vector-append . vs) (numvector-append 's64 vs))
    (define (s64vector-concatenate vs) (numvector-concatenate 's64 vs))
    (define (s64vector-append-subvectors . args)
      (numvector-append-subvectors 's64 args))
    (define (reverse-list->s64vector lst) (reverse-list->numvector 's64 lst))
    (define (vector->s64vector vec . opt)
      (apply vector->numvector 's64 vec opt))
    (define s64vector-comparator (make-numvector-comparator 's64))

    ;; The same procedure under this type's name.
    (define s64vector-copy numvector-copy)
    (define s64vector-reverse-copy numvector-reverse-copy)
    (define s64vector-empty? numvector-empty?)
    (define s64vector= numvector=)
    (define s64vector-take numvector-take)
    (define s64vector-take-right numvector-take-right)
    (define s64vector-drop numvector-drop)
    (define s64vector-drop-right numvector-drop-right)
    (define s64vector-segment numvector-segment)
    (define s64vector-fold numvector-fold)
    (define s64vector-fold-right numvector-fold-right)
    (define s64vector-map numvector-map)
    (define s64vector-map! numvector-map!)
    (define s64vector-for-each numvector-for-each)
    (define s64vector-count numvector-count)
    (define s64vector-cumulate numvector-cumulate)
    (define s64vector-take-while numvector-take-while)
    (define s64vector-take-while-right numvector-take-while-right)
    (define s64vector-drop-while numvector-drop-while)
    (define s64vector-drop-while-right numvector-drop-while-right)
    (define s64vector-index numvector-index)
    (define s64vector-index-right numvector-index-right)
    (define s64vector-skip numvector-skip)
    (define s64vector-skip-right numvector-skip-right)
    (define s64vector-any numvector-any)
    (define s64vector-every numvector-every)
    (define s64vector-partition numvector-partition)
    (define s64vector-filter numvector-filter)
    (define s64vector-remove numvector-remove)
    (define s64vector-swap! numvector-swap!)
    (define s64vector-fill! numvector-fill!)
    (define s64vector-reverse! numvector-reverse!)
    (define s64vector-copy! numvector-copy!)
    (define s64vector-reverse-copy! numvector-reverse-copy!)
    (define s64vector-unfold! numvector-unfold!)
    (define s64vector-unfold-right! numvector-unfold-right!)
    (define reverse-s64vector->list reverse-numvector->list)
    (define s64vector->vector numvector->vector)
    (define make-s64vector-generator make-numvector-generator)
    (define write-s64vector write-numvector)))
