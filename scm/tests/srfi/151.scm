;;; (srfi 151) — bitwise operations.
(import (scm test) (srfi 151))

;; the logical operations, including their identities
(check and (bitwise-and 12 10) 8)
(check and-none (bitwise-and) -1)
(check and-one (bitwise-and 7) 7)
(check and-many (bitwise-and 255 60 12) 12)
(check ior (bitwise-ior 12 10) 14)
(check ior-none (bitwise-ior) 0)
(check xor (bitwise-xor 12 10) 6)
(check xor-none (bitwise-xor) 0)
(check xor-self (bitwise-xor 9 9) 0)
(check not (bitwise-not 5) -6)
(check not-not (bitwise-not (bitwise-not 5)) 5)
(check not-negative (bitwise-not -1) 0)
(check eqv-none (bitwise-eqv) -1)
(check eqv-two (bitwise-eqv 12 10) (bitwise-not (bitwise-xor 12 10)))
(check nand (bitwise-nand 12 10) -9)
(check nor (bitwise-nor 12 10) -15)
(check andc1 (bitwise-andc1 12 10) 2)
(check andc2 (bitwise-andc2 12 10) 4)
(check orc1 (bitwise-orc1 12 10) -5)
(check orc2 (bitwise-orc2 12 10) -3)
(check if (bitwise-if 3 1 8) 9)

;; negative numbers are infinite two's complement, which an i64 already is
(check and-negative (bitwise-and -1 7) 7)
(check ior-negative (bitwise-ior -2 1) -1)
(check xor-negative (bitwise-xor -1 -1) 0)

;; shifting
(check shift-left (arithmetic-shift 1 10) 1024)
(check shift-left-0 (arithmetic-shift 7 0) 7)
(check shift-right (arithmetic-shift 1024 -10) 1)
(check shift-right-truncates (arithmetic-shift 7 -1) 3)
(check shift-right-negative (arithmetic-shift -16 -2) -4)
;; A right shift saturates at the sign, which is the infinite model's
;; answer and needs no bignum.
(check shift-right-far (arithmetic-shift -1 -100) -1)
(check shift-right-far-positive (arithmetic-shift 1 -100) 0)
(check shift-left-edge (arithmetic-shift 1 62) 4611686018427387904)
;; A left shift off the end of the word **widens** as of S18, rather than
;; reporting: `exact-closed` is claimed and the exact answer exists. What
;; §4.5 refuses is a silent *wrap*, which would be a wrong answer, and
;; that has not changed -- these are exact.
(check shift-off-the-word-widens (arithmetic-shift 1 63) 9223372036854775808)
(check shift-off-the-word-is-exact (exact? (arithmetic-shift 1 63)) #t)
(check shift-far-off-the-word (arithmetic-shift 3 100)
       3802951800684688204490109616128)
;; Shifting back is the inverse, so nothing was lost on the way out.
(check shift-out-and-back (arithmetic-shift (arithmetic-shift 3 100) -100) 3)
;; But it is still bounded: unbounded means "not bounded by the word", not
;; "allowed to exhaust the host" (requirement 8).
(check shift-is-still-bounded (arithmetic-shift 1 99999999)
       '(raised "arithmetic-shift: exact integer too large (over 1048576 bits)"))
;; The logical operations stay 64 bits wide, and say so rather than
;; silently truncating a bignum.
(check bitwise-and-refuses-a-bignum (bitwise-and (arithmetic-shift 1 63) 1)
       '(raised "bitwise-and: expects exact integers, not a bignum: the bitwise operations are 64 bits wide"))
(check shift-zero-any-distance (arithmetic-shift 0 1000) 0)
(check shift-rejects-inexact (arithmetic-shift 1.0 2)
       '(raised "arithmetic-shift: expects exact integers, not the inexact 1"))

;; counting
(check count (bit-count 7) 3)
(check count-0 (bit-count 0) 0)
;; For a negative number it is the population count of the complement,
;; which is what keeps it finite under the infinite model.
(check count-negative-1 (bit-count -1) 0)
(check count-negative-2 (bit-count -2) 1)
(check length (integer-length 7) 3)
(check length-0 (integer-length 0) 0)
(check length-1 (integer-length 1) 1)
(check length-negative (integer-length -1) 0)
(check length-negative-2 (integer-length -2) 1)
(check first-set (first-set-bit 12) 2)
(check first-set-0 (first-set-bit 0) -1)
(check first-set-1 (first-set-bit 1) 0)

;; single bits
(check bit-set?-yes (bit-set? 2 12) #t)
(check bit-set?-no (bit-set? 0 12) #f)
(check bit-set?-negative (bit-set? 100 -1) #t)
(check copy-bit-on (copy-bit 1 0 #t) 2)
(check copy-bit-off (copy-bit 1 3 #f) 1)
(check copy-bit-idempotent (copy-bit 1 2 #t) 2)
(check bit-swap (bit-swap 0 2 1) 4)
(check any-bit-set?-yes (any-bit-set? 3 1) #t)
(check any-bit-set?-no (any-bit-set? 6 1) #f)
(check every-bit-set?-yes (every-bit-set? 4 6) #t)
(check every-bit-set?-no (every-bit-set? 7 6) #f)

;; bit fields
(check field (bit-field 255 2 6) 15)
(check field-low (bit-field 6 0 1) 0)
(check field-empty (bit-field 255 3 3) 0)
(check field-any?-yes (bit-field-any? 255 2 6) #t)
(check field-any?-no (bit-field-any? 0 2 6) #f)
(check field-every?-yes (bit-field-every? 255 2 6) #t)
(check field-every?-no (bit-field-every? 11 0 4) #f)
(check field-clear (bit-field-clear 255 2 6) 195)
(check field-set (bit-field-set 0 2 6) 60)
(check field-replace (bit-field-replace 255 0 2 6) 195)
(check field-replace-ones (bit-field-replace 0 15 2 6) 60)
(check field-replace-same (bit-field-replace-same 255 0 2 6) 195)
(check field-rotate (bit-field-rotate 6 1 0 3) 5)
(check field-rotate-full (bit-field-rotate 6 3 0 3) 6)
(check field-rotate-empty (bit-field-rotate 6 1 1 1) 6)
(check field-reverse (bit-field-reverse 1 0 3) 4)
(check field-reverse-twice (bit-field-reverse (bit-field-reverse 5 0 4) 0 4) 5)

;; bits as sequences, least significant first
(check bits->list (bits->list 5) '(#t #f #t))
(check bits->list-0 (bits->list 0) '())
(check bits->list-len (bits->list 5 4) '(#t #f #t #f))
(check list->bits (list->bits '(#t #f #t)) 5)
(check list->bits-empty (list->bits '()) 0)
(check bits->vector (bits->vector 5) (vector #t #f #t))
(check vector->bits (vector->bits (vector #t #f #t)) 5)
(check bits (bits #t #f #t) 5)
(check bits-roundtrip (list->bits (bits->list 12345)) 12345)

;; folding over the bits
(check fold (bitwise-fold cons '() 5) '(#t #f #t))
(check for-each
       (let ((acc '()))
         (bitwise-for-each (lambda (b) (set! acc (cons b acc))) 5)
         acc)
       '(#t #f #t))
(check unfold
       (bitwise-unfold (lambda (k) (= k 4)) even? (lambda (k) (+ k 1)) 0)
       5)
(check generator
       (let ((g (make-bitwise-generator 5))) (list (g) (g) (g)))
       '(#t #f #t))

(test-failures)
