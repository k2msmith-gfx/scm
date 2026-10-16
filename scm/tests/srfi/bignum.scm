;;; Unbounded exact integers (S18, docs/numeric-tower.md).
;;;
;;; Not a SRFI, but gated the same way and on both tiers, because the
;;; property that matters most is that the tiers agree on exact arithmetic
;;; past the word exactly as they agree on everything else.
;;;
;;; The invariant this file is really about is **one integer, one
;;; representation**: a result that fits comes back a fixnum, so `eqv?`,
;;; `equal?`, `=` and `equal-hash` can never disagree about the same value.
(import (scm test))

(define BIG (* 9223372036854775807 4))          ; 2^65 - 4, a bignum
(define TWO64 (expt 2 64))
(define NEG (- 0 BIG))

;; ── widening, and coming back down ───────────────────────────────────
(check overflow-widens (* 9223372036854775807 2) 18446744073709551614)
(check widened-is-exact (exact? (* 9223372036854775807 2)) #t)
(check widened-is-an-integer (integer? (* 9223372036854775807 2)) #t)
(check widened-is-an-exact-integer (exact-integer? (* 9223372036854775807 2)) #t)
(check add-widens (+ 9223372036854775807 1) 9223372036854775808)
(check sub-widens (- -9223372036854775808 1) -9223372036854775809)
;; **Normalisation**: a result that fits is a fixnum again, so there is one
;; representation per integer. Without this every comparison below could
;; disagree with itself.
(check narrows-back (- (+ 9223372036854775807 1) 1) 9223372036854775807)
(check narrows-back-is-exact (exact? (- (+ 9223372036854775807 1) 1)) #t)
(check narrows-to-zero (- BIG BIG) 0)
(check narrows-to-small (quotient TWO64 (expt 2 63)) 2)
;; eqv? must agree with = on two exact numbers however they are stored.
(check eqv-on-two-bignums (eqv? BIG (* 9223372036854775807 4)) #t)
(check equal-on-two-bignums (equal? BIG (* 9223372036854775807 4)) #t)
(check eq-on-two-bignums (= BIG (* 9223372036854775807 4)) #t)
(check eqv-distinguishes (eqv? BIG TWO64) #f)
(check a-bignum-is-not-a-fixnum (= BIG 4) #f)
(check hashes-agree (= (equal-hash BIG) (equal-hash (* 9223372036854775807 4))) #t)

;; ── the four arithmetic operations ───────────────────────────────────
(check mul-big (* TWO64 TWO64) 340282366920938463463374607431768211456)
(check add-big (+ TWO64 TWO64) 36893488147419103232)
(check sub-big (- TWO64 1) 18446744073709551615)
(check neg-big (- 0 TWO64) -18446744073709551616)
(check abs-big (abs NEG) BIG)
;; `(abs i64::MIN)` had no answer before and has one now.
(check abs-of-the-most-negative-fixnum (abs -9223372036854775808) 9223372036854775808)
(check square-big (square TWO64) 340282366920938463463374607431768211456)
(check expt-big (expt 2 100) 1267650600228229401496703205376)
;; 1024^7 is 2^70, not 2^100 -- worth leaving as a reminder that `expt`
;; of an exact base is exact at any size.
(check expt-of-a-bignum (expt (expt 2 10) 7) 1180591620717411303424)
(check expt-zero (expt 0 0) 1)
(check factorial-30
       (let loop ((i 1) (acc 1)) (if (> i 30) acc (loop (+ i 1) (* acc i))))
       265252859812191058636308480000000)

;; ── division keeps Scheme's signs ────────────────────────────────────
(check quotient-big (quotient TWO64 (expt 2 32)) 4294967296)
(check quotient-truncates (quotient (- 0 (+ TWO64 1)) TWO64) -1)
(check remainder-takes-the-dividend-sign (remainder (- 0 (+ TWO64 1)) TWO64) -1)
(check modulo-takes-the-divisor-sign (modulo (- 0 (+ TWO64 1)) TWO64) 18446744073709551615)
(check modulo-big-positive (modulo (+ TWO64 5) TWO64) 5)
(check-raises quotient-by-zero (quotient TWO64 0))
(check floor-quotient-floors (floor-quotient (- 0 (+ TWO64 1)) TWO64) -2)
(check floor-remainder (floor-remainder (- 0 (+ TWO64 1)) TWO64) 18446744073709551615)
(check truncate-div
       (call-with-values (lambda () (truncate/ (+ TWO64 5) TWO64))
                         (lambda (q r) (list q r)))
       '(1 5))
(check floor-div
       (call-with-values (lambda () (floor/ (- 0 (+ TWO64 5)) TWO64))
                         (lambda (q r) (list q r)))
       '(-2 18446744073709551611))
;; `/` is exact when it divides evenly and inexact otherwise, which is the
;; `ratios` decision holding at bignum size too.
(check exact-division (/ TWO64 (expt 2 32)) 4294967296)
(check inexact-division (/ TWO64 3) 6.148914691236517e18)
(check division-stays-exact-when-even (exact? (/ TWO64 (expt 2 32))) #t)

;; ── gcd, lcm, sqrt ──────────────────────────────────────────────────
(check gcd-big (gcd TWO64 (expt 2 40)) 1099511627776)
(check gcd-with-a-fixnum (gcd TWO64 6) 2)
(check gcd-is-non-negative (gcd NEG 6) 2)
(check lcm-big (lcm TWO64 (expt 2 40)) 18446744073709551616)
(check lcm-with-zero (lcm TWO64 0) 0)
(check exact-integer-sqrt-big
       (call-with-values (lambda () (exact-integer-sqrt TWO64))
                         (lambda (r rem) (list r rem)))
       '(4294967296 0))
;; The defining property, on a value that is not a perfect square.
(check exact-integer-sqrt-is-the-floor
       (call-with-values (lambda () (exact-integer-sqrt (+ TWO64 1)))
                         (lambda (r rem) (= (+ (* r r) rem) (+ TWO64 1))))
       #t)
(check sqrt-of-a-bignum-is-inexact (inexact? (sqrt TWO64)) #t)

;; ── ordering is exact, not through a double ─────────────────────────
;;
;; Past 2^53 a double cannot tell these apart, so comparing via `f64`
;; would make them equal. This is the mistake `=` already guarded against
;; for fixnums with i128, which a bignum makes unguardable that way.
(check adjacent-bignums-are-not-equal (= TWO64 (+ TWO64 1)) #f)
(check adjacent-bignums-order (< TWO64 (+ TWO64 1)) #t)
(check adjacent-bignums-order-right (> (+ TWO64 1) TWO64) #t)
(check big-against-fixnum (< 5 TWO64) #t)
(check negative-big-against-fixnum (< NEG 5) #t)
(check big-chain (< 1 TWO64 (* TWO64 2)) #t)
(check min-picks-the-fixnum (min TWO64 1) 1)
(check max-picks-the-bignum (max TWO64 1) TWO64)
;; Against a flonum, compared exactly by splitting at the integral part.
(check big-against-a-flonum (< TWO64 1e30) #t)
(check big-against-infinity (< TWO64 (/ 1.0 0.0)) #t)
(check big-against-negative-infinity (> TWO64 (/ -1.0 0.0)) #t)
(check nan-compares-false (< TWO64 (/ 0.0 0.0)) #f)
(check a-bignum-equals-its-own-double (= TWO64 1.8446744073709552e19) #t)

;; ── predicates ──────────────────────────────────────────────────────
(check zero?-big (zero? TWO64) #f)
(check positive?-big (positive? TWO64) #t)
(check negative?-big (negative? NEG) #t)
(check even?-big (even? TWO64) #t)
(check odd?-big (odd? (+ TWO64 1)) #t)
(check exact?-big (exact? TWO64) #t)
(check inexact?-big (inexact? TWO64) #f)
(check number?-big (number? TWO64) #t)
(check rational?-big (rational? TWO64) #t)
(check real?-big (real? TWO64) #t)
(check floor-is-the-identity (floor TWO64) TWO64)
(check round-is-the-identity (round TWO64) TWO64)
(check numerator-is-itself (numerator TWO64) TWO64)
(check denominator-is-one (denominator TWO64) 1)

;; ── conversion, both ways ───────────────────────────────────────────
(check exact->inexact-big (inexact TWO64) 1.8446744073709552e19)
(check inexact->exact-big (exact 1e30) 1000000000000000019884624838656)
(check exact-of-a-double-is-what-the-double-really-is
       (= (exact 1e30) 1000000000000000019884624838656)
       #t)
(check number->string-big (number->string TWO64) "18446744073709551616")
(check number->string-radix-16 (number->string TWO64 16) "10000000000000000")
(check number->string-radix-2 (number->string (expt 2 70) 2)
       "10000000000000000000000000000000000000000000000000000000000000000000000")
(check string->number-big (string->number "18446744073709551616") TWO64)
(check string->number-negative-big (string->number "-18446744073709551616")
       (- 0 TWO64))
(check string->number-radix-16 (string->number "10000000000000000" 16) TWO64)
;; Read then written then read again is the property most likely to break,
;; the reader and the writer being two independent halves.
(check round-trip (string->number (number->string (expt 7 50))) (expt 7 50))
;; Twenty-six nines, which is 10^26 - 1.
(check literal-reads 99999999999999999999999999 (- (expt 10 26) 1))
(check negative-literal-reads -99999999999999999999999999
       (- 1 (expt 10 26)))
(check hex-literal-reads #x10000000000000000 TWO64)
(check exactness-prefix-reads #e1e30 1000000000000000019884624838656)
(check inexact-prefix-on-a-wide-literal
       (inexact? #i99999999999999999999999999)
       #t)

;; ── shifting widens; the logical operations do not ──────────────────
(check shift-left-widens (arithmetic-shift 1 64) TWO64)
(check shift-right-narrows (arithmetic-shift TWO64 -64) 1)
(check shift-round-trip (arithmetic-shift (arithmetic-shift 3 100) -100) 3)
;; A right shift is floor division, so a negative value rounds away from
;; zero -- the two's complement reading.
(check shift-right-of-a-negative-floors (arithmetic-shift (- 0 TWO64 1) -64) -2)
(check integer-length-big (integer-length TWO64) 65)
(check integer-length-of-a-negative-big (integer-length (- 0 TWO64)) 64)

;; ── bounded, because unbounded is not a licence to exhaust the host ─
(check-raises expt-too-large (expt 2 1099511627776))
(check-raises shift-too-large (arithmetic-shift 1 99999999))

(test-failures)
