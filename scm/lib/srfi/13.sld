;;; (srfi 13) — String Libraries.
;;;
;;; Written against the SRFI 13 specification rather than copied from its
;;; reference implementation. The reference was read on the points the
;;; prose compresses: `string-unfold`'s six arguments, `string-join`'s
;;; grammars, the direction `string-pad` truncates, and the exact default
;;; bound of `string-hash`.
;;;
;;; **This library predates R7RS**, and that shows up in three ways worth
;;; knowing before importing it.
;;;
;;; *One.* It needs `(srfi 14)` char-sets: twelve procedures take a
;;; "char/char-set/pred" criterion, and `string-trim` *defaults* to
;;; `char-set:whitespace`, so it reaches for SRFI 14 even when the caller
;;; passes nothing. Char-sets here are Latin-1 ([srfi.md](../../docs/srfi.md)),
;;; so **for Unicode-correct matching pass a predicate** -- every one of
;;; those twelve accepts `char-alphabetic?` or any other procedure, and
;;; scm's character predicates are Unicode-correct over the whole range.
;;;
;;; *Two.* `string-map` and `string-for-each` collide with R7RS, which
;;; gives them a different shape: R7RS takes any number of strings, this
;;; SRFI takes one string and an optional range. Narrowing them would
;;; break ordinary R7RS code the moment it imported this library, so both
;;; are defined here as **supersets of both shapes**, told apart by
;;; whether the argument after the string is a string or an integer.
;;; `(srfi 1)` does the same for `map` and `for-each`.
;;;
;;; *Three.* SRFI 13 is inconsistent with *itself* about where the
;;; criterion goes, and this is the thing most likely to bite:
;;;
;;;   criterion first:  string-every  string-any  string-filter  string-delete
;;;   string first:     string-index  string-skip  string-count
;;;                     string-trim*  string-tokenize
;;;
;;; So it is `(string-filter pred s)` but `(string-count s pred)`. Both
;;; orders are as specified, the later string SRFIs regularised it, and
;;; the tests here pin every one of them.
;;;
;;; **The `/shared` procedures copy.** `substring/shared`,
;;; `string-append/shared`, `string-concatenate/shared` and
;;; `string-concatenate-reverse/shared` are permitted to share storage
;;; with their arguments, not required to; scm's strings are `Vec<char>`
;;; with no shared-slice representation (plan §4.3), so each returns a
;;; fresh string. Sharing is an optimisation the specification allows, and
;;; a caller that relied on `eq?`-ness would be relying on something the
;;; SRFI does not promise.
;;;
;;; The genuinely destructive procedures here -- `string-upcase!`,
;;; `string-reverse!`, `string-map!`, `string-copy!`, `string-fill!`,
;;; `string-xcopy!` and the rest -- **do mutate**, unlike the `!`
;;; procedures in `(srfi 1)` and `(srfi 14)`. Those were *linear-update*
;;; variants, which the SRFI lets an implementation make pure; these are
;;; specified as mutators returning an unspecified value, and a caller
;;; cannot get the result any other way.
;;;
;;; `string-lib-internals` -- `string-parse-start+end`,
;;; `let-string-start+end`, `make-kmp-restart-vector`, `kmp-step` and
;;; `string-kmp-partial-search` -- is a **separate library** in the SRFI,
;;; not part of this one, and is not provided. Its purpose is to let
;;; *other* libraries parse SRFI 13-style argument lists; nothing here
;;; needs to export that.
(define-library (srfi 13)
  (export
    ;; predicates
    string? string-null? string-every string-any
    ;; constructors
    make-string string string-tabulate
    ;; list and string conversion
    string->list list->string reverse-list->string string-join
    ;; selection
    string-length string-ref string-copy substring/shared string-copy!
    string-take string-take-right string-drop string-drop-right
    string-pad string-pad-right
    string-trim string-trim-right string-trim-both
    ;; modification
    string-set! string-fill!
    ;; comparison
    string-compare string-compare-ci
    string= string<> string< string> string<= string>=
    string-ci= string-ci<> string-ci< string-ci> string-ci<= string-ci>=
    string-hash string-hash-ci
    ;; prefixes and suffixes
    string-prefix-length string-suffix-length
    string-prefix-length-ci string-suffix-length-ci
    string-prefix? string-suffix? string-prefix-ci? string-suffix-ci?
    ;; searching
    string-index string-index-right string-skip string-skip-right
    string-count string-contains string-contains-ci
    ;; case mapping
    string-titlecase string-titlecase!
    string-upcase string-upcase! string-downcase string-downcase!
    ;; reverse and append
    string-reverse string-reverse! string-append
    string-concatenate string-concatenate/shared string-append/shared
    string-concatenate-reverse string-concatenate-reverse/shared
    ;; fold, unfold and map
    string-map string-map! string-fold string-fold-right
    string-unfold string-unfold-right
    string-for-each string-for-each-index
    ;; replicate and rotate
    xsubstring string-xcopy!
    ;; miscellaneous
    string-replace string-tokenize
    ;; filtering and deleting
    string-filter string-delete)
  (import (scheme base) (srfi 14))
  (begin

    ;; ── parsing the optional ranges ───────────────────────────────────
    ;;
    ;; Almost every procedure here ends in an optional `[start end]`, and
    ;; the comparisons end in `[start1 end1 start2 end2]`. These two
    ;; helpers are what the reference implementation needs a macro for.
    (define (%range s opt)
      (values (if (pair? opt) (car opt) 0)
              (if (and (pair? opt) (pair? (cdr opt)))
                  (cadr opt)
                  (string-length s))))

    (define (%range2 s1 s2 opt)
      (let* ((start1 (if (pair? opt) (car opt) 0))
             (r1 (if (pair? opt) (cdr opt) '()))
             (end1 (if (pair? r1) (car r1) (string-length s1)))
             (r2 (if (pair? r1) (cdr r1) '()))
             (start2 (if (pair? r2) (car r2) 0))
             (r3 (if (pair? r2) (cdr r2) '()))
             (end2 (if (pair? r3) (car r3) (string-length s2))))
        (values start1 end1 start2 end2)))

    ;; A criterion as a predicate. **This is the whole of the `(srfi 14)`
    ;; dependency**: a char-set criterion is the only reason this library
    ;; imports it, and a predicate criterion bypasses it entirely, which
    ;; is the Unicode-correct way in.
    (define (%pred criterion who)
      (cond ((char? criterion) (lambda (c) (char=? c criterion)))
            ((char-set? criterion)
             (lambda (c) (char-set-contains? criterion c)))
            ((procedure? criterion) criterion)
            (else (error (string-append
                           who ": expects a character, a char-set or a"
                               " predicate")
                         criterion))))

    (define (%substring s start end)
      (let* ((n (- end start)) (out (make-string n)))
        (let loop ((i 0))
          (if (= i n)
              out
              (begin (string-set! out i (string-ref s (+ start i)))
                     (loop (+ i 1)))))))

    ;; ── predicates ────────────────────────────────────────────────────
    (define (string-null? s) (= 0 (string-length s)))

    ;; **Criterion first.** Answers the value the predicate gave for the
    ;; last character, and `#t` for an empty range.
    (define (string-every criterion s . opt)
      (let ((p (%pred criterion "string-every")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i start))
            (cond ((>= i end) #t)
                  ((= (+ i 1) end) (p (string-ref s i)))
                  ((p (string-ref s i)) (loop (+ i 1)))
                  (else #f))))))

    ;; **Criterion first.** Answers the predicate's value for the
    ;; character that matched, and `#f` for an empty range.
    (define (string-any criterion s . opt)
      (let ((p (%pred criterion "string-any")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i start))
            (and (< i end)
                 (if (= (+ i 1) end)
                     (p (string-ref s i))
                     (or (p (string-ref s i)) (loop (+ i 1)))))))))

    ;; ── constructors ──────────────────────────────────────────────────
    (define (string-tabulate proc len)
      (let ((out (make-string len)))
        (let loop ((i 0))
          (if (= i len)
              out
              (begin (string-set! out i (proc i)) (loop (+ i 1)))))))

    ;; ── list and string conversion ────────────────────────────────────
    (define (reverse-list->string chars)
      (let* ((n (length chars)) (out (make-string n)))
        (let loop ((i (- n 1)) (rest chars))
          (if (null? rest)
              out
              (begin (string-set! out i (car rest))
                     (loop (- i 1) (cdr rest)))))))

    ;; The delimiter defaults to one space and the grammar to `infix`.
    ;; `strict-infix` differs from `infix` **only** on the empty list,
    ;; where it raises rather than answering "" -- the distinction exists
    ;; because `infix` is not invertible there.
    (define (string-join strings . opt)
      (let ((delim (if (pair? opt) (car opt) " "))
            (grammar (if (and (pair? opt) (pair? (cdr opt)))
                         (cadr opt)
                         'infix)))
        (cond
         ((null? strings)
          (if (eq? grammar 'strict-infix)
              (error "string-join: an empty list cannot be joined with the strict-infix grammar")
              ""))
         ((memq grammar '(infix strict-infix))
          (let loop ((rest (cdr strings)) (acc (car strings)))
            (if (null? rest)
                acc
                (loop (cdr rest) (string-append acc delim (car rest))))))
         ((eq? grammar 'prefix)
          (let loop ((rest strings) (acc ""))
            (if (null? rest)
                acc
                (loop (cdr rest) (string-append acc delim (car rest))))))
         ((eq? grammar 'suffix)
          (let loop ((rest strings) (acc ""))
            (if (null? rest)
                acc
                (loop (cdr rest) (string-append acc (car rest) delim)))))
         (else (error "string-join: unknown grammar" grammar)))))

    ;; ── selection ─────────────────────────────────────────────────────
    ;;
    ;; Permitted to share, not required to; scm has no shared-slice
    ;; string, so this copies.
    (define (substring/shared s start . opt)
      (%substring s start (if (pair? opt) (car opt) (string-length s))))

    (define (string-take s n) (%substring s 0 n))
    (define (string-drop s n) (%substring s n (string-length s)))
    (define (string-take-right s n)
      (let ((len (string-length s))) (%substring s (- len n) len)))
    (define (string-drop-right s n)
      (%substring s 0 (- (string-length s) n)))

    ;; Pads on the **left** to reach `n`, and when the string is already
    ;; longer than `n` keeps the **rightmost** `n` characters -- it
    ;; truncates on the left, the side it pads. `string-pad-right` is the
    ;; mirror image. Getting this backwards silently keeps the wrong half.
    (define (string-pad s n . opt)
      (let ((char (if (and (pair? opt) (char? (car opt))) (car opt) #\space))
            (rest (if (and (pair? opt) (char? (car opt))) (cdr opt) opt)))
        (let-values (((start end) (%range s rest)))
          (let ((len (- end start)))
            (if (<= n len)
                (%substring s (- end n) end)
                (let ((out (make-string n char)))
                  (let loop ((i 0))
                    (if (= i len)
                        out
                        (begin (string-set! out (+ (- n len) i)
                                            (string-ref s (+ start i)))
                               (loop (+ i 1)))))))))))

    (define (string-pad-right s n . opt)
      (let ((char (if (and (pair? opt) (char? (car opt))) (car opt) #\space))
            (rest (if (and (pair? opt) (char? (car opt))) (cdr opt) opt)))
        (let-values (((start end) (%range s rest)))
          (let ((len (- end start)))
            (if (<= n len)
                (%substring s start (+ start n))
                (let ((out (make-string n char)))
                  (let loop ((i 0))
                    (if (= i len)
                        out
                        (begin (string-set! out i (string-ref s (+ start i)))
                               (loop (+ i 1)))))))))))

    ;; **String first**, then the criterion, which defaults to
    ;; `char-set:whitespace`.
    (define (%trim-args s opt)
      (if (and (pair? opt)
               (or (char? (car opt)) (char-set? (car opt))
                   (procedure? (car opt))))
          (values (car opt) (cdr opt))
          (values char-set:whitespace opt)))

    (define (string-trim s . opt)
      (let-values (((criterion rest) (%trim-args s opt)))
        (let ((p (%pred criterion "string-trim")))
          (let-values (((start end) (%range s rest)))
            (let loop ((i start))
              (cond ((>= i end) "")
                    ((p (string-ref s i)) (loop (+ i 1)))
                    (else (%substring s i end))))))))

    (define (string-trim-right s . opt)
      (let-values (((criterion rest) (%trim-args s opt)))
        (let ((p (%pred criterion "string-trim-right")))
          (let-values (((start end) (%range s rest)))
            (let loop ((i (- end 1)))
              (cond ((< i start) "")
                    ((p (string-ref s i)) (loop (- i 1)))
                    (else (%substring s start (+ i 1)))))))))

    (define (string-trim-both s . opt)
      (let-values (((criterion rest) (%trim-args s opt)))
        (let ((p (%pred criterion "string-trim-both")))
          (let-values (((start end) (%range s rest)))
            (let loop ((i start))
              (cond ((>= i end) "")
                    ((p (string-ref s i)) (loop (+ i 1)))
                    (else
                     (let loop2 ((j (- end 1)))
                       (if (p (string-ref s j))
                           (loop2 (- j 1))
                           (%substring s i (+ j 1)))))))))))

    ;; ── comparison ────────────────────────────────────────────────────
    ;;
    ;; The length of the common prefix of the two ranges, which is what
    ;; every comparison and every prefix procedure here is built on.
    (define (%common-prefix s1 start1 end1 s2 start2 end2 fold?)
      (let ((n (min (- end1 start1) (- end2 start2))))
        (let loop ((i 0))
          (if (or (= i n)
                  (not (char=? (%fold (string-ref s1 (+ start1 i)) fold?)
                               (%fold (string-ref s2 (+ start2 i)) fold?))))
              i
              (loop (+ i 1))))))

    (define (%common-suffix s1 start1 end1 s2 start2 end2 fold?)
      (let ((n (min (- end1 start1) (- end2 start2))))
        (let loop ((i 0))
          (if (or (= i n)
                  (not (char=? (%fold (string-ref s1 (- end1 i 1)) fold?)
                               (%fold (string-ref s2 (- end2 i 1)) fold?))))
              i
              (loop (+ i 1))))))

    (define (%fold c fold?) (if fold? (char-downcase c) c))

    ;; `proc<`, `proc=` and `proc>` are all applied to the **same** index,
    ;; `start1 + match` -- one past the common prefix. The specification
    ;; says the index is `end1` "in the case of equality", which is not a
    ;; separate rule but the same one: when the ranges are equal the
    ;; common prefix is the whole of range 1, so `start1 + match` *is*
    ;; `end1`. Reading it as a separate rule gets the case where range 2
    ;; is a proper prefix of range 1 wrong, reporting `end1` where the
    ;; strings actually diverge earlier.
    (define (%compare s1 s2 proc< proc= proc> opt fold? who)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (let* ((size1 (- end1 start1))
               (size2 (- end2 start2))
               (match (%common-prefix s1 start1 end1 s2 start2 end2 fold?))
               (i (+ start1 match)))
          (cond ((= match size1)
                 ;; Range 1 ran out: equal if range 2 did too, else less.
                 (if (= match size2) (proc= i) (proc< i)))
                ;; Range 2 ran out first, so range 1 is the greater.
                ((= match size2) (proc> i))
                (else
                 (if (char<? (%fold (string-ref s1 i) fold?)
                             (%fold (string-ref s2 (+ start2 match)) fold?))
                     (proc< i)
                     (proc> i)))))))

    (define (string-compare s1 s2 proc< proc= proc> . opt)
      (%compare s1 s2 proc< proc= proc> opt #f "string-compare"))

    (define (string-compare-ci s1 s2 proc< proc= proc> . opt)
      (%compare s1 s2 proc< proc= proc> opt #t "string-compare-ci"))

    ;; Each comparison answers the mismatch index, which is a true value,
    ;; rather than `#t` -- so they compose with `and` and still tell you
    ;; where. `string=` has nothing useful to point at and answers `#t`.
    (define (%cmp s1 s2 opt fold? lt eq gt who)
      (%compare s1 s2
                (lambda (i) (and lt (if (eq? lt 'index) i #t)))
                (lambda (i) (and eq (if (eq? eq 'index) i #t)))
                (lambda (i) (and gt (if (eq? gt 'index) i #t)))
                opt fold? who))

    (define (string= s1 s2 . opt)
      (%cmp s1 s2 opt #f #f #t #f "string="))
    (define (string<> s1 s2 . opt)
      (%cmp s1 s2 opt #f 'index #f 'index "string<>"))
    (define (string< s1 s2 . opt)
      (%cmp s1 s2 opt #f 'index #f #f "string<"))
    (define (string> s1 s2 . opt)
      (%cmp s1 s2 opt #f #f #f 'index "string>"))
    (define (string<= s1 s2 . opt)
      (%cmp s1 s2 opt #f 'index 'index #f "string<="))
    (define (string>= s1 s2 . opt)
      (%cmp s1 s2 opt #f #f 'index 'index "string>="))

    (define (string-ci= s1 s2 . opt)
      (%cmp s1 s2 opt #t #f #t #f "string-ci="))
    (define (string-ci<> s1 s2 . opt)
      (%cmp s1 s2 opt #t 'index #f 'index "string-ci<>"))
    (define (string-ci< s1 s2 . opt)
      (%cmp s1 s2 opt #t 'index #f #f "string-ci<"))
    (define (string-ci> s1 s2 . opt)
      (%cmp s1 s2 opt #t #f #f 'index "string-ci>"))
    (define (string-ci<= s1 s2 . opt)
      (%cmp s1 s2 opt #t 'index 'index #f "string-ci<="))
    (define (string-ci>= s1 s2 . opt)
      (%cmp s1 s2 opt #t #f 'index 'index "string-ci>="))

    ;; `bound` defaults to 2^22 and 0 asks for that default. The
    ;; accumulator is reduced at every step, because `(* 37 h)` over a
    ;; long string overflows without a numeric tower -- the same trap
    ;; `(srfi 128)`'s combiner and `(srfi 14)`'s hash hit (plan §2.3).
    ;; This is therefore **not** the same function as the native
    ;; `string-hash`, which the SRFI permits: a hash value is only ever
    ;; promised to agree with itself.
    (define (%string-hash s opt fold? who)
      (let ((bound (if (and (pair? opt) (not (eqv? 0 (car opt))))
                       (car opt)
                       4194304))
            (rest (if (pair? opt) (cdr opt) '())))
        (let-values (((start end) (%range s rest)))
          (let loop ((i start) (h 0))
            (if (= i end)
                (modulo h bound)
                (loop (+ i 1)
                      (modulo (+ (* 37 h)
                                 (char->integer
                                   (%fold (string-ref s i) fold?)))
                              4294967296)))))))

    (define (string-hash s . opt) (%string-hash s opt #f "string-hash"))
    (define (string-hash-ci s . opt) (%string-hash s opt #t "string-hash-ci"))

    ;; ── prefixes and suffixes ─────────────────────────────────────────
    (define (%prefix-length s1 s2 opt fold?)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (%common-prefix s1 start1 end1 s2 start2 end2 fold?)))

    (define (%suffix-length s1 s2 opt fold?)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (%common-suffix s1 start1 end1 s2 start2 end2 fold?)))

    (define (string-prefix-length s1 s2 . opt) (%prefix-length s1 s2 opt #f))
    (define (string-suffix-length s1 s2 . opt) (%suffix-length s1 s2 opt #f))
    (define (string-prefix-length-ci s1 s2 . opt) (%prefix-length s1 s2 opt #t))
    (define (string-suffix-length-ci s1 s2 . opt) (%suffix-length s1 s2 opt #t))

    ;; Is `s1` a prefix of `s2`? -- the first argument is the one being
    ;; looked for.
    (define (%prefix? s1 s2 opt fold?)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (let ((len1 (- end1 start1)))
          (and (<= len1 (- end2 start2))
               (= len1 (%prefix-length s1 s2 opt fold?))))))

    (define (%suffix? s1 s2 opt fold?)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (let ((len1 (- end1 start1)))
          (and (<= len1 (- end2 start2))
               (= len1 (%suffix-length s1 s2 opt fold?))))))

    (define (string-prefix? s1 s2 . opt) (%prefix? s1 s2 opt #f))
    (define (string-suffix? s1 s2 . opt) (%suffix? s1 s2 opt #f))
    (define (string-prefix-ci? s1 s2 . opt) (%prefix? s1 s2 opt #t))
    (define (string-suffix-ci? s1 s2 . opt) (%suffix? s1 s2 opt #t))

    ;; ── searching ─────────────────────────────────────────────────────
    ;;
    ;; **String first** for all five of these, unlike `string-every` and
    ;; `string-filter`.
    (define (string-index s criterion . opt)
      (let ((p (%pred criterion "string-index")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i start))
            (cond ((>= i end) #f)
                  ((p (string-ref s i)) i)
                  (else (loop (+ i 1))))))))

    (define (string-index-right s criterion . opt)
      (let ((p (%pred criterion "string-index-right")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i (- end 1)))
            (cond ((< i start) #f)
                  ((p (string-ref s i)) i)
                  (else (loop (- i 1))))))))

    (define (string-skip s criterion . opt)
      (let ((p (%pred criterion "string-skip")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i start))
            (cond ((>= i end) #f)
                  ((p (string-ref s i)) (loop (+ i 1)))
                  (else i))))))

    (define (string-skip-right s criterion . opt)
      (let ((p (%pred criterion "string-skip-right")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i (- end 1)))
            (cond ((< i start) #f)
                  ((p (string-ref s i)) (loop (- i 1)))
                  (else i))))))

    (define (string-count s criterion . opt)
      (let ((p (%pred criterion "string-count")))
        (let-values (((start end) (%range s opt)))
          (let loop ((i start) (n 0))
            (cond ((>= i end) n)
                  ((p (string-ref s i)) (loop (+ i 1) (+ n 1)))
                  (else (loop (+ i 1) n)))))))

    ;; The index in `text` where `pattern` occurs, or `#f`. A plain scan:
    ;; the reference reaches for Knuth-Morris-Pratt, which is what
    ;; `string-lib-internals` exports it for, and is worth having only
    ;; when the pattern is long and highly self-similar.
    (define (%contains text pattern opt fold? who)
      (let-values (((tstart tend pstart pend) (%range2 text pattern opt)))
        (let ((plen (- pend pstart)))
          (let loop ((i tstart))
            (cond ((> (+ i plen) tend) #f)
                  ((let match ((j 0))
                     (or (= j plen)
                         (and (char=? (%fold (string-ref text (+ i j)) fold?)
                                      (%fold (string-ref pattern (+ pstart j))
                                             fold?))
                              (match (+ j 1)))))
                   i)
                  (else (loop (+ i 1))))))))

    (define (string-contains text pattern . opt)
      (%contains text pattern opt #f "string-contains"))
    (define (string-contains-ci text pattern . opt)
      (%contains text pattern opt #t "string-contains-ci"))

    ;; ── case mapping ──────────────────────────────────────────────────
    (define (%case-map s opt proc who)
      (let-values (((start end) (%range s opt)))
        (let ((out (%substring s start end)))
          (let loop ((i 0))
            (if (= i (string-length out))
                out
                (begin (string-set! out i (proc (string-ref out i)))
                       (loop (+ i 1))))))))

    (define (string-upcase s . opt)
      (%case-map s opt char-upcase "string-upcase"))
    (define (string-downcase s . opt)
      (%case-map s opt char-downcase "string-downcase"))

    ;; A character starts a word when it is a letter and what precedes it
    ;; is not -- so the first letter of each run is upper-cased and the
    ;; rest of the run lower-cased.
    (define (string-titlecase s . opt)
      (let-values (((start end) (%range s opt)))
        (let ((out (%substring s start end)))
          (let loop ((i 0) (in-word? #f))
            (if (= i (string-length out))
                out
                (let* ((c (string-ref out i))
                       (letter? (char-alphabetic? c)))
                  (string-set! out i
                               (if letter?
                                   (if in-word? (char-downcase c) (char-upcase c))
                                   c))
                  (loop (+ i 1) letter?)))))))

    ;; These three **mutate** `s` in place and answer an unspecified
    ;; value, which is what the SRFI specifies for them.
    (define (%case-map! s opt proc)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start))
          (if (< i end)
              (begin (string-set! s i (proc (string-ref s i)))
                     (loop (+ i 1)))))))

    (define (string-upcase! s . opt) (%case-map! s opt char-upcase))
    (define (string-downcase! s . opt) (%case-map! s opt char-downcase))

    (define (string-titlecase! s . opt)
      (let-values (((start end) (%range s opt)))
        (let ((titled (string-titlecase s start end)))
          (let loop ((i 0))
            (if (< i (string-length titled))
                (begin (string-set! s (+ start i) (string-ref titled i))
                       (loop (+ i 1))))))))

    ;; ── reverse and append ────────────────────────────────────────────
    (define (string-reverse s . opt)
      (let-values (((start end) (%range s opt)))
        (let* ((n (- end start)) (out (make-string n)))
          (let loop ((i 0))
            (if (= i n)
                out
                (begin (string-set! out i (string-ref s (- end i 1)))
                       (loop (+ i 1))))))))

    (define (string-reverse! s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start) (j (- end 1)))
          (if (< i j)
              (let ((c (string-ref s i)))
                (string-set! s i (string-ref s j))
                (string-set! s j c)
                (loop (+ i 1) (- j 1)))))))

    (define (string-concatenate strings)
      (let loop ((rest strings) (acc ""))
        (if (null? rest)
            acc
            (loop (cdr rest) (string-append acc (car rest))))))

    ;; `final` is appended after the reversed list, and `end` bounds it.
    (define (string-concatenate-reverse strings . opt)
      (let* ((final (if (pair? opt) (car opt) ""))
             (end (if (and (pair? opt) (pair? (cdr opt)))
                      (cadr opt)
                      (string-length final))))
        (let loop ((rest strings) (acc (%substring final 0 end)))
          (if (null? rest)
              acc
              (loop (cdr rest) (string-append (car rest) acc))))))

    ;; Permitted to share; see the header.
    (define (string-append/shared . strings) (string-concatenate strings))
    (define (string-concatenate/shared strings) (string-concatenate strings))
    (define (string-concatenate-reverse/shared strings . opt)
      (apply string-concatenate-reverse strings opt))

    ;; ── fold, unfold and map ──────────────────────────────────────────
    ;;
    ;; `(kons char state)` -- the **character first**, which agrees with
    ;; `(srfi 1)`'s `fold` and `(srfi 14)`'s `char-set-fold` and disagrees
    ;; with `(srfi 133)`'s `vector-fold`, where the state comes first.
    (define (string-fold kons knil s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start) (acc knil))
          (if (>= i end)
              acc
              (loop (+ i 1) (kons (string-ref s i) acc))))))

    (define (string-fold-right kons knil s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i (- end 1)) (acc knil))
          (if (< i start)
              acc
              (loop (- i 1) (kons (string-ref s i) acc))))))

    ;; `(string-unfold p f g seed [base make-final])`: `g` makes the next
    ;; seed, `p` **says when to stop**, `f` makes the character, `base` is
    ;; the leftmost part of the result and `make-final` is applied to the
    ;; terminal seed to make the rightmost part. The specification gives
    ;; the identity this is built to satisfy:
    ;;
    ;;   (string-append base
    ;;                  (let recur ((seed seed))
    ;;                    (if (p seed) (make-final seed)
    ;;                        (string-append (string (f seed))
    ;;                                       (recur (g seed))))))
    (define (string-unfold p f g seed . opt)
      (let ((base (if (pair? opt) (car opt) ""))
            (make-final (if (and (pair? opt) (pair? (cdr opt)))
                            (cadr opt)
                            (lambda (x) ""))))
        (let loop ((seed seed) (acc base))
          (if (p seed)
              (string-append acc (make-final seed))
              (loop (g seed)
                    (string-append acc (string (f seed))))))))

    ;; The mirror image: characters are assembled **right to left**, so
    ;; `base` is the *rightmost* part of the result and `make-final` makes
    ;; the leftmost. The first character `f` yields ends up adjacent to
    ;; `base`.
    (define (string-unfold-right p f g seed . opt)
      (let ((base (if (pair? opt) (car opt) ""))
            (make-final (if (and (pair? opt) (pair? (cdr opt)))
                            (cadr opt)
                            (lambda (x) ""))))
        (let loop ((seed seed) (acc base))
          (if (p seed)
              (string-append (make-final seed) acc)
              (loop (g seed)
                    (string-append (string (f seed)) acc))))))

    ;; R7RS's `string-map` takes any number of strings and stops at the
    ;; shortest; this SRFI's takes one string and an optional range. The
    ;; two shapes are told apart by what follows the first string -- a
    ;; string means R7RS, an integer means this SRFI -- so this is a
    ;; superset of both and importing the library takes nothing away.
    (define (string-map proc s . rest)
      (if (and (pair? rest) (string? (car rest)))
          (let* ((ss (cons s rest))
                 (n (apply min (map string-length ss)))
                 (out (make-string n)))
            (let loop ((i 0))
              (if (= i n)
                  out
                  (begin (string-set! out i
                                      (apply proc (map (lambda (x) (string-ref x i))
                                                       ss)))
                         (loop (+ i 1))))))
          (let-values (((start end) (%range s rest)))
            (let* ((n (- end start)) (out (make-string n)))
              (let loop ((i 0))
                (if (= i n)
                    out
                    (begin (string-set! out i (proc (string-ref s (+ start i))))
                           (loop (+ i 1)))))))))

    (define (string-for-each proc s . rest)
      (if (and (pair? rest) (string? (car rest)))
          (let* ((ss (cons s rest))
                 (n (apply min (map string-length ss))))
            (let loop ((i 0))
              (if (< i n)
                  (begin (apply proc (map (lambda (x) (string-ref x i)) ss))
                         (loop (+ i 1))))))
          (let-values (((start end) (%range s rest)))
            (let loop ((i start))
              (if (< i end)
                  (begin (proc (string-ref s i)) (loop (+ i 1))))))))

    ;; Mutates `s`.
    (define (string-map! proc s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start))
          (if (< i end)
              (begin (string-set! s i (proc (string-ref s i)))
                     (loop (+ i 1)))))))

    ;; Applied to the index, not the character.
    (define (string-for-each-index proc s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start))
          (if (< i end)
              (begin (proc i) (loop (+ i 1)))))))

    ;; ── replicate and rotate ──────────────────────────────────────────
    ;;
    ;; `s[start:end]` replicated in **both** directions without end, then
    ;; sliced from `from` to `to`. So `from` and `to` may be negative, and
    ;; `to` defaults to one period past `from`.
    (define (%xsubstring s from opt who)
      (let* ((to-given? (pair? opt))
             (rest (if to-given? (cdr opt) '())))
        (let-values (((start end) (%range s rest)))
          (let* ((period (- end start))
                 (to (if to-given? (car opt) (+ from period))))
            (if (and (= period 0) (< from to))
                (error (string-append
                         who ": cannot replicate an empty range")
                       s))
            (let* ((n (- to from)) (out (make-string n)))
              (let loop ((i 0))
                (if (= i n)
                    out
                    (begin (string-set!
                             out i
                             (string-ref s (+ start (modulo (+ from i) period))))
                           (loop (+ i 1))))))))))

    (define (xsubstring s from . opt) (%xsubstring s from opt "xsubstring"))

    ;; Writes that same slice into `target` at `tstart`.
    (define (string-xcopy! target tstart s sfrom . opt)
      (let ((piece (%xsubstring s sfrom opt "string-xcopy!")))
        (let loop ((i 0))
          (if (< i (string-length piece))
              (begin (string-set! target (+ tstart i) (string-ref piece i))
                     (loop (+ i 1)))))))

    ;; ── miscellaneous ─────────────────────────────────────────────────
    ;;
    ;; `s1[0:start1]` ++ `s2[start2:end2]` ++ `s1[end1:]`.
    (define (string-replace s1 s2 start1 end1 . opt)
      (let ((start2 (if (pair? opt) (car opt) 0))
            (end2 (if (and (pair? opt) (pair? (cdr opt)))
                      (cadr opt)
                      (string-length s2))))
        (string-append (%substring s1 0 start1)
                       (%substring s2 start2 end2)
                       (%substring s1 end1 (string-length s1)))))

    ;; Maximal runs of characters in the token set, which defaults to
    ;; `char-set:graphic` -- so by default this splits on whitespace and
    ;; control characters.
    (define (string-tokenize s . opt)
      (let-values (((criterion rest) (%token-args opt)))
        (let ((p (%pred criterion "string-tokenize")))
          (let-values (((start end) (%range s rest)))
            (let loop ((i (- end 1)) (run-end #f) (acc '()))
              (cond ((< i start)
                     (if run-end
                         (cons (%substring s start run-end) acc)
                         acc))
                    ((p (string-ref s i))
                     (loop (- i 1) (if run-end run-end (+ i 1)) acc))
                    (run-end
                     (loop (- i 1) #f
                           (cons (%substring s (+ i 1) run-end) acc)))
                    (else (loop (- i 1) #f acc))))))))

    (define (%token-args opt)
      (if (and (pair? opt)
               (or (char? (car opt)) (char-set? (car opt))
                   (procedure? (car opt))))
          (values (car opt) (cdr opt))
          (values char-set:graphic opt)))

    ;; ── filtering and deleting ────────────────────────────────────────
    ;;
    ;; **Criterion first**, unlike `string-index` and `string-count`.
    (define (string-filter criterion s . opt)
      (let ((p (%pred criterion "string-filter")))
        (let-values (((start end) (%range s opt)))
          (%keep s start end p))))

    (define (string-delete criterion s . opt)
      (let ((p (%pred criterion "string-delete")))
        (let-values (((start end) (%range s opt)))
          (%keep s start end (lambda (c) (not (p c)))))))

    (define (%keep s start end keep?)
      (let loop ((i (- end 1)) (acc '()))
        (if (< i start)
            (list->string acc)
            (loop (- i 1)
                  (let ((c (string-ref s i)))
                    (if (keep? c) (cons c acc) acc))))))))
