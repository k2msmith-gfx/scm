;;; (srfi 160 s32) — the full operation set for s32vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `s32vector-map` and `f64vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `s32?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 s32)
  (export
    make-s32vector s32vector s32vector? s32vector-length s32vector-ref
    s32vector-set! s32vector->list list->s32vector s32? s32vector-unfold
    s32vector-unfold-right s32vector-append s32vector-concatenate
    s32vector-append-subvectors reverse-list->s32vector vector->s32vector
    s32vector-comparator s32vector-copy s32vector-reverse-copy
    s32vector-empty? s32vector= s32vector-take s32vector-take-right
    s32vector-drop s32vector-drop-right s32vector-segment s32vector-fold
    s32vector-fold-right s32vector-map s32vector-map! s32vector-for-each
    s32vector-count s32vector-cumulate s32vector-take-while
    s32vector-take-while-right s32vector-drop-while
    s32vector-drop-while-right s32vector-index s32vector-index-right
    s32vector-skip s32vector-skip-right s32vector-any s32vector-every
    s32vector-partition s32vector-filter s32vector-remove s32vector-swap!
    s32vector-fill! s32vector-reverse! s32vector-copy!
    s32vector-reverse-copy! s32vector-unfold! s32vector-unfold-right!
    reverse-s32vector->list s32vector->vector make-s32vector-generator
    write-s32vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (s32? x) (numvector-element? 's32 x))
    (define (s32vector-unfold f len seed) (numvector-unfold 's32 f len seed))
    (define (s32vector-unfold-right f len seed)
      (numvector-unfold-right 's32 f len seed))
    (define (s32vector-append . vs) (numvector-append 's32 vs))
    (define (s32vector-concatenate vs) (numvector-concatenate 's32 vs))
    (define (s32vector-append-subvectors . args)
      (numvector-append-subvectors 's32 args))
    (define (reverse-list->s32vector lst) (reverse-list->numvector 's32 lst))
    (define (vector->s32vector vec . opt)
      (apply vector->numvector 's32 vec opt))
    (define s32vector-comparator (make-numvector-comparator 's32))

    ;; The same procedure under this type's name.
    (define s32vector-copy numvector-copy)
    (define s32vector-reverse-copy numvector-reverse-copy)
    (define s32vector-empty? numvector-empty?)
    (define s32vector= numvector=)
    (define s32vector-take numvector-take)
    (define s32vector-take-right numvector-take-right)
    (define s32vector-drop numvector-drop)
    (define s32vector-drop-right numvector-drop-right)
    (define s32vector-segment numvector-segment)
    (define s32vector-fold numvector-fold)
    (define s32vector-fold-right numvector-fold-right)
    (define s32vector-map numvector-map)
    (define s32vector-map! numvector-map!)
    (define s32vector-for-each numvector-for-each)
    (define s32vector-count numvector-count)
    (define s32vector-cumulate numvector-cumulate)
    (define s32vector-take-while numvector-take-while)
    (define s32vector-take-while-right numvector-take-while-right)
    (define s32vector-drop-while numvector-drop-while)
    (define s32vector-drop-while-right numvector-drop-while-right)
    (define s32vector-index numvector-index)
    (define s32vector-index-right numvector-index-right)
    (define s32vector-skip numvector-skip)
    (define s32vector-skip-right numvector-skip-right)
    (define s32vector-any numvector-any)
    (define s32vector-every numvector-every)
    (define s32vector-partition numvector-partition)
    (define s32vector-filter numvector-filter)
    (define s32vector-remove numvector-remove)
    (define s32vector-swap! numvector-swap!)
    (define s32vector-fill! numvector-fill!)
    (define s32vector-reverse! numvector-reverse!)
    (define s32vector-copy! numvector-copy!)
    (define s32vector-reverse-copy! numvector-reverse-copy!)
    (define s32vector-unfold! numvector-unfold!)
    (define s32vector-unfold-right! numvector-unfold-right!)
    (define reverse-s32vector->list reverse-numvector->list)
    (define s32vector->vector numvector->vector)
    (define make-s32vector-generator make-numvector-generator)
    (define write-s32vector write-numvector)))
