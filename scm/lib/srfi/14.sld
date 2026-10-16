;;; (srfi 14) — Character-set Library.
;;;
;;; Written against the SRFI 14 specification rather than copied from its
;;; reference implementation. The reference was read on four points: the
;;; corrected argument order of `char-set-unfold`, the cursor protocol,
;;; the `error?` behaviour of `ucs-range->char-set`, and the exact
;;; contents of the seventeen standard sets, which are Latin-1 *data* and
;;; not something to derive.
;;;
;;; **Char-sets here are Latin-1: code points 0 to 255.** Three things say
;;; this is the right bound rather than a shortcut. The SRFI itself makes
;;; `char-set:full` and `char-set-complement` relative to "the
;;; implementation's character range" and warns that a Unicode-wide
;;; implementation finds them expensive. scm does not claim the
;;; `full-unicode` feature (plan §2.2), so its declared character range
;;; is already smaller than its `char` type. And the reference
;;; implementation is itself Latin-1, in its own words: "Is Latin-1
;;; specific. Would certainly have to be rewritten for Unicode."
;;;
;;; What that costs: scm's characters really are Unicode scalar values, so
;;; `(char-set-contains? char-set:letter #\λ)` is `#f` while
;;; `(char-alphabetic? #\λ)` is `#t`. **For Unicode-correct behaviour, pass
;;; a predicate rather than a char-set** — every `(srfi 13)` procedure that
;;; takes a char-set takes a predicate instead, and scm's `char-alphabetic?`
;;; and friends are Unicode-correct over the whole range.
;;;
;;; The standard sets below are the reference implementation's Latin-1 data
;;; copied verbatim, which for Latin-1 *is* the specification of what they
;;; hold. Three of them therefore disagree with scm's native predicates
;;; *inside* the representable range, and the test file pins each exactly:
;;;
;;;   char-set:digit       vs char-numeric?     — 6: ² ³ ¹ ¼ ½ ¾
;;;   char-set:lower-case  vs char-lower-case?  — 2: ª º
;;;   char-set:whitespace  vs char-whitespace?  — 1: U+0085 NEL
;;;
;;; None is a bug in either party. `char-set:digit` is the *decimal* digits
;;; where Rust's `is_numeric` is Unicode `Nd`+`Nl`+`No`; `char-lower-case?`
;;; follows the Unicode Lowercase property, which includes the two ordinal
;;; indicators through `Other_Lowercase`, while the reference files them
;;; under `char-set:letter` only; and `char-whitespace?` counts NEL, a C1
;;; control, which the reference's list omits. Adding NEL to make them agree
;;; would put a control character into `char-set:printing`, which is defined
;;; as whitespace union graphic — so the verbatim data is kept and the
;;; divergence is documented and tested rather than patched set by set.
;;;
;;; A set is a 256-byte bytevector, one byte per code point. That is the
;;; reference's structure with the improvement it asks for in its own
;;; notes ("Should be rewritten to use bit strings or byte vecs"), and
;;; scm's bytevectors are native and unboxed.
;;;
;;; **The sixteen linear-update `!` variants are the non-destructive
;;; versions**, which is what `(srfi 1)` does here for the same reason:
;;; the SRFI permits an implementation to reuse its argument's storage or
;;; not, and "not" is the version that cannot surprise a caller who kept a
;;; reference. It also removes a hazard the reference implementation warns
;;; about and cannot fix — that `(char-set-adjoin! char-set:digit #\x)`
;;; would corrupt a shared constant for the whole program.
(define-library (srfi 14)
  (export
    ;; predicates and comparison
    char-set? char-set= char-set<= char-set-hash
    ;; iterating
    char-set-cursor char-set-ref char-set-cursor-next end-of-char-set?
    char-set-fold char-set-unfold char-set-unfold!
    char-set-for-each char-set-map
    ;; creating
    char-set-copy char-set
    list->char-set list->char-set! string->char-set string->char-set!
    char-set-filter char-set-filter!
    ucs-range->char-set ucs-range->char-set! ->char-set
    ;; querying
    char-set-size char-set-count char-set->list char-set->string
    char-set-contains? char-set-every char-set-any
    ;; algebra
    char-set-adjoin char-set-delete char-set-adjoin! char-set-delete!
    char-set-complement char-set-union char-set-intersection
    char-set-difference char-set-xor char-set-diff+intersection
    char-set-complement! char-set-union! char-set-intersection!
    char-set-difference! char-set-xor! char-set-diff+intersection!
    ;; the standard sets
    char-set:lower-case char-set:upper-case char-set:title-case
    char-set:letter char-set:digit char-set:letter+digit
    char-set:graphic char-set:printing char-set:whitespace
    char-set:iso-control char-set:punctuation char-set:symbol
    char-set:hex-digit char-set:blank char-set:ascii
    char-set:empty char-set:full)
  (import (scheme base))
  (begin

    ;; ── the representation ────────────────────────────────────────────
    ;;
    ;; One byte per code point, 1 for present. `%limit` is the single
    ;; place the Latin-1 bound is written down.
    (define %limit 256)

    (define-record-type <char-set> (%wrap bits) char-set? (bits %raw))

    (define (%bits cs who)
      (if (char-set? cs)
          (%raw cs)
          (error (string-append who ": expects a char-set") cs)))

    (define (%fresh) (make-bytevector %limit 0))
    (define (%full-bits) (make-bytevector %limit 1))
    (define (%in? bits i) (= 1 (bytevector-u8-ref bits i)))
    (define (%add! bits i) (bytevector-u8-set! bits i 1))
    (define (%drop! bits i) (bytevector-u8-set! bits i 0))

    ;; A character's index, **enforcing the bound**. A character past
    ;; Latin-1 cannot be stored, so putting one in is an error rather
    ;; than a silent omission. Asking whether one is a member is not an
    ;; error -- the answer is simply no -- and taking one out is not
    ;; either, since there is nothing there to take.
    (define (%index char who)
      (let ((n (char->integer char)))
        (if (< n %limit)
            n
            (error (string-append
                     who ": char-sets in this implementation are Latin-1,"
                         " so this character cannot be put in one")
                   char))))

    ;; The optional trailing `base-cs`, as a fresh bitmap. Always a copy,
    ;; which is what makes every `!` variant here non-destructive.
    (define (%base opt who)
      (if (pair? opt)
          (bytevector-copy (%bits (car opt) who))
          (%fresh)))

    ;; Every algebraic operation is this loop under a different combiner.
    (define (%zip! out b f)
      (let loop ((i 0))
        (if (< i %limit)
            (begin (bytevector-u8-set!
                     out i (f (bytevector-u8-ref out i)
                              (bytevector-u8-ref b i)))
                   (loop (+ i 1))))))

    (define (%or a b) (if (or (= a 1) (= b 1)) 1 0))
    (define (%and a b) (if (and (= a 1) (= b 1)) 1 0))
    (define (%andnot a b) (if (and (= a 1) (= b 0)) 1 0))
    (define (%xor-byte a b) (if (= a b) 0 1))

    ;; ── predicates and comparison ─────────────────────────────────────
    (define (char-set= . css)
      (or (null? css)
          (let ((first (%bits (car css) "char-set=")))
            (let loop ((rest (cdr css)))
              (or (null? rest)
                  (and (%bits=? first (%bits (car rest) "char-set="))
                       (loop (cdr rest))))))))

    (define (%bits=? a b)
      (let loop ((i 0))
        (cond ((= i %limit) #t)
              ((= (bytevector-u8-ref a i) (bytevector-u8-ref b i))
               (loop (+ i 1)))
              (else #f))))

    ;; Each set a subset of the one after it, so the chain is read
    ;; pairwise and not every pair compared.
    (define (char-set<= . css)
      (or (null? css)
          (let loop ((a (%bits (car css) "char-set<=")) (rest (cdr css)))
            (or (null? rest)
                (let ((b (%bits (car rest) "char-set<=")))
                  (and (%subset? a b) (loop b (cdr rest))))))))

    (define (%subset? a b)
      (let loop ((i 0))
        (cond ((= i %limit) #t)
              ((and (= 1 (bytevector-u8-ref a i))
                    (= 0 (bytevector-u8-ref b i)))
               #f)
              (else (loop (+ i 1))))))

    ;; `bound` defaults to 2^22 and 0 asks for that default, as the
    ;; reference does. The accumulator is reduced at **every step**, which
    ;; is how a bounded hash is written rather than a workaround: with
    ;; `exact-closed` claimed (S18) the unreduced form would work, but
    ;; `(* 37 h)` over 256 code points would build a 1300-bit integer to
    ;; throw away. Reducing modulo 2^32 keeps `37 * h` far inside an i64.
    (define (char-set-hash cs . opt)
      (let ((bound (if (and (pair? opt) (not (eqv? 0 (car opt))))
                       (car opt)
                       4194304))
            (bits (%bits cs "char-set-hash")))
        (let loop ((i 0) (h 0))
          (if (= i %limit)
              (modulo h bound)
              (loop (+ i 1)
                    (if (%in? bits i)
                        (modulo (+ (* 37 h) i) 4294967296)
                        h))))))

    ;; ── iterating ─────────────────────────────────────────────────────
    ;;
    ;; A cursor is the index of a member, and `%limit` is the
    ;; past-the-end value. `end-of-char-set?` is handed only the cursor,
    ;; so the sentinel has to identify itself; the reference counts down
    ;; and uses a negative one, and this counts **up**, so that
    ;; `char-set->list` comes out in code-point order rather than
    ;; reversed.
    (define (%next-member bits i)
      (let loop ((i i))
        (cond ((>= i %limit) %limit)
              ((%in? bits i) i)
              (else (loop (+ i 1))))))

    (define (char-set-cursor cs)
      (%next-member (%bits cs "char-set-cursor") 0))

    (define (end-of-char-set? cursor) (>= cursor %limit))

    (define (char-set-ref cs cursor)
      (if (end-of-char-set? cursor)
          (error "char-set-ref: the cursor is past the end of the set" cursor)
          (integer->char cursor)))

    (define (char-set-cursor-next cs cursor)
      (%next-member (%bits cs "char-set-cursor-next") (+ cursor 1)))

    ;; `(kons char state)` -- **the character first**. This project ships
    ;; three folds with three orders: `(srfi 1)`'s `fold` is
    ;; `(f elem acc)`, `(srfi 133)`'s `vector-fold` is `(kons state elt)`,
    ;; and this one is `(kons char state)`. The tests pin it with a `kons`
    ;; that is not commutative.
    (define (char-set-fold kons knil cs)
      (let ((bits (%bits cs "char-set-fold")))
        (let loop ((i 0) (acc knil))
          (cond ((= i %limit) acc)
                ((%in? bits i) (loop (+ i 1) (kons (integer->char i) acc)))
                (else (loop (+ i 1) acc))))))

    ;; `(char-set-unfold p f g seed [base])`: **`p` says when to stop**,
    ;; `f` makes a character from the seed, `g` makes the next seed. The
    ;; first two arguments were the other way round in the SRFI's
    ;; original signature and were corrected by erratum -- the examples
    ;; and the reference implementation always used this order.
    (define (char-set-unfold p f g seed . opt)
      (let ((bits (%base opt "char-set-unfold")))
        (%unfold-into! bits p f g seed "char-set-unfold")
        (%wrap bits)))

    (define (char-set-unfold! p f g seed base)
      (char-set-unfold p f g seed base))

    (define (%unfold-into! bits p f g seed who)
      (let loop ((seed seed))
        (if (not (p seed))
            (begin (%add! bits (%index (f seed) who))
                   (loop (g seed))))))

    (define (char-set-for-each proc cs)
      (let ((bits (%bits cs "char-set-for-each")))
        (let loop ((i 0))
          (if (< i %limit)
              (begin (if (%in? bits i) (proc (integer->char i)))
                     (loop (+ i 1)))))))

    (define (char-set-map proc cs)
      (let ((src (%bits cs "char-set-map"))
            (out (%fresh)))
        (let loop ((i 0))
          (if (= i %limit)
              (%wrap out)
              (begin (if (%in? src i)
                         (%add! out (%index (proc (integer->char i))
                                            "char-set-map")))
                     (loop (+ i 1)))))))

    ;; ── creating ──────────────────────────────────────────────────────
    (define (char-set-copy cs)
      (%wrap (bytevector-copy (%bits cs "char-set-copy"))))

    (define (char-set . chars)
      (%wrap (%chars-into! (%fresh) chars "char-set")))

    (define (%chars-into! bits chars who)
      (for-each (lambda (c) (%add! bits (%index c who))) chars)
      bits)

    (define (list->char-set chars . opt)
      (%wrap (%chars-into! (%base opt "list->char-set")
                           chars "list->char-set")))

    (define (list->char-set! chars base)
      (list->char-set chars base))

    (define (string->char-set s . opt)
      (%wrap (%chars-into! (%base opt "string->char-set")
                           (string->list s) "string->char-set")))

    (define (string->char-set! s base)
      (string->char-set s base))

    ;; Filters the **domain** set, adding what passes to the base.
    (define (char-set-filter pred domain . opt)
      (let ((out (%base opt "char-set-filter"))
            (src (%bits domain "char-set-filter")))
        (let loop ((i 0))
          (if (= i %limit)
              (%wrap out)
              (begin (if (and (%in? src i) (pred (integer->char i)))
                         (%add! out i))
                     (loop (+ i 1)))))))

    (define (char-set-filter! pred domain base)
      (char-set-filter pred domain base))

    ;; `upper` is **exclusive**. `error?` asks for the Latin-1 bound to be
    ;; reported rather than silently clipped, and defaults to `#f`, so by
    ;; default a range running past 255 quietly stops there.
    (define (ucs-range->char-set lower upper . rest)
      (let* ((error? (and (pair? rest) (car rest)))
             (opt (if (pair? rest) (cdr rest) '()))
             (bits (%base opt "ucs-range->char-set")))
        (if (and (< lower upper) (< %limit upper) error?)
            (error (string-append
                     "ucs-range->char-set: the requested UCS range contains"
                     " characters this implementation's char-sets cannot"
                     " hold, which are Latin-1")
                   lower upper))
        (let ((stop (if (< upper %limit) upper %limit)))
          (let loop ((i lower))
            (if (< i stop)
                (begin (%add! bits i) (loop (+ i 1))))))
        (%wrap bits)))

    (define (ucs-range->char-set! lower upper error? base)
      (ucs-range->char-set lower upper error? base))

    ;; A char-set comes back **as is**, which the SRFI specifies -- so
    ;; this is not a way to copy one.
    (define (->char-set x)
      (cond ((char-set? x) x)
            ((char? x) (char-set x))
            ((string? x) (string->char-set x))
            (else (error "->char-set: expects a char-set, a character or a string" x))))

    ;; ── querying ──────────────────────────────────────────────────────
    (define (char-set-size cs)
      (let ((bits (%bits cs "char-set-size")))
        (let loop ((i 0) (n 0))
          (cond ((= i %limit) n)
                ((%in? bits i) (loop (+ i 1) (+ n 1)))
                (else (loop (+ i 1) n))))))

    (define (char-set-count pred cs)
      (let ((bits (%bits cs "char-set-count")))
        (let loop ((i 0) (n 0))
          (cond ((= i %limit) n)
                ((and (%in? bits i) (pred (integer->char i)))
                 (loop (+ i 1) (+ n 1)))
                (else (loop (+ i 1) n))))))

    ;; In code-point order. The SRFI leaves the order unspecified; this
    ;; one is worth having because it makes a set readable.
    (define (char-set->list cs)
      (let ((bits (%bits cs "char-set->list")))
        (let loop ((i (- %limit 1)) (acc '()))
          (cond ((< i 0) acc)
                ((%in? bits i) (loop (- i 1) (cons (integer->char i) acc)))
                (else (loop (- i 1) acc))))))

    (define (char-set->string cs)
      (list->string (char-set->list cs)))

    ;; Not an error for a character past Latin-1 -- it is simply not a
    ;; member of any set.
    (define (char-set-contains? cs char)
      (let ((n (char->integer char)))
        (and (< n %limit) (%in? (%bits cs "char-set-contains?") n))))

    ;; `char-set-every` answers `#t`, not the last value the predicate
    ;; gave -- unlike `(srfi 133)`'s `vector-every`, which answers the
    ;; last value. `char-set-any` does answer the predicate's value.
    ;; Empty set: `#t` and `#f` respectively.
    (define (char-set-every pred cs)
      (let ((bits (%bits cs "char-set-every")))
        (let loop ((i 0))
          (cond ((= i %limit) #t)
                ((not (%in? bits i)) (loop (+ i 1)))
                ((pred (integer->char i)) (loop (+ i 1)))
                (else #f)))))

    (define (char-set-any pred cs)
      (let ((bits (%bits cs "char-set-any")))
        (let loop ((i 0))
          (and (< i %limit)
               (if (%in? bits i)
                   (or (pred (integer->char i)) (loop (+ i 1)))
                   (loop (+ i 1)))))))

    ;; ── algebra ───────────────────────────────────────────────────────
    (define (char-set-adjoin cs . chars)
      (%wrap (%chars-into! (bytevector-copy (%bits cs "char-set-adjoin"))
                           chars "char-set-adjoin")))

    ;; Deleting a character past Latin-1 is a no-op rather than an error:
    ;; it was not in the set, and the result is right either way.
    (define (char-set-delete cs . chars)
      (let ((out (bytevector-copy (%bits cs "char-set-delete"))))
        (for-each (lambda (c)
                    (let ((n (char->integer c)))
                      (if (< n %limit) (%drop! out n))))
                  chars)
        (%wrap out)))

    (define (char-set-adjoin! cs . chars) (apply char-set-adjoin cs chars))
    (define (char-set-delete! cs . chars) (apply char-set-delete cs chars))

    ;; Relative to **this implementation's** character range, which the
    ;; SRFI says it is -- so the complement of the empty set is Latin-1
    ;; and not the whole of Unicode.
    (define (char-set-complement cs)
      (let ((src (%bits cs "char-set-complement"))
            (out (%fresh)))
        (let loop ((i 0))
          (if (= i %limit)
              (%wrap out)
              (begin (if (not (%in? src i)) (%add! out i))
                     (loop (+ i 1)))))))

    (define (char-set-union . css)
      (let ((out (%fresh)))
        (for-each (lambda (cs) (%zip! out (%bits cs "char-set-union") %or))
                  css)
        (%wrap out)))

    ;; No arguments is the **full** set, since the identity for
    ;; intersection is everything -- where for union it is nothing.
    (define (char-set-intersection . css)
      (if (null? css)
          (%wrap (%full-bits))
          (let ((out (bytevector-copy
                       (%bits (car css) "char-set-intersection"))))
            (for-each (lambda (cs)
                        (%zip! out (%bits cs "char-set-intersection") %and))
                      (cdr css))
            (%wrap out))))

    ;; `cs1` less the union of the rest.
    (define (char-set-difference cs1 . css)
      (let ((out (bytevector-copy (%bits cs1 "char-set-difference"))))
        (for-each (lambda (cs)
                    (%zip! out (%bits cs "char-set-difference") %andnot))
                  css)
        (%wrap out)))

    (define (char-set-xor . css)
      (let ((out (%fresh)))
        (for-each (lambda (cs) (%zip! out (%bits cs "char-set-xor") %xor-byte))
                  css)
        (%wrap out)))

    (define (char-set-diff+intersection cs1 . css)
      (values (apply char-set-difference cs1 css)
              (char-set-intersection cs1 (apply char-set-union css))))

    (define (char-set-complement! cs) (char-set-complement cs))
    (define (char-set-union! cs1 . css) (apply char-set-union cs1 css))
    (define (char-set-intersection! cs1 . css)
      (apply char-set-intersection cs1 css))
    (define (char-set-difference! cs1 . css)
      (apply char-set-difference cs1 css))
    (define (char-set-xor! cs1 . css) (apply char-set-xor cs1 css))
    (define (char-set-diff+intersection! cs1 cs2 . css)
      (apply char-set-diff+intersection cs1 cs2 css))

    ;; ── the standard sets ─────────────────────────────────────────────
    ;;
    ;; These come last because a library body runs in order: a set built
    ;; here calls the procedures above, which must already be defined.
    ;;
    ;; The contents are the reference implementation's, which for Latin-1
    ;; *is* the specification of what these sets hold -- deriving them
    ;; from scm's native `char-alphabetic?` and friends would be a
    ;; different set, and the test file checks where the two differ.
    (define char-set:empty (char-set))
    (define char-set:full (char-set-complement char-set:empty))

    ;; a-z, then the Latin-1 lower-case runs, then MICRO SIGN.
    (define char-set:lower-case
      (char-set-adjoin
        (char-set-union (ucs-range->char-set #x61 #x7B)
                        (ucs-range->char-set #xDF #xF7)
                        (ucs-range->char-set #xF8 #x100))
        (integer->char #xB5)))

    (define char-set:upper-case
      (char-set-union (ucs-range->char-set #x41 #x5B)
                      (ucs-range->char-set #xC0 #xD7)
                      (ucs-range->char-set #xD8 #xDF)))

    ;; Latin-1 has no title-case characters.
    (define char-set:title-case char-set:empty)

    ;; Plus the two ordinal indicators, which are letters and are neither
    ;; upper- nor lower-case.
    (define char-set:letter
      (char-set-adjoin (char-set-union char-set:upper-case char-set:lower-case)
                       (integer->char #xAA)    ; FEMININE ORDINAL INDICATOR
                       (integer->char #xBA)))  ; MASCULINE ORDINAL INDICATOR

    (define char-set:digit (string->char-set "0123456789"))
    (define char-set:hex-digit (string->char-set "0123456789abcdefABCDEF"))

    (define char-set:letter+digit
      (char-set-union char-set:letter char-set:digit))

    (define char-set:punctuation
      (char-set-union
        (string->char-set "!\"#%&'()*,-./:;?@[\\]_{}")
        (list->char-set (map integer->char
                             '(#xA1      ; INVERTED EXCLAMATION MARK
                               #xAB      ; LEFT DOUBLE ANGLE QUOTATION MARK
                               #xAD      ; SOFT HYPHEN
                               #xB7      ; MIDDLE DOT
                               #xBB      ; RIGHT DOUBLE ANGLE QUOTATION MARK
                               #xBF))))) ; INVERTED QUESTION MARK

    (define char-set:symbol
      (char-set-union
        (string->char-set "$+<=>^`|~")
        (list->char-set (map integer->char
                             '(#xA2 #xA3 #xA4 #xA5 #xA6 #xA7 #xA8 #xA9
                               #xAC #xAE #xAF #xB0 #xB1 #xB4 #xB6 #xB8
                               #xD7      ; MULTIPLICATION SIGN
                               #xF7))))) ; DIVISION SIGN

    (define char-set:graphic
      (char-set-union char-set:letter+digit
                      char-set:punctuation
                      char-set:symbol))

    (define char-set:whitespace
      (list->char-set (map integer->char
                           '(#x09    ; HORIZONTAL TABULATION
                             #x0A    ; LINE FEED
                             #x0B    ; VERTICAL TABULATION
                             #x0C    ; FORM FEED
                             #x0D    ; CARRIAGE RETURN
                             #x20    ; SPACE
                             #xA0))))  ; NO-BREAK SPACE

    (define char-set:printing
      (char-set-union char-set:whitespace char-set:graphic))

    (define char-set:blank
      (list->char-set (map integer->char '(#x09 #x20 #xA0))))

    ;; The C0 controls and the Latin-1 C1 controls.
    (define char-set:iso-control
      (char-set-union (ucs-range->char-set 0 32)
                      (ucs-range->char-set #x7F #xA0)))

    (define char-set:ascii (ucs-range->char-set 0 128))))
