;;; (srfi 14) — char-sets.
;;;
;;; Three things this file is really for. The **Latin-1 bound** is the
;;; library's one substantive decision, so its three different answers
;;; (membership says no, adding raises, deleting shrugs) are each pinned.
;;; The **`!` variants are non-destructive**, which is only worth doing if
;;; nothing can corrupt a shared constant, so that is asserted directly.
;;; And `char-set-fold` takes the **character first**, which is the third
;;; fold order in this project and the easiest thing here to get backwards.
(import (scm test) (srfi 14))

;; ── predicates and comparison ────────────────────────────────────────
(check char-set?-yes (char-set? (char-set #\a)) #t)
(check char-set?-no (char-set? #\a) #f)
(check char-set?-not-a-string (char-set? "a") #f)
;; Zero and one argument are #t, as for the other n-ary comparisons here.
(check char-set=-zero (char-set=) #t)
(check char-set=-one (char-set= (char-set #\a)) #t)
(check char-set=-two (char-set= (char-set #\a #\b) (char-set #\b #\a)) #t)
(check char-set=-differs (char-set= (char-set #\a) (char-set #\b)) #f)
(check char-set=-three
       (char-set= (char-set #\a) (char-set #\a) (char-set #\a))
       #t)
(check char-set=-three-last-differs
       (char-set= (char-set #\a) (char-set #\a) (char-set #\b))
       #f)
(check char-set=-empties (char-set= char-set:empty (char-set)) #t)
;; A chain of subsets, read pairwise.
(check char-set<=-zero (char-set<=) #t)
(check char-set<=-chain
       (char-set<= (char-set #\a) (char-set #\a #\b) (char-set #\a #\b #\c))
       #t)
(check char-set<=-equal (char-set<= (char-set #\a) (char-set #\a)) #t)
(check char-set<=-not (char-set<= (char-set #\a #\b) (char-set #\a)) #f)
;; Pairwise and not transitively-against-the-first: the first is a subset
;; of the third here, but the second is not a subset of the third.
(check char-set<=-is-pairwise
       (char-set<= (char-set #\a) (char-set #\a #\b #\c) (char-set #\a #\b))
       #f)
(check char-set<=-empty-is-subset (char-set<= char-set:empty (char-set #\a)) #t)
;; The hash is bounded and equal sets hash alike.
(check hash-is-bounded (< (char-set-hash char-set:digit 100) 100) #t)
(check hash-agrees-with-equality
       (= (char-set-hash (char-set #\a #\b)) (char-set-hash (char-set #\b #\a)))
       #t)
(check hash-of-empty-is-fine (char-set-hash char-set:empty 100) 0)
;; A bound of 0 asks for the default, so the result is not reduced to 0.
(check hash-bound-zero-means-default
       (= (char-set-hash char-set:digit 0) (char-set-hash char-set:digit))
       #t)
;; 256 code points through a (* 37 h) accumulator is where a numeric tower
;; would be needed, so the full set must hash without raising.
(check hash-of-full-does-not-overflow
       (exact? (char-set-hash char-set:full))
       #t)

;; ── the Latin-1 bound, which has three different answers ─────────────
;;
;; Membership for a character past the bound is #f, not an error: it
;; simply is not in the set.
(check beyond-latin1-is-not-a-member
       (char-set-contains? char-set:letter (integer->char #x3BB))
       #f)
(check beyond-latin1-not-in-full
       (char-set-contains? char-set:full (integer->char #x3BB))
       #f)
;; scm's own predicate says otherwise, which is the documented cost of
;; the bound and the reason to pass a predicate when Unicode matters.
(check native-predicate-disagrees-beyond-latin1
       (char-alphabetic? (integer->char #x3BB))
       #t)
;; Adding one raises, because it cannot be stored and silently dropping
;; it would be worse.
(check-raises adjoining-beyond-latin1-raises
              (char-set-adjoin char-set:empty (integer->char #x3BB)))
(check-raises char-set-of-beyond-latin1-raises
              (char-set (integer->char #x100)))
(check-raises string->char-set-beyond-latin1-raises
              (string->char-set "λ"))
;; Deleting one is a no-op, because the result is right either way.
(check deleting-beyond-latin1-is-a-no-op
       (char-set= (char-set-delete (char-set #\a) (integer->char #x3BB))
                  (char-set #\a))
       #t)
;; The bound is exactly 256 code points.
(check full-is-latin1 (char-set-size char-set:full) 256)
(check last-latin1-char-fits
       (char-set-contains? (char-set (integer->char 255)) (integer->char 255))
       #t)

;; ── the ! variants are non-destructive ───────────────────────────────
;;
;; The whole point of the decision: a client cannot corrupt a shared
;; constant. If these mutated, char-set:digit would grow a #\z for the
;; rest of the program.
(check adjoin!-leaves-the-constant-alone
       (let ((grown (char-set-adjoin! char-set:digit #\z)))
         (list (char-set-size grown) (char-set-size char-set:digit)))
       '(11 10))
(check delete!-leaves-its-argument-alone
       (let* ((cs (char-set #\a #\b))
              (smaller (char-set-delete! cs #\a)))
         (list (char-set->string smaller) (char-set->string cs)))
       '("b" "ab"))
(check union!-leaves-its-argument-alone
       (let* ((cs (char-set #\a))
              (u (char-set-union! cs (char-set #\b))))
         (list (char-set->string u) (char-set->string cs)))
       '("ab" "a"))
(check complement!-leaves-its-argument-alone
       (let* ((cs (char-set #\a))
              (c (char-set-complement! cs)))
         (list (char-set-contains? c #\a) (char-set-contains? cs #\a)))
       '(#f #t))
(check intersection!-leaves-its-argument-alone
       (let* ((cs (char-set #\a #\b))
              (i (char-set-intersection! cs (char-set #\b))))
         (list (char-set->string i) (char-set->string cs)))
       '("b" "ab"))
(check difference!-leaves-its-argument-alone
       (let* ((cs (char-set #\a #\b))
              (d (char-set-difference! cs (char-set #\b))))
         (list (char-set->string d) (char-set->string cs)))
       '("a" "ab"))
(check xor!-leaves-its-argument-alone
       (let* ((cs (char-set #\a #\b))
              (x (char-set-xor! cs (char-set #\b #\c))))
         (list (char-set->string x) (char-set->string cs)))
       '("ac" "ab"))
(check list->char-set!-leaves-its-base-alone
       (let* ((base (char-set #\a))
              (grown (list->char-set! '(#\b) base)))
         (list (char-set->string grown) (char-set->string base)))
       '("ab" "a"))
(check string->char-set!-leaves-its-base-alone
       (let* ((base (char-set #\a))
              (grown (string->char-set! "b" base)))
         (list (char-set->string grown) (char-set->string base)))
       '("ab" "a"))
(check filter!-leaves-its-base-alone
       (let* ((base (char-set #\x))
              (f (char-set-filter! char-numeric? char-set:digit base)))
         (list (char-set-size f) (char-set->string base)))
       '(11 "x"))
(check ucs-range->char-set!-leaves-its-base-alone
       (let* ((base (char-set #\x))
              (r (ucs-range->char-set! 97 99 #f base)))
         (list (char-set->string r) (char-set->string base)))
       '("abx" "x"))
(check unfold!-leaves-its-base-alone
       (let* ((base (char-set #\x))
              (u (char-set-unfold! (lambda (i) (= i 3))
                                   (lambda (i) (integer->char (+ 97 i)))
                                   (lambda (i) (+ i 1))
                                   0 base)))
         (list (char-set->string u) (char-set->string base)))
       '("abcx" "x"))
;; char-set-copy is still meaningful even so: it is how you get a set you
;; know nothing else holds.
(check copy-is-equal (char-set= (char-set-copy char-set:digit) char-set:digit) #t)

;; ── iterating: fold takes the CHARACTER FIRST ────────────────────────
;;
;; `(kons char state)`. This project ships three folds with three orders:
;; (srfi 1)'s fold is (f elem acc), (srfi 133)'s vector-fold is
;; (kons state elt), and this is (kons char state). Consing builds the
;; set reversed, so the other order would not even typecheck here.
(check fold-char-first
       (char-set-fold cons '() (char-set #\a #\b #\c))
       '(#\c #\b #\a))
(check fold-empty (char-set-fold cons '() char-set:empty) '())
(check fold-counts (char-set-fold (lambda (c n) (+ n 1)) 0 char-set:digit) 10)
;; Cursors: an integer index, with end-of-char-set? reading the cursor
;; alone. Walking one yields the members in code-point order.
(check cursor-walk
       (let loop ((cur (char-set-cursor char-set:digit)) (acc '()))
         (if (end-of-char-set? cur)
             (reverse acc)
             (loop (char-set-cursor-next char-set:digit cur)
                   (cons (char-set-ref char-set:digit cur) acc))))
       '(#\0 #\1 #\2 #\3 #\4 #\5 #\6 #\7 #\8 #\9))
(check cursor-of-empty-is-already-at-the-end
       (end-of-char-set? (char-set-cursor char-set:empty))
       #t)
(check-raises ref-past-the-end-raises
              (char-set-ref char-set:empty (char-set-cursor char-set:empty)))
(check for-each-visits-every-member
       (let ((acc '()))
         (char-set-for-each (lambda (c) (set! acc (cons c acc)))
                            (char-set #\a #\b))
         acc)
       '(#\b #\a))
(check map-lifts-a-char-procedure
       (char-set->string (char-set-map char-upcase (char-set #\a #\b)))
       "AB")
;; Mapping can collapse two characters onto one, and the result is a set.
(check map-can-collapse
       (char-set-size (char-set-map char-upcase (char-set #\a #\A)))
       1)

;; ── unfold: p says when to STOP ──────────────────────────────────────
;;
;; (char-set-unfold p f g seed): p is the stop predicate, f makes the
;; character, g makes the next seed. If p were read as "keep going" this
;; would produce the empty set.
(check unfold-p-is-the-stop-predicate
       (char-set->string
         (char-set-unfold (lambda (i) (= i 3))
                          (lambda (i) (integer->char (+ 97 i)))
                          (lambda (i) (+ i 1))
                          0))
       "abc")
(check unfold-stops-immediately
       (char-set-size (char-set-unfold (lambda (i) #t)
                                       (lambda (i) #\a)
                                       (lambda (i) i)
                                       0))
       0)
(check unfold-onto-a-base
       (char-set->string
         (char-set-unfold (lambda (i) (= i 2))
                          (lambda (i) (integer->char (+ 97 i)))
                          (lambda (i) (+ i 1))
                          0
                          (char-set #\z)))
       "abz")

;; ── creating ─────────────────────────────────────────────────────────
(check char-set-of-nothing (char-set-size (char-set)) 0)
(check char-set-dedups (char-set-size (char-set #\a #\a #\a)) 1)
(check list->char-set (char-set->string (list->char-set '(#\c #\a))) "ac")
(check list->char-set-with-base
       (char-set->string (list->char-set '(#\b) (char-set #\a)))
       "ab")
(check string->char-set (char-set->string (string->char-set "cab")) "abc")
(check string->char-set-empty (char-set-size (string->char-set "")) 0)
;; char-set-filter filters the DOMAIN set, not the whole character range.
(check filter-filters-the-domain
       (char-set->string (char-set-filter char-upper-case?
                                          (string->char-set "aAbB")))
       "AB")
(check filter-with-base
       (char-set->string (char-set-filter char-upper-case?
                                          (string->char-set "aA")
                                          (char-set #\z)))
       "Az")
;; ucs-range: upper is EXCLUSIVE.
(check ucs-range-upper-is-exclusive
       (char-set->string (ucs-range->char-set 97 100))
       "abc")
(check ucs-range-empty (char-set-size (ucs-range->char-set 97 97)) 0)
(check ucs-range-with-base
       (char-set->string (ucs-range->char-set 97 99 #f (char-set #\z)))
       "abz")
;; Past the bound it clips silently by default, and reports when asked.
(check ucs-range-clips-silently-by-default
       (char-set-size (ucs-range->char-set 250 400))
       6)
(check-raises ucs-range-reports-when-asked
              (ucs-range->char-set 250 400 #t))
(check ucs-range-at-the-bound-does-not-raise
       (char-set-size (ucs-range->char-set 0 256 #t))
       256)
;; ->char-set returns a char-set AS IS, so it is not a way to copy one.
(check ->char-set-of-a-char (char-set->string (->char-set #\a)) "a")
(check ->char-set-of-a-string (char-set->string (->char-set "ab")) "ab")
(check ->char-set-of-a-set-is-the-same-object
       (let ((cs (char-set #\a))) (eq? cs (->char-set cs)))
       #t)
(check-raises ->char-set-of-anything-else (->char-set 42))

;; ── querying ─────────────────────────────────────────────────────────
(check size (char-set-size (char-set #\a #\b)) 2)
(check size-empty (char-set-size char-set:empty) 0)
(check count (char-set-count char-upper-case? (string->char-set "aAbB")) 2)
(check count-none (char-set-count char-upper-case? (char-set #\a)) 0)
;; In code-point order, which this implementation promises though the SRFI
;; leaves it open.
(check ->list-is-in-code-point-order
       (char-set->list (string->char-set "cba"))
       '(#\a #\b #\c))
(check ->string-is-in-code-point-order (char-set->string (string->char-set "cba")) "abc")
(check ->list-empty (char-set->list char-set:empty) '())
(check contains-yes (char-set-contains? (char-set #\a) #\a) #t)
(check contains-no (char-set-contains? (char-set #\a) #\b) #f)
;; char-set-every answers #t, NOT the last value the predicate gave --
;; where (srfi 133)'s vector-every answers the last value. char-set-any
;; does answer the predicate's value.
(check every-answers-true-not-the-last-value
       (char-set-every (lambda (c) (char->integer c)) (char-set #\a))
       #t)
(check every-false (char-set-every char-upper-case? (string->char-set "aA")) #f)
(check any-answers-the-predicates-value
       (char-set-any (lambda (c) (and (char=? c #\a) 'found)) (char-set #\a #\b))
       'found)
(check any-none (char-set-any char-upper-case? (char-set #\a)) #f)
;; Empty set: #t and #f, the same convention as (srfi 133).
(check every-of-empty (char-set-every char-upper-case? char-set:empty) #t)
(check any-of-empty (char-set-any char-upper-case? char-set:empty) #f)

;; ── algebra ──────────────────────────────────────────────────────────
(check adjoin (char-set->string (char-set-adjoin (char-set #\a) #\b)) "ab")
(check adjoin-many
       (char-set->string (char-set-adjoin (char-set #\a) #\b #\c))
       "abc")
(check adjoin-none (char-set->string (char-set-adjoin (char-set #\a))) "a")
(check delete (char-set->string (char-set-delete (char-set #\a #\b) #\a)) "b")
(check delete-absent
       (char-set->string (char-set-delete (char-set #\a) #\z))
       "a")
;; The complement is relative to THIS implementation's range, so the
;; complement of empty is Latin-1 and not the whole of Unicode.
(check complement-of-empty-is-full
       (char-set= (char-set-complement char-set:empty) char-set:full)
       #t)
(check complement-of-full-is-empty
       (char-set= (char-set-complement char-set:full) char-set:empty)
       #t)
(check complement-is-an-involution
       (char-set= (char-set-complement (char-set-complement char-set:digit))
                  char-set:digit)
       #t)
(check complement-excludes-the-member
       (char-set-contains? (char-set-complement (char-set #\a)) #\a)
       #f)
;; The two identities differ: nothing for union, everything for
;; intersection.
(check union-of-nothing-is-empty (char-set-size (char-set-union)) 0)
(check intersection-of-nothing-is-full
       (char-set= (char-set-intersection) char-set:full)
       #t)
(check union (char-set->string (char-set-union (char-set #\a) (char-set #\b))) "ab")
(check union-of-one (char-set->string (char-set-union (char-set #\a))) "a")
(check union-n-ary
       (char-set->string (char-set-union (char-set #\a) (char-set #\b)
                                         (char-set #\c)))
       "abc")
(check intersection
       (char-set->string (char-set-intersection (char-set #\a #\b)
                                                (char-set #\b #\c)))
       "b")
(check intersection-disjoint
       (char-set-size (char-set-intersection (char-set #\a) (char-set #\b)))
       0)
;; Difference is cs1 less the union of all the rest, not a left fold of
;; pairwise differences that could re-add anything.
(check difference
       (char-set->string (char-set-difference (char-set #\a #\b) (char-set #\b)))
       "a")
(check difference-n-ary
       (char-set->string (char-set-difference (char-set #\a #\b #\c)
                                              (char-set #\b)
                                              (char-set #\c)))
       "a")
(check difference-of-one
       (char-set->string (char-set-difference (char-set #\a)))
       "a")
(check xor
       (char-set->string (char-set-xor (char-set #\a #\b) (char-set #\b #\c)))
       "ac")
(check xor-of-nothing-is-empty (char-set-size (char-set-xor)) 0)
(check xor-with-itself-is-empty
       (char-set-size (char-set-xor char-set:digit char-set:digit))
       0)
;; Two values: the difference and the intersection.
(check diff+intersection
       (call-with-values
         (lambda () (char-set-diff+intersection (char-set #\a #\b)
                                                (char-set #\b #\c)))
         (lambda (d i) (list (char-set->string d) (char-set->string i))))
       '("a" "b"))
(check diff+intersection-with-no-others
       (call-with-values
         (lambda () (char-set-diff+intersection (char-set #\a)))
         (lambda (d i) (list (char-set->string d) (char-set-size i))))
       '("a" 0))
(check diff+intersection!-also-gives-two-values
       (call-with-values
         (lambda () (char-set-diff+intersection! (char-set #\a #\b)
                                                 (char-set #\b)))
         (lambda (d i) (list (char-set->string d) (char-set->string i))))
       '("a" "b"))

;; ── the seventeen standard sets ──────────────────────────────────────
(check digit (char-set->string char-set:digit) "0123456789")
(check hex-digit (char-set->string char-set:hex-digit) "0123456789ABCDEFabcdef")
(check empty-is-empty (char-set-size char-set:empty) 0)
(check ascii-is-128 (char-set-size char-set:ascii) 128)
(check ascii-excludes-latin1
       (char-set-contains? char-set:ascii (integer->char 128))
       #f)
;; Latin-1 has no title-case characters at all.
(check title-case-is-empty (char-set-size char-set:title-case) 0)
(check letter-is-upper-and-lower-and-the-two-ordinals
       (char-set-size char-set:letter)
       117)
(check upper-case-size (char-set-size char-set:upper-case) 56)
(check lower-case-size (char-set-size char-set:lower-case) 59)
(check letter+digit-is-the-union
       (char-set= char-set:letter+digit
                  (char-set-union char-set:letter char-set:digit))
       #t)
(check graphic-is-the-union
       (char-set= char-set:graphic
                  (char-set-union char-set:letter+digit
                                  char-set:punctuation
                                  char-set:symbol))
       #t)
(check printing-is-whitespace-and-graphic
       (char-set= char-set:printing
                  (char-set-union char-set:whitespace char-set:graphic))
       #t)
(check whitespace-contents
       (char-set->list char-set:whitespace)
       (map integer->char '(#x09 #x0A #x0B #x0C #x0D #x20 #xA0)))
(check blank-contents
       (char-set->list char-set:blank)
       (map integer->char '(#x09 #x20 #xA0)))
(check iso-control-is-c0-and-c1 (char-set-size char-set:iso-control) 65)
(check iso-control-has-nul (char-set-contains? char-set:iso-control (integer->char 0)) #t)
(check iso-control-has-del (char-set-contains? char-set:iso-control (integer->char #x7F)) #t)
(check iso-control-excludes-space
       (char-set-contains? char-set:iso-control #\space)
       #f)
(check punctuation-has-ascii (char-set-contains? char-set:punctuation #\!) #t)
(check punctuation-excludes-dollar
       (char-set-contains? char-set:punctuation #\$)
       #f)
(check symbol-has-dollar (char-set-contains? char-set:symbol #\$) #t)
(check lower-case-has-micro-sign
       (char-set-contains? char-set:lower-case (integer->char #xB5))
       #t)
(check letter-has-the-ordinal-indicators
       (list (char-set-contains? char-set:letter (integer->char #xAA))
             (char-set-contains? char-set:letter (integer->char #xBA)))
       '(#t #t))
;; The division and multiplication signs sit in the middle of the Latin-1
;; letter runs and are NOT letters -- the ranges are chosen to skip them.
(check multiplication-sign-is-not-a-letter
       (char-set-contains? char-set:letter (integer->char #xD7))
       #f)
(check division-sign-is-not-a-letter
       (char-set-contains? char-set:letter (integer->char #xF7))
       #f)

;; ── where the Latin-1 data diverges from scm's own predicates ────────
;;
;; The standard sets are the reference implementation's verbatim data, so
;; three of them disagree with scm's Unicode-correct predicates INSIDE
;; Latin-1. Each is listed exactly, so that a change to either side shows
;; up here as a failure rather than as a surprise in (srfi 13).
(define (disagreements cs pred)
  (let loop ((i 255) (acc '()))
    (if (< i 0)
        acc
        (loop (- i 1)
              (if (eq? (and (char-set-contains? cs (integer->char i)) #t)
                       (and (pred (integer->char i)) #t))
                  acc
                  (cons i acc))))))

;; char-set:digit is the DECIMAL digits; char-numeric? is Unicode
;; Nd+Nl+No, so it also takes the superscripts and the vulgar fractions.
(check digit-diverges-on-superscripts-and-fractions
       (disagreements char-set:digit char-numeric?)
       '(178 179 185 188 189 190))
;; char-lower-case? follows the Unicode Lowercase property, which includes
;; the two ordinal indicators through Other_Lowercase; the reference files
;; them under char-set:letter only.
(check lower-case-diverges-on-the-ordinal-indicators
       (disagreements char-set:lower-case char-lower-case?)
       '(170 186))
;; char-whitespace? counts U+0085 NEL, a C1 control, which the
;; reference's list omits. Adding it would put a control character into
;; char-set:printing, which is whitespace union graphic.
(check whitespace-diverges-on-nel
       (disagreements char-set:whitespace char-whitespace?)
       '(133))
(check nel-is-not-printing
       (char-set-contains? char-set:printing (integer->char #x85))
       #f)
;; These two agree exactly over the whole representable range.
(check letter-agrees-with-char-alphabetic
       (disagreements char-set:letter char-alphabetic?)
       '())
(check upper-case-agrees-with-char-upper-case
       (disagreements char-set:upper-case char-upper-case?)
       '())

(test-failures)
