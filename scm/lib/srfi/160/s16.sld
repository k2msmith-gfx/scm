;;; (srfi 160 s16) — the full operation set for s16vectors.
;;;
;;; SRFI 160 gives each element type about sixty operations. Nearly all of
;;; them are `(scm numvector)`'s, under a per-type name: a numeric vector
;;; carries its own element type, so `s16vector-map` and `s32vector-map`
;;; are the same procedure and there is one implementation of each
;;; operation rather than ten. The nine that must be told the type are the
;;; ones with no input vector to ask -- the constructors, the element
;;; predicate `s16?` and the comparator.
;;;
;;; The eight core operations come from `(srfi 4)`, which is also what
;;; `(srfi 160 base)` exports, so importing this library beside either of
;;; those is harmless -- the name means the same binding.
(define-library (srfi 160 s16)
  (export
    make-s16vector s16vector s16vector? s16vector-length s16vector-ref
    s16vector-set! s16vector->list list->s16vector s16? s16vector-unfold
    s16vector-unfold-right s16vector-append s16vector-concatenate
    s16vector-append-subvectors reverse-list->s16vector vector->s16vector
    s16vector-comparator s16vector-copy s16vector-reverse-copy
    s16vector-empty? s16vector= s16vector-take s16vector-take-right
    s16vector-drop s16vector-drop-right s16vector-segment s16vector-fold
    s16vector-fold-right s16vector-map s16vector-map! s16vector-for-each
    s16vector-count s16vector-cumulate s16vector-take-while
    s16vector-take-while-right s16vector-drop-while
    s16vector-drop-while-right s16vector-index s16vector-index-right
    s16vector-skip s16vector-skip-right s16vector-any s16vector-every
    s16vector-partition s16vector-filter s16vector-remove s16vector-swap!
    s16vector-fill! s16vector-reverse! s16vector-copy!
    s16vector-reverse-copy! s16vector-unfold! s16vector-unfold-right!
    reverse-s16vector->list s16vector->vector make-s16vector-generator
    write-s16vector)
  (import (scheme base) (srfi 4) (scm numvector))
  (begin

    ;; Told the element type, because there is no vector to ask.
    (define (s16? x) (numvector-element? 's16 x))
    (define (s16vector-unfold f len seed) (numvector-unfold 's16 f len seed))
    (define (s16vector-unfold-right f len seed)
      (numvector-unfold-right 's16 f len seed))
    (define (s16vector-append . vs) (numvector-append 's16 vs))
    (define (s16vector-concatenate vs) (numvector-concatenate 's16 vs))
    (define (s16vector-append-subvectors . args)
      (numvector-append-subvectors 's16 args))
    (define (reverse-list->s16vector lst) (reverse-list->numvector 's16 lst))
    (define (vector->s16vector vec . opt)
      (apply vector->numvector 's16 vec opt))
    (define s16vector-comparator (make-numvector-comparator 's16))

    ;; The same procedure under this type's name.
    (define s16vector-copy numvector-copy)
    (define s16vector-reverse-copy numvector-reverse-copy)
    (define s16vector-empty? numvector-empty?)
    (define s16vector= numvector=)
    (define s16vector-take numvector-take)
    (define s16vector-take-right numvector-take-right)
    (define s16vector-drop numvector-drop)
    (define s16vector-drop-right numvector-drop-right)
    (define s16vector-segment numvector-segment)
    (define s16vector-fold numvector-fold)
    (define s16vector-fold-right numvector-fold-right)
    (define s16vector-map numvector-map)
    (define s16vector-map! numvector-map!)
    (define s16vector-for-each numvector-for-each)
    (define s16vector-count numvector-count)
    (define s16vector-cumulate numvector-cumulate)
    (define s16vector-take-while numvector-take-while)
    (define s16vector-take-while-right numvector-take-while-right)
    (define s16vector-drop-while numvector-drop-while)
    (define s16vector-drop-while-right numvector-drop-while-right)
    (define s16vector-index numvector-index)
    (define s16vector-index-right numvector-index-right)
    (define s16vector-skip numvector-skip)
    (define s16vector-skip-right numvector-skip-right)
    (define s16vector-any numvector-any)
    (define s16vector-every numvector-every)
    (define s16vector-partition numvector-partition)
    (define s16vector-filter numvector-filter)
    (define s16vector-remove numvector-remove)
    (define s16vector-swap! numvector-swap!)
    (define s16vector-fill! numvector-fill!)
    (define s16vector-reverse! numvector-reverse!)
    (define s16vector-copy! numvector-copy!)
    (define s16vector-reverse-copy! numvector-reverse-copy!)
    (define s16vector-unfold! numvector-unfold!)
    (define s16vector-unfold-right! numvector-unfold-right!)
    (define reverse-s16vector->list reverse-numvector->list)
    (define s16vector->vector numvector->vector)
    (define make-s16vector-generator make-numvector-generator)
    (define write-s16vector write-numvector)))
