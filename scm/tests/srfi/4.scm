;;; (srfi 4) — homogeneous numeric vectors, and the literal syntax.
(import (scm test) (srfi 4))

;; constructors and literals
(check f64-literal (f64vector 1 2.5) #f64(1.0 2.5))
(check f64-literal-exact #f64(1 2) #f64(1.0 2.0))
(check s8-literal (s8vector -1 2) #s8(-1 2))
(check u16-literal (u16vector 65535) #u16(65535))
(check s32-literal (s32vector -2147483648 2147483647) #s32(-2147483648 2147483647))
(check empty (f64vector) #f64())
(check make-fill (make-f64vector 3 1.5) #f64(1.5 1.5 1.5))
(check make-default (make-f64vector 2) #f64(0.0 0.0))
(check make-default-int (make-s32vector 2) #s32(0 0))
(check make-zero (make-f64vector 0) #f64())
(check list-> (list->f64vector '(1 2)) #f64(1.0 2.0))
(check list->-empty (list->s8vector '()) #s8())

;; `#f` still reads as false, which `#f64(` must not have broken
(check false-is-false #f #f)
(check false-list (list #f #f64(1)) (list #f #f64(1.0)))

;; predicates are per type, and a vector is not one
(check pred-yes (f64vector? #f64(1)) #t)
(check pred-wrong-type (f64vector? #s8(1)) #f)
(check pred-not-numvector (f64vector? (vector 1)) #f)
(check pred-u8-is-bytevector (u8vector? #u8(1)) #t)
(check not-a-vector (vector? #f64(1)) #f)
(check not-a-bytevector (bytevector? #f64(1)) #f)

;; length, ref, set!
(check length (f64vector-length #f64(1 2 3)) 3)
(check length-empty (f64vector-length #f64()) 0)
(check ref (f64vector-ref #f64(1.5 2.5) 1) 2.5)
(check ref-int-is-exact (s32vector-ref #s32(7 8) 0) 7)
(check set! (let ((v (make-f64vector 2 0))) (f64vector-set! v 1 9.25) v) #f64(0.0 9.25))
(check set!-exact-into-float
       (let ((v (make-f64vector 1 0))) (f64vector-set! v 0 3) v)
       #f64(3.0))
(check ref-out-of-range-raises (f64vector-ref #f64(1) 1) '(raised "numvector-ref: index 1 is out of range for a vector of length 1"))
(check ref-negative-raises (f64vector-ref #f64(1) -1) '(raised "numvector-ref: index -1 is out of range for a vector of length 1"))

;; the element type is enforced on a write
(check s8-range (let ((v (make-s8vector 1 0))) (s8vector-set! v 0 127) v) #s8(127))
(check s8-overflow (let ((v (make-s8vector 1 0))) (s8vector-set! v 0 128) v)
       '(raised "128 is out of range for a s8vector"))
(check u16-negative (let ((v (make-u16vector 1 0))) (u16vector-set! v 0 -1) v)
       '(raised "-1 is out of range for a u16vector"))
(check int-rejects-inexact
       (let ((v (make-s32vector 1 0))) (s32vector-set! v 0 1.5) v)
       '(raised "a s32vector holds exact integers, not the inexact 1.5"))
(check make-checks-fill (make-s8vector 2 999) '(raised "999 is out of range for a s8vector"))

;; conversions, with the optional range
(check ->list (f64vector->list #f64(1 2 3)) '(1.0 2.0 3.0))
(check ->list-start (f64vector->list #f64(1 2 3) 1) '(2.0 3.0))
(check ->list-range (f64vector->list #f64(1 2 3 4) 1 3) '(2.0 3.0))
(check ->list-empty-range (f64vector->list #f64(1 2) 1 1) '())
(check roundtrip (list->f64vector (f64vector->list #f64(1 2 3))) #f64(1.0 2.0 3.0))

;; u8 is the bytevector
(check u8-is-bytevector (u8vector 1 2) #u8(1 2))
(check u8-ref (u8vector-ref #u8(7 8) 1) 8)
(check u8-set! (let ((v (make-u8vector 2 0))) (u8vector-set! v 0 255) v) #u8(255 0))
(check u8->list (u8vector->list #u8(1 2 3) 1) '(2 3))
(check u8-list-> (list->u8vector '(1 2)) #u8(1 2))

;; equal? is structural within a type and false across types
(check equal-same (equal? #f64(1 2) #f64(1 2)) #t)
(check equal-diff (equal? #f64(1 2) #f64(1 3)) #f)
(check equal-cross-type (equal? #f64(1 2) #f32(1 2)) #f)
(check equal-u8-vs-f64 (equal? #u8(1 2) #f64(1 2)) #f)
(check eqv-identity (let ((v #f64(1))) (eqv? v v)) #t)
(check eqv-distinct (eqv? #f64(1) #f64(1)) #f)

;; the generic operations, which are globals rather than exports
(check generic-copy (numvector-copy #f64(1 2 3) 1) #f64(2.0 3.0))
(check generic-copy-all (numvector-copy #s8(1 2)) #s8(1 2))
(check generic-copy!
       (let ((to (make-f64vector 4 0)))
         (numvector-copy! to 1 #f64(7 8))
         to)
       #f64(0.0 7.0 8.0 0.0))
(check generic-copy!-overlap
       (let ((v (list->f64vector '(1 2 3 4))))
         (numvector-copy! v 1 v 0 3)
         v)
       #f64(1.0 1.0 2.0 3.0))
(check generic-copy!-too-big
       (let ((to (make-f64vector 2 0))) (numvector-copy! to 1 #f64(7 8)) to)
       '(raised "numvector-copy!: 2 elements at 1 do not fit a vector of length 2"))
(check generic-fill!
       (let ((v (make-s16vector 3 0))) (numvector-fill! v 5 1) v)
       #s16(0 5 5))
(check generic-tag (numvector-tag #f32(1)) 'f32)
(check generic-any (list (numvector? #f64(1)) (numvector? (vector 1))) '(#t #f))

;; f32 rounds to single precision, which is the point of asking for it
(check f32-rounds (= (f32vector-ref (f32vector 0.1) 0) 0.1) #f)
(check f32-exact-halves (f32vector-ref (f32vector 0.5) 0) 0.5)

;; a big one, to say the storage is real
(check big (let ((v (make-f64vector 100000 0)))
             (f64vector-set! v 99999 1.5)
             (list (f64vector-length v) (f64vector-ref v 99999) (f64vector-ref v 0)))
       '(100000 1.5 0.0))

(test-failures)
