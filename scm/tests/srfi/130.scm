;;; (srfi 130) — the cursor-based string library.
;;;
;;; Cursors here are indices, so most of the cursor surface is trivially
;;; right and what is worth testing is elsewhere: the searches answer a
;;; **cursor rather than `#f`**, which changes every "not found" case;
;;; `string-index-right` answers the **successor** of what it found;
;;; `string-trim` defaults to `char-whitespace?` where `(srfi 13)`'s
;;; defaults to a Latin-1 char-set, so the two libraries really differ;
;;; and `string-split` is not `(srfi 13)`'s `string-tokenize`.
;;;
;;; Both libraries are imported, prefixed, so the differences can be
;;; asserted side by side rather than described.
(import (scm test)
        (srfi 130)
        (prefix (srfi 13) s13:))

;; ── cursors, which here are indices ──────────────────────────────────
(check cursor-start (string-cursor-start "abc") 0)
(check cursor-end (string-cursor-end "abc") 3)
(check cursor-end-of-empty (string-cursor-end "") 0)
(check cursor-next (string-cursor-next "abc" 0) 1)
(check cursor-prev (string-cursor-prev "abc" 2) 1)
(check cursor-forward (string-cursor-forward "abcde" 1 3) 4)
(check cursor-back (string-cursor-back "abcde" 4 3) 1)
(check cursor-diff (string-cursor-diff "abcde" 1 4) 3)
(check cursor-diff-of-nothing (string-cursor-diff "abc" 2 2) 0)
(check cursor=? (list (string-cursor=? 1 1) (string-cursor=? 1 2)) '(#t #f))
(check cursor<? (list (string-cursor<? 1 2) (string-cursor<? 2 1)) '(#t #f))
(check cursor>? (list (string-cursor>? 2 1) (string-cursor>? 1 2)) '(#t #f))
(check cursor<=? (list (string-cursor<=? 1 1) (string-cursor<=? 2 1)) '(#t #f))
(check cursor>=? (list (string-cursor>=? 1 1) (string-cursor>=? 1 2)) '(#t #f))
;; A string of length n has n+1 cursors: one per character and one past
;; the end.
(check there-are-n+1-cursors
       (let loop ((cur (string-cursor-start "abc")) (n 0))
         (if (string-cursor>=? cur (string-cursor-end "abc"))
             (+ n 1)
             (loop (string-cursor-next "abc" cur) (+ n 1))))
       4)
;; The conversions are the identity, because the two representations are
;; the same one.
(check cursor->index-is-the-identity (string-cursor->index "abc" 2) 2)
(check index->cursor-is-the-identity (string-index->cursor "abc" 2) 2)
(check the-conversions-round-trip
       (string-cursor->index "abc" (string-index->cursor "abc" 1))
       1)
;; The SRFI requires cursors to be disjoint from #f, which integers are.
(check a-cursor-is-not-false (string-cursor? #f) #f)
(check an-index-is-a-cursor (string-cursor? 0) #t)
(check a-negative-integer-is-not-a-cursor (string-cursor? -1) #f)
(check a-string-is-not-a-cursor (string-cursor? "a") #f)
;; Because cursors are indices, a procedure's start/end may be written
;; either way with the same result.
(check cursors-and-indices-are-interchangeable
       (list (substring/cursors "abcde" 1 3)
             (substring/cursors "abcde"
                                (string-index->cursor "abcde" 1)
                                (string-index->cursor "abcde" 3)))
       '("bc" "bc"))

;; ── the /cursor and /cursors spellings ───────────────────────────────
(check ref/cursor (string-ref/cursor "abc" 1) #\b)
(check substring/cursors (substring/cursors "abcde" 1 3) "bc")
(check string-copy/cursors (string-copy/cursors "abcde" 1 3) "bc")
(check string->list/cursors (string->list/cursors "abc") '(#\a #\b #\c))
(check string->list/cursors-range (string->list/cursors "abcd" 1 3) '(#\b #\c))
(check string->vector/cursors (string->vector/cursors "abc") '#(#\a #\b #\c))

;; ── searching: "not found" is a CURSOR, never #f ─────────────────────
;;
;; This is the change that touches every caller. (srfi 13) answers #f;
;; here a cursor must be disjoint from #f, so string-index answers the
;; END cursor and string-index-right the START cursor.
(check index-found (string-index "hello" (lambda (c) (char=? c #\l))) 2)
(check index-not-found-is-the-end-cursor
       (string-index "hello" (lambda (c) (char=? c #\z)))
       5)
(check srfi-13-answers-false-instead
       (s13:string-index "hello" (lambda (c) (char=? c #\z)))
       #f)
(check index-not-found-in-an-empty-string (string-index "" char-alphabetic?) 0)
;; string-index-right answers the SUCCESSOR of the character it found, so
;; that the result is a cursor a substring can be cut at. (srfi 13)
;; answers the character's own index, one less.
(check index-right-answers-the-successor
       (string-index-right "hello" (lambda (c) (char=? c #\l)))
       4)
(check srfi-13-answers-the-character-itself
       (s13:string-index-right "hello" (lambda (c) (char=? c #\l)))
       3)
(check index-right-not-found-is-the-start-cursor
       (string-index-right "hello" (lambda (c) (char=? c #\z)))
       0)
;; The successor convention is what makes this work out: cutting at the
;; returned cursor keeps the match.
(check the-successor-convention-is-what-you-cut-at
       (let ((s "hello world"))
         (substring/cursors s (string-cursor-start s)
                              (string-index-right s (lambda (c) (char=? c #\o)))))
       "hello wo")
(check skip (string-skip "  hi" char-whitespace?) 2)
(check skip-not-found-is-the-end-cursor
       (string-skip "   " char-whitespace?)
       3)
(check skip-right (string-skip-right "hi  " char-whitespace?) 2)
(check skip-right-not-found-is-the-start-cursor
       (string-skip-right "   " char-whitespace?)
       0)
(check search-in-a-range
       (string-index "hello" (lambda (c) (char=? c #\l)) 3)
       3)
(check search-in-a-range-not-found
       (string-index "hello" (lambda (c) (char=? c #\h)) 1 5)
       5)
;; The cost of the convention, worth knowing: a match at the very end and
;; no match at all give the SAME cursor, so a caller that must tell them
;; apart compares against the end first.
(check a-match-at-the-end-looks-like-no-match
       (list (string-index "ab" (lambda (c) (char=? c #\z)))
             (string-skip "ab" char-alphabetic?))
       '(2 2))
;; string-contains is the exception the spec allows: it may answer #f,
;; because "found at the end" and "not found" both need an answer.
(check contains (string-contains "banana" "nan") 2)
(check contains-answers-false (string-contains "banana" "zz") #f)
(check contains-right-answers-the-last-match
       (string-contains-right "banana" "na")
       4)
(check contains-right-answers-false (string-contains-right "banana" "zz") #f)
(check contains-right-single-match (string-contains-right "banana" "ban") 0)
;; An empty pattern is found leftmost by one and rightmost by the other,
;; which is consistent even though the pair looks odd.
(check contains-of-an-empty-pattern (string-contains "banana" "") 0)
(check contains-right-of-an-empty-pattern
       (string-contains-right "banana" "")
       6)
(check contains-right-in-a-range
       (string-contains-right "banana" "na" 0 4)
       2)

;; ── trimming defaults to char-whitespace?, not a char-set ────────────
(check trim (string-trim "  hi  ") "hi  ")
(check trim-right (string-trim-right "  hi  ") "  hi")
(check trim-both (string-trim-both "  hi  ") "hi")
(check trim-empty (string-trim-both "") "")
(check trim-all (string-trim-both "   ") "")
(check trim-with-an-explicit-predicate
       (string-trim-both "12hi34" char-numeric?)
       "hi")
(check trim-in-a-range (string-trim-both "  hi  " char-whitespace? 1 5) "hi")
;; The real difference from (srfi 13), and the reason this is not a
;; restatement: 13 defaults to (srfi 14)'s char-set:whitespace, which is
;; Latin-1 and does not contain U+0085 NEL. This defaults to
;; char-whitespace?, which is Unicode-correct. So the two libraries trim
;; differently, and the test says exactly how.
(check nel-is-trimmed-here
       (string-trim-both
         (string-append (string (integer->char #x85)) "a"
                        (string (integer->char #x85))))
       "a")
(check nel-is-not-trimmed-by-srfi-13
       (s13:string-trim-both
         (string-append (string (integer->char #x85)) "a"
                        (string (integer->char #x85))))
       (string-append (string (integer->char #x85)) "a"
                      (string (integer->char #x85))))
;; They agree on ordinary whitespace.
(check the-two-agree-on-a-space
       (equal? (string-trim-both " a ") (s13:string-trim-both " a "))
       #t)

;; ── the whole string ─────────────────────────────────────────────────
;;
;; string-for-each-cursor hands over the CURSOR, not the character --
;; which is what replaces (srfi 13)'s string-for-each here.
(check for-each-cursor-gets-the-cursor
       (let ((acc '()))
         (string-for-each-cursor (lambda (cur) (set! acc (cons cur acc))) "abc")
         acc)
       '(2 1 0))
(check for-each-cursor-in-a-range
       (let ((acc '()))
         (string-for-each-cursor (lambda (cur) (set! acc (cons cur acc)))
                                 "abcd" 1 3)
         acc)
       '(2 1))
(check for-each-cursor-can-read-through
       (let ((acc '()))
         (string-for-each-cursor
           (lambda (cur) (set! acc (cons (string-ref/cursor "abc" cur) acc)))
           "abc")
         (reverse acc))
       '(#\a #\b #\c))
;; string-replicate is xsubstring with `to` mandatory and from > to an
;; error.
(check replicate (string-replicate "abc" 0 7) "abcabca")
(check replicate-rotates (string-replicate "abcdef" 2 8) "cdefab")
(check replicate-negative-from (string-replicate "abc" -2 2) "bcab")
(check replicate-empty-slice (string-replicate "abc" 2 2) "")
(check replicate-in-a-range (string-replicate "xabcx" 0 5 1 4) "abcab")
(check-raises replicate-from-greater-than-to (string-replicate "abc" 3 1))
(check-raises replicate-of-an-empty-range (string-replicate "" 0 3))

;; ── string-split, which is NOT string-tokenize ───────────────────────
;;
;; The delimiter is a string and the pieces are what lies between, so
;; adjacent delimiters give empty pieces -- where (srfi 13)'s
;; string-tokenize keeps the runs that match a criterion and never
;; returns an empty one.
(check split (string-split "a,b,c" ",") '("a" "b" "c"))
(check split-keeps-empty-pieces (string-split "a,,b" ",") '("a" "" "b"))
(check tokenize-does-not
       (s13:string-tokenize "a  b")
       '("a" "b"))
(check split-no-delimiter-present (string-split "abc" ",") '("abc"))
(check split-of-the-empty-string (string-split "" ",") '())
(check split-leading-delimiter (string-split ",a" ",") '("" "a"))
(check split-trailing-delimiter (string-split "a," ",") '("a" ""))
(check split-multi-character-delimiter
       (string-split "a::b::c" "::")
       '("a" "b" "c"))
;; An empty delimiter splits into characters.
(check split-on-an-empty-delimiter (string-split "abc" "") '("a" "b" "c"))
;; limit is the maximum number of SPLITS, so at most limit+1 pieces come
;; back and the last keeps whatever is left, delimiters included.
(check split-limit (string-split "a,b,c,d" "," 'infix 2) '("a" "b" "c,d"))
(check split-limit-zero (string-split "a,b" "," 'infix 0) '("a,b"))
(check split-limit-beyond-the-splits
       (string-split "a,b" "," 'infix 99)
       '("a" "b"))
(check split-limit-on-characters
       (string-split "abcd" "" 'infix 2)
       '("a" "b" "cd"))
;; The grammars, which drop the empty piece a leading or trailing
;; delimiter would otherwise leave.
(check split-prefix (string-split ",a,b" "," 'prefix) '("a" "b"))
(check split-prefix-without-one (string-split "a,b" "," 'prefix) '("a" "b"))
(check split-suffix (string-split "a,b," "," 'suffix) '("a" "b"))
(check split-suffix-without-one (string-split "a,b" "," 'suffix) '("a" "b"))
(check split-empty-infix (string-split "" "," 'infix) '())
(check-raises split-empty-strict-infix (string-split "" "," 'strict-infix))
(check split-non-empty-strict-infix
       (string-split "a,b" "," 'strict-infix)
       '("a" "b"))
(check-raises split-unknown-grammar (string-split "a" "," 'around))
(check split-in-a-range (string-split "xa,bx" "," 'infix #f 1 4) '("a" "b"))

;; ── string-remove, which is (srfi 13)'s string-delete renamed ────────
(check remove (string-remove char-numeric? "a1b2") "ab")
(check remove-nothing (string-remove char-numeric? "abc") "abc")
(check remove-everything (string-remove char-alphabetic? "abc") "")
(check remove-in-a-range (string-remove char-numeric? "a1b2" 0 2) "a")
(check remove-is-string-delete-renamed
       (equal? (string-remove char-numeric? "a1b2")
               (s13:string-delete char-numeric? "a1b2"))
       #t)
;; filter is its complement and is inherited unchanged.
(check filter (string-filter char-numeric? "a1b2") "12")

;; ── the twenty-eight inherited from (srfi 13) ────────────────────────
;;
;; Re-exported rather than rewritten, so what is checked here is that
;; importing (srfi 130) really supplies them.
(check null? (list (string-null? "") (string-null? "a")) '(#t #f))
(check every (string-every char-alphabetic? "abc") #t)
(check any (string-any char-numeric? "ab1") #t)
(check tabulate
       (string-tabulate (lambda (i) (integer->char (+ 97 i))) 3)
       "abc")
(check unfold
       (string-unfold (lambda (i) (= i 3))
                      (lambda (i) (integer->char (+ 97 i)))
                      (lambda (i) (+ i 1))
                      0)
       "abc")
(check unfold-right
       (string-unfold-right (lambda (i) (= i 3))
                            (lambda (i) (integer->char (+ 97 i)))
                            (lambda (i) (+ i 1))
                            0)
       "cba")
(check reverse-list->string (reverse-list->string '(#\a #\b)) "ba")
(check join (string-join '("a" "b") "-") "a-b")
(check take (string-take "hello" 2) "he")
(check take-right (string-take-right "hello" 2) "lo")
(check drop (string-drop "hello" 2) "llo")
(check drop-right (string-drop-right "hello" 2) "hel")
(check pad (string-pad "42" 5) "   42")
(check pad-right (string-pad-right "42" 5) "42   ")
(check prefix-length (string-prefix-length "about" "abort") 3)
(check suffix-length (string-suffix-length "cutting" "setting") 5)
(check prefix? (string-prefix? "ab" "abc") #t)
(check suffix? (string-suffix? "bc" "abc") #t)
(check reverse (string-reverse "abc") "cba")
(check concatenate (string-concatenate '("a" "b")) "ab")
(check concatenate-reverse (string-concatenate-reverse '("b" "a")) "ab")
;; (kons char state) -- the character first, as in (srfi 13) and
;; (srfi 14), and unlike (srfi 133)'s vector-fold.
(check fold-char-first (string-fold cons '() "abc") '(#\c #\b #\a))
(check fold-right (string-fold-right cons '() "abc") '(#\a #\b #\c))
(check count (string-count "banana" #\a) 3)
(check replace (string-replace "hello" "XY" 1 3) "hXYlo")

;; ── and what it does NOT export ──────────────────────────────────────
;;
;; (srfi 130) drops the case, comparison and mutation operations, and with
;; them string-map and string-for-each -- so unlike (srfi 13) it collides
;; with R7RS nowhere. This file imports (srfi 130) unprefixed, and
;; string-map here is still R7RS's n-ary one.
(check string-map-is-still-r7rs
       (string-map (lambda (a b) (if (char<? a b) a b)) "adc" "bbd")
       "abc")
(check string-for-each-is-still-r7rs
       (let ((acc '()))
         (string-for-each (lambda (a b) (set! acc (cons (list a b) acc)))
                          "ab" "xy")
         acc)
       '((#\b #\y) (#\a #\x)))

(test-failures)
