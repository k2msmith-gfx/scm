;;; (srfi 130) — Cursor-based string library.
;;;
;;; SRFI 13 again, with three changes of direction: subranges are given by
;;; *cursors* rather than indices, the searches answer a cursor instead of
;;; `#f`, and the comparison, case-mapping and mutation operations are
;;; gone. Written against the specification; the reference implementation
;;; was read for `string-split`'s `limit`, `string-replicate`'s error
;;; cases, and `string-contains-right`'s empty-pattern answer.
;;;
;;; **Cursors here are indices.** The SRFI requires only that cursors be
;;; "either disjoint from indexes or identical to indexes", and says an
;;; implementation may make them identical where "there is no benefit in
;;; implementing them separately". The whole point of the abstraction is
;;; strings stored as UTF-8 or UTF-16, where the *n*th character is not at
;;; byte *n* and an index costs a scan. scm's strings are `Vec<char>`
;;; (plan §4.3), so `string-ref` is already O(1) and a cursor has nothing
;;; left to buy. So a cursor is an exact non-negative integer,
;;; `string-cursor->index` and `string-index->cursor` are the identity,
;;; `string-cursor-diff` is subtraction, and the reference
;;; implementation's own portable layer does exactly this.
;;;
;;; Two things that follow, and are tested. Cursors must be disjoint from
;;; `#f`, and integers are. And every procedure that takes a `start`/`end`
;;; takes a cursor or an index without needing to tell them apart, because
;;; here there is nothing to tell apart.
;;;
;;; **The searches cannot answer `#f`.** That is what a cursor being
;;; disjoint from `#f` costs: `string-index` answers the `end` cursor when
;;; nothing matches, and `string-index-right` the `start` cursor. And
;;; `string-index-right` answers the **successor** of the character it
;;; found, so that whatever comes back can be cut at. The one place this
;;; is not carried through is `string-contains` and
;;; `string-contains-right`, which the specification does let answer `#f`
;;; — "found at the end" and "not found" both need an answer and there is
;;; no spare cursor between them.
;;;
;;; **`string-trim` defaults to `char-whitespace?`**, where `(srfi 13)`'s
;;; defaults to `(srfi 14)`'s `char-set:whitespace`. That is not a
;;; restatement: char-sets here are Latin-1, so `(srfi 13)` does not trim
;;; U+0085 NEL and this library does. Where the two libraries differ, the
;;; test file asserts the difference rather than leaving it to be found.
;;;
;;; **Built on `(srfi 13)`.** Twenty-eight of these procedures are that
;;; library's, imported by name and re-exported rather than written twice
;;; — the same reasoning as `(srfi 133)`'s sixteen re-exports. The eight
;;; that differ are imported under private names and adjusted, so there is
;;; still one implementation of each piece of work: the trims pass an
;;; explicit predicate, the searches change the sentinel, and
;;; `string-replicate` is `xsubstring` with `to` made mandatory.
;;;
;;; One welcome consequence of the dropped operations: this library does
;;; **not** export `string-map` or `string-for-each`, so unlike
;;; `(srfi 13)` it has no collision with R7RS at all. `string-for-each`'s
;;; replacement, `string-for-each-cursor`, hands the procedure the cursor
;;; rather than the character.
(define-library (srfi 130)
  (export
    ;; cursors
    string-cursor? string-cursor-start string-cursor-end
    string-cursor-next string-cursor-prev
    string-cursor-forward string-cursor-back
    string-cursor=? string-cursor<? string-cursor>?
    string-cursor<=? string-cursor>=?
    string-cursor-diff string-cursor->index string-index->cursor
    ;; predicates
    string-null? string-every string-any
    ;; constructors
    string-tabulate string-unfold string-unfold-right
    ;; conversion
    string->list/cursors string->vector/cursors
    reverse-list->string string-join
    ;; selection
    string-ref/cursor substring/cursors string-copy/cursors
    string-take string-take-right string-drop string-drop-right
    string-pad string-pad-right
    string-trim string-trim-right string-trim-both
    ;; prefixes and suffixes
    string-prefix-length string-suffix-length
    string-prefix? string-suffix?
    ;; searching
    string-index string-index-right string-skip string-skip-right
    string-contains string-contains-right
    ;; the whole string
    string-reverse string-concatenate string-concatenate-reverse
    string-fold string-fold-right string-for-each-cursor
    string-replicate string-count
    string-replace string-split
    string-filter string-remove)
  (import
    (scheme base)
    ;; The twenty-eight that are SRFI 13's unchanged.
    (only (srfi 13)
          string-null? string-every string-any
          string-tabulate string-unfold string-unfold-right
          reverse-list->string string-join
          string-take string-take-right string-drop string-drop-right
          string-pad string-pad-right
          string-prefix-length string-suffix-length
          string-prefix? string-suffix?
          string-reverse string-concatenate string-concatenate-reverse
          string-fold string-fold-right
          string-count string-replace string-filter string-contains)
    ;; `string-delete` is this SRFI's `string-remove`, unchanged but for
    ;; the name; the rest are the ones adjusted below.
    (rename (only (srfi 13)
                  string-delete
                  string-index string-index-right
                  string-skip string-skip-right
                  string-trim string-trim-right string-trim-both
                  xsubstring)
            (string-delete string-remove)
            (string-index %13-index)
            (string-index-right %13-index-right)
            (string-skip %13-skip)
            (string-skip-right %13-skip-right)
            (string-trim %13-trim)
            (string-trim-right %13-trim-right)
            (string-trim-both %13-trim-both)
            (xsubstring %13-xsubstring)))
  (begin

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

    ;; ── cursors, which here are indices ───────────────────────────────
    ;;
    ;; A cursor is an exact non-negative integer. Every one of these is
    ;; therefore trivial, which is the point: the abstraction exists for
    ;; implementations where it is not.
    (define (string-cursor? x) (and (exact-integer? x) (>= x 0)))

    (define (string-cursor-start s) 0)
    (define (string-cursor-end s) (string-length s))
    (define (string-cursor-next s cursor) (+ cursor 1))
    (define (string-cursor-prev s cursor) (- cursor 1))
    (define (string-cursor-forward s cursor n) (+ cursor n))
    (define (string-cursor-back s cursor n) (- cursor n))

    (define (string-cursor=? a b) (= a b))
    (define (string-cursor<? a b) (< a b))
    (define (string-cursor>? a b) (> a b))
    (define (string-cursor<=? a b) (<= a b))
    (define (string-cursor>=? a b) (>= a b))

    (define (string-cursor-diff s start end) (- end start))

    ;; The identity, both ways, because the two representations are the
    ;; same representation. The SRFI specifies these as accepting either,
    ;; which is free here.
    (define (string-cursor->index s cursor) cursor)
    (define (string-index->cursor s index) index)

    ;; ── the /cursors and /cursor spellings ────────────────────────────
    ;;
    ;; Named apart from their R7RS counterparts because elsewhere they
    ;; would take a different kind of argument. Here they are the same
    ;; procedures, and aliasing them is more honest than copying them.
    (define string->list/cursors string->list)
    (define string->vector/cursors string->vector)
    (define string-ref/cursor string-ref)
    (define substring/cursors substring)
    (define string-copy/cursors string-copy)

    ;; ── trimming, which defaults to char-whitespace? ──────────────────
    ;;
    ;; `(srfi 13)` defaults to `(srfi 14)`'s `char-set:whitespace`, which
    ;; is Latin-1. This defaults to `char-whitespace?`, which is
    ;; Unicode-correct over scm's whole character range -- so the two
    ;; libraries genuinely differ on a character like U+0085 NEL.
    (define (%trim-args opt)
      (if (pair? opt)
          (values (car opt) (cdr opt))
          (values char-whitespace? '())))

    (define (string-trim s . opt)
      (let-values (((pred rest) (%trim-args opt)))
        (apply %13-trim s pred rest)))

    (define (string-trim-right s . opt)
      (let-values (((pred rest) (%trim-args opt)))
        (apply %13-trim-right s pred rest)))

    (define (string-trim-both s . opt)
      (let-values (((pred rest) (%trim-args opt)))
        (apply %13-trim-both s pred rest)))

    ;; ── searching, where "not found" is a cursor ──────────────────────
    ;;
    ;; `string-index` answers `end` and `string-index-right` answers
    ;; `start` when nothing matches -- never `#f`, which a cursor must be
    ;; disjoint from. Note what that costs: a match *at* the end of the
    ;; range and no match at all give the same answer, so a caller who
    ;; needs to tell them apart compares against `end` first.
    (define (string-index s criterion . opt)
      (let-values (((start end) (%range s opt)))
        (or (%13-index s criterion start end) end)))

    ;; The **successor** of the rightmost match, so that the result is a
    ;; cursor you can cut a substring at; `start` when nothing matches.
    (define (string-index-right s criterion . opt)
      (let-values (((start end) (%range s opt)))
        (let ((i (%13-index-right s criterion start end)))
          (if i (+ i 1) start))))

    (define (string-skip s criterion . opt)
      (let-values (((start end) (%range s opt)))
        (or (%13-skip s criterion start end) end)))

    (define (string-skip-right s criterion . opt)
      (let-values (((start end) (%range s opt)))
        (let ((i (%13-skip-right s criterion start end)))
          (if i (+ i 1) start))))

    ;; The rightmost occurrence, scanning from the right rather than
    ;; running the leftward search repeatedly as the reference does. An
    ;; empty pattern is found at `end1` here, where `string-contains`
    ;; finds it at `start1` -- leftmost and rightmost respectively, which
    ;; is consistent even though the pair of answers looks odd.
    (define (string-contains-right s1 s2 . opt)
      (let-values (((start1 end1 start2 end2) (%range2 s1 s2 opt)))
        (let ((plen (- end2 start2)))
          (if (= plen 0)
              end1
              (let loop ((i (- end1 plen)))
                (cond ((< i start1) #f)
                      ((let match ((j 0))
                         (or (= j plen)
                             (and (char=? (string-ref s1 (+ i j))
                                          (string-ref s2 (+ start2 j)))
                                  (match (+ j 1)))))
                       i)
                      (else (loop (- i 1)))))))))

    ;; ── the whole string ──────────────────────────────────────────────
    ;;
    ;; Hands the procedure the **cursor**, not the character -- which is
    ;; what replaces `(srfi 13)`'s `string-for-each` here.
    (define (string-for-each-cursor proc s . opt)
      (let-values (((start end) (%range s opt)))
        (let loop ((i start))
          (if (< i end)
              (begin (proc i) (loop (+ i 1)))))))

    ;; `xsubstring` with `to` made mandatory and `from > to` made an
    ;; error, which are the only differences.
    (define (string-replicate s from to . opt)
      (if (> from to)
          (error "string-replicate: from is greater than to" from to)
          (apply %13-xsubstring s from to opt)))

    ;; `(string-split s delimiter [grammar limit start end])`. The
    ;; delimiter is a **string**, not a criterion -- this is not
    ;; `(srfi 13)`'s `string-tokenize`, which keeps the runs that match a
    ;; char-set; this one cuts at a fixed word and keeps what is between,
    ;; so adjacent delimiters give empty pieces.
    ;;
    ;; `limit` is the maximum number of *splits*, so the result holds at
    ;; most `limit + 1` pieces and the last one keeps whatever is left,
    ;; delimiters and all. An empty delimiter splits into characters.
    (define (string-split s delimiter . opt)
      (let* ((grammar (if (pair? opt) (car opt) 'infix))
             (r1 (if (pair? opt) (cdr opt) '()))
             (limit (if (pair? r1) (car r1) #f))
             (r2 (if (pair? r1) (cdr r1) '())))
        (let-values (((start end) (%range s r2)))
          (let* ((text (substring s start end))
                 (limit (or limit (string-length text)))
                 (pieces (if (= 0 (string-length delimiter))
                             (%split-chars text limit)
                             (%split-word text delimiter limit))))
            (cond ((memq grammar '(infix strict-infix))
                   (if (= 0 (string-length text))
                       (if (eq? grammar 'infix)
                           '()
                           (error "string-split: an empty string cannot be split with the strict-infix grammar"))
                       pieces))
                  ((eq? grammar 'prefix)
                   (if (and (pair? pieces) (= 0 (string-length (car pieces))))
                       (cdr pieces)
                       pieces))
                  ((eq? grammar 'suffix)
                   (if (and (pair? pieces)
                            (= 0 (string-length (%last pieces))))
                       (%drop-last pieces)
                       pieces))
                  (else (error "string-split: unknown grammar" grammar)))))))

    (define (%last lst)
      (if (null? (cdr lst)) (car lst) (%last (cdr lst))))

    (define (%drop-last lst)
      (if (null? (cdr lst)) '() (cons (car lst) (%drop-last (cdr lst)))))

    (define (%split-chars text limit)
      (let ((n (string-length text)))
        (if (<= n limit)
            (map string (string->list text))
            (append (map string (string->list (substring text 0 limit)))
                    (list (substring text limit n))))))

    (define (%split-word text sep limit)
      (if (= limit 0)
          (list text)
          (let ((i (string-contains text sep)))
            (if i
                (cons (substring text 0 i)
                      (%split-word (substring text (+ i (string-length sep))
                                              (string-length text))
                                   sep
                                   (- limit 1)))
                (list text)))))))
