;;; (srfi 13) — strings.
;;;
;;; The library predates R7RS and is inconsistent with itself about
;;; argument order, so most of what this file does is pin orders. Three
;;; groups matter: where the **criterion** goes (first in some procedures,
;;; second in others), which end `string-pad` **truncates**, and the fact
;;; that the comparisons answer a **mismatch index** rather than `#t`.
(import (scm test) (srfi 13) (srfi 14))

;; ── predicates ───────────────────────────────────────────────────────
(check null-yes (string-null? "") #t)
(check null-no (string-null? "a") #f)
;; CRITERION FIRST for every? and any?. Both answer the predicate's value,
;; not #t -- every? the last one's, any? the matching one's.
(check every-answers-the-last-value
       (string-every (lambda (c) (char->integer c)) "ab")
       98)
(check every-true (string-every char-alphabetic? "abc") #t)
(check every-false (string-every char-alphabetic? "ab1") #f)
(check every-of-empty (string-every char-alphabetic? "") #t)
(check any-answers-the-matching-value
       (string-any (lambda (c) (and (char-numeric? c) c)) "ab1c")
       #\1)
(check any-false (string-any char-numeric? "abc") #f)
(check any-of-empty (string-any char-alphabetic? "") #f)
;; A char, a char-set and a predicate are all accepted as the criterion.
(check every-with-a-char (string-every #\a "aaa") #t)
(check any-with-a-char-set
       (string-any (string->char-set "xyz") "abx")
       #t)
(check-raises criterion-must-be-one-of-the-three (string-every 42 "abc"))
;; Ranges.
(check every-in-a-range (string-every char-numeric? "ab12" 2 4) #t)
(check any-in-a-range (string-any char-numeric? "12ab" 2 4) #f)

;; ── constructors and conversion ──────────────────────────────────────
(check tabulate (string-tabulate (lambda (i) (integer->char (+ 97 i))) 3) "abc")
(check tabulate-empty (string-tabulate (lambda (i) #\a) 0) "")
(check reverse-list->string (reverse-list->string '(#\a #\b #\c)) "cba")
(check reverse-list->string-empty (reverse-list->string '()) "")
;; string-join: the delimiter defaults to one space and the grammar to
;; infix.
(check join-default-delimiter (string-join '("a" "b")) "a b")
(check join-infix (string-join '("a" "b" "c") "-") "a-b-c")
(check join-infix-one (string-join '("a") "-") "a")
(check join-prefix (string-join '("a" "b") "-" 'prefix) "-a-b")
(check join-suffix (string-join '("a" "b") "-" 'suffix) "a-b-")
;; The empty list is the only place infix and strict-infix differ, which
;; is the whole reason strict-infix exists.
(check join-empty-infix (string-join '() "-") "")
(check join-empty-prefix (string-join '() "-" 'prefix) "")
(check join-empty-suffix (string-join '() "-" 'suffix) "")
(check-raises join-empty-strict-infix (string-join '() "-" 'strict-infix))
(check join-non-empty-strict-infix
       (string-join '("a" "b") "-" 'strict-infix)
       "a-b")
(check-raises join-unknown-grammar (string-join '("a") "-" 'around))

;; ── selection ────────────────────────────────────────────────────────
(check take (string-take "hello" 2) "he")
(check take-all (string-take "hi" 2) "hi")
(check take-none (string-take "hi" 0) "")
(check drop (string-drop "hello" 2) "llo")
(check take-right (string-take-right "hello" 2) "lo")
(check drop-right (string-drop-right "hello" 2) "hel")
;; substring/shared is allowed to share and here copies, so the result is
;; equal but not required to be eq?.
(check substring/shared (substring/shared "hello" 1 3) "el")
(check substring/shared-to-the-end (substring/shared "hello" 3) "lo")
;; string-pad pads on the LEFT, and when the string is too long it keeps
;; the RIGHTMOST characters -- it truncates on the side it pads. The other
;; reading would give "hel" here.
(check pad-pads-on-the-left (string-pad "42" 5) "   42")
(check pad-truncates-on-the-left (string-pad "hello" 3) "llo")
(check pad-with-a-char (string-pad "42" 5 #\0) "00042")
(check pad-exact (string-pad "hello" 5) "hello")
(check pad-in-a-range (string-pad "hello" 2 #\space 0 3) "el")
;; string-pad-right is the mirror image in both respects.
(check pad-right-pads-on-the-right (string-pad-right "42" 5) "42   ")
(check pad-right-truncates-on-the-right (string-pad-right "hello" 3) "hel")
(check pad-right-with-a-char (string-pad-right "42" 5 #\0) "42000")

;; ── trimming: STRING FIRST, and the default is char-set:whitespace ───
(check trim (string-trim "  hi  ") "hi  ")
(check trim-right (string-trim-right "  hi  ") "  hi")
(check trim-both (string-trim-both "  hi  ") "hi")
(check trim-nothing-to-do (string-trim-both "hi") "hi")
(check trim-all-whitespace (string-trim-both "   ") "")
(check trim-empty (string-trim-both "") "")
;; The default really is the SRFI 14 set, so a tab and a newline go too.
(check trim-default-covers-tab-and-newline
       (string-trim-both "\t\n hi \n\t")
       "hi")
(check trim-with-a-char (string-trim-both "xxhixx" #\x) "hi")
(check trim-with-a-char-set
       (string-trim-both "--hi--" (string->char-set "-"))
       "hi")
(check trim-with-a-predicate (string-trim-both "12hi34" char-numeric?) "hi")
(check trim-in-a-range (string-trim "  hi" char-whitespace? 1 4) "hi")

;; ── comparison: the answer is a MISMATCH INDEX, not #t ───────────────
;;
;; Every one of these but string= answers the index where the strings
;; diverge, which is a true value and also tells you where.
(check string<-answers-the-index (string< "ab" "ac") 1)
(check string>-answers-the-index (string> "ac" "ab") 1)
(check string<>-answers-the-index (string<> "ab" "ac") 1)
(check string<-false-when-greater (string< "ac" "ab") #f)
;; string= has no index worth pointing at, so it answers #t.
(check string=-answers-true (string= "ab" "ab") #t)
(check string=-false (string= "ab" "ac") #f)
(check string<>-false-when-equal (string<> "ab" "ab") #f)
;; A prefix is less than what extends it, and the index handed over is
;; end1 because there is no differing character.
(check shorter-is-less (string< "ab" "abc") 2)
(check longer-is-greater (string> "abc" "ab") 2)
;; The index is one past the common prefix in EVERY case, including this
;; one -- reading the spec's "end1 in the case of equality" as a separate
;; rule would report 3 here, where the strings actually diverge at 2.
(check greater-reports-where-the-shorter-ran-out
       (string-compare "abc" "ab" (lambda (i) (list 'lt i))
                                  (lambda (i) (list 'eq i))
                                  (lambda (i) (list 'gt i)))
       '(gt 2))
(check less-reports-where-the-shorter-ran-out
       (string-compare "ab" "abc" (lambda (i) (list 'lt i))
                                  (lambda (i) (list 'eq i))
                                  (lambda (i) (list 'gt i)))
       '(lt 2))
(check string<=-when-equal (string<= "ab" "ab") 2)
(check string<=-when-less (string<= "ab" "ac") 1)
(check string<=-false (string<= "ac" "ab") #f)
(check string>=-when-equal (string>= "ab" "ab") 2)
(check string>=-when-greater (string>= "ac" "ab") 1)
(check empty-strings-are-equal (string= "" "") #t)
(check empty-is-less-than-anything (string< "" "a") 0)
;; The ci forms fold case.
(check string-ci= (string-ci= "AB" "ab") #t)
(check string-ci<> (string-ci<> "AB" "ab") #f)
(check string-ci< (string-ci< "AB" "ac") 1)
(check string-ci>= (string-ci>= "AB" "ab") 2)
;; Ranges are start1 end1 start2 end2.
(check comparison-with-ranges (string= "xabx" "yaby" 1 3 1 3) #t)
;; string-compare applies one of three procedures to the mismatch index,
;; and for proc= the index is end1.
(check compare-less
       (string-compare "abc" "abd" (lambda (i) (list 'lt i))
                                   (lambda (i) (list 'eq i))
                                   (lambda (i) (list 'gt i)))
       '(lt 2))
(check compare-greater
       (string-compare "abd" "abc" (lambda (i) (list 'lt i))
                                   (lambda (i) (list 'eq i))
                                   (lambda (i) (list 'gt i)))
       '(gt 2))
(check compare-equal-gets-end1
       (string-compare "abc" "abc" (lambda (i) (list 'lt i))
                                   (lambda (i) (list 'eq i))
                                   (lambda (i) (list 'gt i)))
       '(eq 3))
(check compare-ci-folds
       (string-compare-ci "ABC" "abc" (lambda (i) 'lt)
                                      (lambda (i) 'eq)
                                      (lambda (i) 'gt))
       'eq)

;; ── hashing ──────────────────────────────────────────────────────────
(check hash-is-bounded (< (string-hash "hello" 100) 100) #t)
(check hash-agrees-with-itself
       (= (string-hash "hello") (string-hash "hello"))
       #t)
(check hash-bound-zero-means-default
       (= (string-hash "hello" 0) (string-hash "hello"))
       #t)
(check hash-ci-folds
       (= (string-hash-ci "HELLO") (string-hash-ci "hello"))
       #t)
(check hash-of-empty (string-hash "" 100) 0)
;; A long string must not overflow the accumulator for want of bignums.
(check hash-of-a-long-string-does-not-overflow
       (exact? (string-hash (make-string 500 #\x)))
       #t)
(check hash-in-a-range
       (= (string-hash "xhellox" 1000 1 6) (string-hash "hello" 1000))
       #t)

;; ── prefixes and suffixes ────────────────────────────────────────────
(check prefix-length (string-prefix-length "about" "abort") 3)
(check prefix-length-none (string-prefix-length "xyz" "abc") 0)
(check prefix-length-all (string-prefix-length "ab" "ab") 2)
(check suffix-length (string-suffix-length "cutting" "setting") 5)
(check suffix-length-none (string-suffix-length "abc" "xyz") 0)
(check prefix-length-ci (string-prefix-length-ci "ABOUT" "abort") 3)
(check suffix-length-ci (string-suffix-length-ci "CUTTING" "setting") 5)
;; The FIRST argument is the one being looked for.
(check prefix?-yes (string-prefix? "ab" "abc") #t)
(check prefix?-no (string-prefix? "abc" "ab") #f)
(check prefix?-equal (string-prefix? "ab" "ab") #t)
(check prefix?-empty-is-a-prefix (string-prefix? "" "abc") #t)
(check suffix?-yes (string-suffix? "bc" "abc") #t)
(check suffix?-no (string-suffix? "ab" "abc") #f)
(check prefix-ci?-yes (string-prefix-ci? "AB" "abc") #t)
(check suffix-ci?-yes (string-suffix-ci? "BC" "abc") #t)

;; ── searching: STRING FIRST, unlike every/any/filter/delete ──────────
(check index (string-index "hello" #\l) 2)
(check index-none (string-index "hello" #\z) #f)
(check index-with-a-predicate (string-index "ab1" char-numeric?) 2)
(check index-with-a-char-set
       (string-index "abc" (string->char-set "cb"))
       1)
(check index-right (string-index-right "hello" #\l) 3)
(check index-right-none (string-index-right "hello" #\z) #f)
(check skip (string-skip "  hi" char-whitespace?) 2)
(check skip-none (string-skip "   " char-whitespace?) #f)
(check skip-right (string-skip-right "hi  " char-whitespace?) 1)
(check count (string-count "banana" #\a) 3)
(check count-zero (string-count "xyz" #\a) 0)
(check count-with-a-predicate (string-count "a1b2" char-numeric?) 2)
(check index-in-a-range (string-index "hello" #\l 3) 3)
(check count-in-a-range (string-count "banana" #\a 2 4) 1)
;; string-contains answers the index in the first string.
(check contains (string-contains "banana" "nan") 2)
(check contains-at-zero (string-contains "banana" "ban") 0)
(check contains-absent (string-contains "banana" "xyz") #f)
(check contains-empty-pattern (string-contains "banana" "") 0)
(check contains-longer-than-text (string-contains "ab" "abc") #f)
(check contains-ci (string-contains-ci "BANANA" "nan") 2)
(check contains-in-a-range (string-contains "banana" "na" 3) 4)

;; ── case mapping ─────────────────────────────────────────────────────
(check upcase (string-upcase "hello") "HELLO")
(check downcase (string-downcase "HELLO") "hello")
(check upcase-in-a-range (string-upcase "hello" 0 2) "HE")
(check titlecase (string-titlecase "hello world") "Hello World")
(check titlecase-lowers-the-rest (string-titlecase "hELLO") "Hello")
(check titlecase-across-punctuation
       (string-titlecase "it's a-b")
       "It'S A-B")
(check titlecase-in-a-range (string-titlecase "hello world" 6 11) "World")
;; These three mutate, which is how the SRFI specifies them -- unlike the
;; ! procedures in (srfi 1) and (srfi 14), which are pure here.
(check upcase!-mutates
       (let ((s (string-copy "hello"))) (string-upcase! s) s)
       "HELLO")
(check downcase!-mutates
       (let ((s (string-copy "HELLO"))) (string-downcase! s) s)
       "hello")
(check titlecase!-mutates
       (let ((s (string-copy "hello world"))) (string-titlecase! s) s)
       "Hello World")
(check upcase!-in-a-range
       (let ((s (string-copy "hello"))) (string-upcase! s 0 2) s)
       "HEllo")

;; ── reverse and append ───────────────────────────────────────────────
(check reverse (string-reverse "abc") "cba")
(check reverse-empty (string-reverse "") "")
(check reverse-in-a-range (string-reverse "abcde" 1 4) "dcb")
(check reverse!-mutates
       (let ((s (string-copy "abc"))) (string-reverse! s) s)
       "cba")
(check reverse!-in-a-range
       (let ((s (string-copy "abcde"))) (string-reverse! s 1 4) s)
       "adcbe")
(check concatenate (string-concatenate '("a" "bc" "d")) "abcd")
(check concatenate-empty (string-concatenate '()) "")
;; The list is reversed, then the optional final string is appended.
(check concatenate-reverse (string-concatenate-reverse '("c" "b" "a")) "abc")
(check concatenate-reverse-with-final
       (string-concatenate-reverse '("b" "a") "cd")
       "abcd")
(check concatenate-reverse-with-end
       (string-concatenate-reverse '("b" "a") "cdef" 2)
       "abcd")
(check concatenate-reverse-empty (string-concatenate-reverse '()) "")
;; The /shared variants are permitted to share and here copy, so they are
;; equal to their plain counterparts.
(check append/shared (string-append/shared "a" "b") "ab")
(check concatenate/shared (string-concatenate/shared '("a" "b")) "ab")
(check concatenate-reverse/shared
       (string-concatenate-reverse/shared '("b" "a"))
       "ab")

;; ── fold and unfold ──────────────────────────────────────────────────
;;
;; (kons char state) -- the CHARACTER FIRST, which agrees with (srfi 1)'s
;; fold and (srfi 14)'s char-set-fold and disagrees with (srfi 133)'s
;; vector-fold, where the state comes first.
(check fold-char-first (string-fold cons '() "abc") '(#\c #\b #\a))
(check fold-right-char-first (string-fold-right cons '() "abc") '(#\a #\b #\c))
(check fold-empty (string-fold cons '() "") '())
(check fold-counts (string-fold (lambda (c n) (+ n 1)) 0 "abc") 3)
(check fold-in-a-range (string-fold cons '() "abcd" 1 3) '(#\c #\b))
(check fold-right-in-a-range
       (string-fold-right cons '() "abcd" 1 3)
       '(#\b #\c))
;; (string-unfold p f g seed [base make-final]): g makes the next seed, p
;; says when to STOP, f makes the character. Reading p as "keep going"
;; would give the empty string.
(check unfold
       (string-unfold (lambda (i) (= i 3))
                      (lambda (i) (integer->char (+ 97 i)))
                      (lambda (i) (+ i 1))
                      0)
       "abc")
(check unfold-stops-at-once
       (string-unfold (lambda (i) #t) (lambda (i) #\a) (lambda (i) i) 0)
       "")
;; base is the LEFTMOST part, make-final the rightmost.
(check unfold-base-is-on-the-left
       (string-unfold (lambda (i) (= i 2))
                      (lambda (i) (integer->char (+ 97 i)))
                      (lambda (i) (+ i 1))
                      0 "<")
       "<ab")
(check unfold-make-final-is-on-the-right
       (string-unfold (lambda (i) (= i 2))
                      (lambda (i) (integer->char (+ 97 i)))
                      (lambda (i) (+ i 1))
                      0 "<" (lambda (i) ">"))
       "<ab>")
;; The SRFI's own example: list->string is an unfold.
(check unfold-is-list->string
       (string-unfold null? car cdr '(#\a #\b #\c))
       "abc")
;; unfold-right assembles right to left, so base is the RIGHTMOST part
;; and make-final the leftmost.
(check unfold-right
       (string-unfold-right (lambda (i) (= i 3))
                            (lambda (i) (integer->char (+ 97 i)))
                            (lambda (i) (+ i 1))
                            0)
       "cba")
(check unfold-right-base-is-on-the-right
       (string-unfold-right (lambda (i) (= i 2))
                            (lambda (i) (integer->char (+ 97 i)))
                            (lambda (i) (+ i 1))
                            0 ">")
       "ba>")
(check unfold-right-make-final-is-on-the-left
       (string-unfold-right (lambda (i) (= i 2))
                            (lambda (i) (integer->char (+ 97 i)))
                            (lambda (i) (+ i 1))
                            0 ">" (lambda (i) "<"))
       "<ba>")
;; unfold-right reverses a string, which is the idiom it exists for.
(check unfold-right-reverses
       (let ((s "abc"))
         (string-unfold-right (lambda (i) (= i (string-length s)))
                              (lambda (i) (string-ref s i))
                              (lambda (i) (+ i 1))
                              0))
       "cba")

;; ── map and for-each: a superset of BOTH shapes ──────────────────────
;;
;; R7RS takes any number of strings; this SRFI takes one string and a
;; range. Narrowing to the SRFI's shape would break R7RS code that merely
;; imported this library, so both work and are told apart by type.
(check map-srfi-13-shape (string-map char-upcase "abc") "ABC")
(check map-srfi-13-range (string-map char-upcase "abcd" 1 3) "BC")
(check map-r7rs-shape
       (string-map (lambda (a b) (if (char<? a b) a b)) "adc" "bbd")
       "abc")
(check map-r7rs-stops-at-the-shortest
       (string-map (lambda (a b) a) "abcd" "xy")
       "ab")
(check for-each-srfi-13-shape
       (let ((acc '()))
         (string-for-each (lambda (c) (set! acc (cons c acc))) "abc")
         acc)
       '(#\c #\b #\a))
(check for-each-srfi-13-range
       (let ((acc '()))
         (string-for-each (lambda (c) (set! acc (cons c acc))) "abcd" 1 3)
         acc)
       '(#\c #\b))
(check for-each-r7rs-shape
       (let ((acc '()))
         (string-for-each (lambda (a b) (set! acc (cons (list a b) acc)))
                          "ab" "xy")
         acc)
       '((#\b #\y) (#\a #\x)))
;; string-map! mutates; string-for-each-index gets the index, not the
;; character.
(check map!-mutates
       (let ((s (string-copy "abc"))) (string-map! char-upcase s) s)
       "ABC")
(check map!-in-a-range
       (let ((s (string-copy "abcd"))) (string-map! char-upcase s 1 3) s)
       "aBCd")
(check for-each-index
       (let ((acc '()))
         (string-for-each-index (lambda (i) (set! acc (cons i acc))) "abc")
         acc)
       '(2 1 0))
(check for-each-index-in-a-range
       (let ((acc '()))
         (string-for-each-index (lambda (i) (set! acc (cons i acc))) "abcd" 1 3)
         acc)
       '(2 1))

;; ── replicate and rotate ─────────────────────────────────────────────
;;
;; The range is replicated in BOTH directions without end, so from and to
;; may be negative and the slice wraps.
(check xsubstring-repeats (xsubstring "abc" 0 7) "abcabca")
(check xsubstring-default-to-is-one-period (xsubstring "abc" 0) "abc")
(check xsubstring-rotates (xsubstring "abcdef" 2 8) "cdefab")
(check xsubstring-negative-from (xsubstring "abc" -2 2) "bcab")
(check xsubstring-wholly-negative (xsubstring "abc" -3 0) "abc")
(check xsubstring-empty-slice (xsubstring "abc" 2 2) "")
(check xsubstring-of-a-range (xsubstring "xabcx" 0 5 1 4) "abcab")
;; Replicating an empty range has no answer, so it raises rather than
;; looping or dividing by zero.
(check-raises xsubstring-of-an-empty-range (xsubstring "" 0 3))
(check string-xcopy!
       (let ((s (string-copy ".......")))
         (string-xcopy! s 0 "abc" 0 7)
         s)
       "abcabca")
(check string-xcopy!-at-an-offset
       (let ((s (string-copy ".......")))
         (string-xcopy! s 2 "ab" 0 3)
         s)
       "..aba..")

;; ── miscellaneous ────────────────────────────────────────────────────
(check replace (string-replace "hello" "XY" 1 3) "hXYlo")
(check replace-at-the-start (string-replace "hello" "X" 0 1) "Xello")
(check replace-inserting (string-replace "hello" "XY" 2 2) "heXYllo")
(check replace-deleting (string-replace "hello" "" 1 3) "hlo")
(check replace-with-a-range (string-replace "hello" "aXYb" 1 3 1 3) "hXYlo")
;; string-tokenize: STRING FIRST, and the token set defaults to
;; char-set:graphic -- so it splits on whitespace by default.
(check tokenize (string-tokenize "a bb  ccc") '("a" "bb" "ccc"))
(check tokenize-leading-and-trailing
       (string-tokenize "  a b  ")
       '("a" "b"))
(check tokenize-empty (string-tokenize "") '())
(check tokenize-all-whitespace (string-tokenize "   ") '())
(check tokenize-one-token (string-tokenize "abc") '("abc"))
(check tokenize-with-a-char-set
       (string-tokenize "a1b22c" char-numeric?)
       '("1" "22"))
(check tokenize-in-a-range (string-tokenize "x a b x" char-alphabetic? 1 6)
       '("a" "b"))

;; ── filtering and deleting: CRITERION FIRST ──────────────────────────
;;
;; (string-filter pred s) but (string-count s pred) -- the SRFI is
;; inconsistent with itself here and both orders are as specified.
(check filter (string-filter char-alphabetic? "a1b2") "ab")
(check filter-none (string-filter char-numeric? "abc") "")
(check filter-all (string-filter char-alphabetic? "abc") "abc")
(check filter-with-a-char (string-filter #\a "banana") "aaa")
(check filter-with-a-char-set
       (string-filter (string->char-set "ab") "xaybz")
       "ab")
(check delete (string-delete char-numeric? "a1b2") "ab")
(check delete-with-a-char (string-delete #\a "banana") "bnn")
(check filter-in-a-range (string-filter char-alphabetic? "a1b2" 2 4) "b")
(check delete-in-a-range (string-delete char-numeric? "a1b2" 0 2) "a")
;; The two orders side by side, which is the thing to get right.
(check criterion-first-and-string-first-both-work
       (list (string-filter char-numeric? "a1b2")
             (string-count "a1b2" char-numeric?))
       '("12" 2))

;; ── the (srfi 14) dependency, and the way round it ───────────────────
;;
;; A char-set criterion is the only reason this library imports (srfi 14),
;; and char-sets there are Latin-1 -- so a char-set cannot match λ while a
;; predicate can. This is the documented escape hatch.
(check a-char-set-criterion-misses-beyond-latin1
       (string-index "aλb" char-set:letter 1 2)
       #f)
(check a-predicate-criterion-finds-it
       (string-index "aλb" char-alphabetic? 1 2)
       1)
(check trim-with-a-char-set-is-latin1
       (string-trim-both "λaλ" char-set:letter)
       "λaλ")
(check trim-with-a-predicate-is-unicode-correct
       (string-trim-both "λaλ" char-alphabetic?)
       "")

;; ── the re-exported R7RS procedures ──────────────────────────────────
(check base-string? (list (string? "a") (string? #\a)) '(#t #f))
(check base-make-string (make-string 3 #\x) "xxx")
(check base-string (string #\a #\b) "ab")
(check base-string-length (string-length "abc") 3)
(check base-string-ref (string-ref "abc" 1) #\b)
(check base-string-set!
       (let ((s (string-copy "abc"))) (string-set! s 0 #\x) s)
       "xbc")
(check base-string-append (string-append "a" "bc") "abc")
(check base-string->list (string->list "abc") '(#\a #\b #\c))
(check base-string->list-range (string->list "abcd" 1 3) '(#\b #\c))
(check base-list->string (list->string '(#\a #\b)) "ab")
(check base-string-copy (string-copy "abc") "abc")
(check base-string-copy-range (string-copy "abcd" 1 3) "bc")
(check base-string-copy!
       (let ((s (string-copy "xxxx"))) (string-copy! s 1 "ab") s)
       "xabx")
(check base-string-fill!
       (let ((s (string-copy "abc"))) (string-fill! s #\x) s)
       "xxx")
(check base-string-fill!-range
       (let ((s (string-copy "abcd"))) (string-fill! s #\x 1 3) s)
       "axxd")

(test-failures)
