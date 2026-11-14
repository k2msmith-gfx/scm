;## 00-base.scm — what the editor's shell needs before anything else
;## (docs/editor-crate-plan.md, E3a). Loaded first of `edit/scm/`, which the
;## host loads before its own prelude: the modeler after its vocabulary
;## baseline and before 00-data, so these are its names too.
;##
;## What the shell leans on from the image beneath it, besides scm's own
;## (its R7RS, SRFI 1, 2, 125 and 204) — the list a standalone host supplies
;## (E4); `the_shell_runs_on_a_bare_interpreter` holds it:
;##   from ray's prelude   list-sort, string-split, string-contains,
;##                        string-prefix?, string-suffix?, string-trim-both,
;##                        string-trim-right, string-pad-right
;##   from the modeler     spit, slurp
;##   kernel prims         %focused-pane, %typing-pane, %focus-pane, %pane-at,
;##                        %pane-rect (the pane registry); %hold-typing,
;##                        %release-typing (the keyboard); %set-pie-delay (the
;##                        marking menu's hold timer is the host's loop);
;##                        %os-clock, %os-dir, %os-stat, %os-realpath

;; Hash tables by their SRFI-125 names — the keymaps are tables, made as
;; this loads, before the host's own files import it.
(import (srfi 125))

;; `match` (SRFI 204) and `and-let*` (SRFI 2): scm's own libraries, imported
;; here rather than left to the host (ray's prelude imports them too), since
;; the help screens take bindings apart with them.
(import (srfi 2) (srfi 204))

(define (%env-ref name default)
  "The value NAME is bound to in the live environment, or DEFAULT when it is
unbound. How a forward reference reads a name a LATER file defines, and how
a key bound to a symbol finds the verb's CURRENT definition at press time.
A name bound to #f answers #f, as an unbound one does when DEFAULT is #f."
  (guard (e (#t default)) (eval name (interaction-environment))))

(define (%key-string k)
  "A NAME as a prim takes it: `:select-ink` and \"select-ink\" both answer
\"select-ink\", as Janet's (string k) accepted either. A symbol is its name."
  (cond ((keyword? k) (keyword->string k))
        ((symbol? k) (symbol->string k))
        (else k)))

;; The small kit the menus (03-menus) are written in, from the modeler's
;; 00-helpers (E3b).

(define (%as-list xs)
  "XS as a list: a vector is converted, a list passes through."
  (if (vector? xs) (vector->list xs) xs))
; The modeler's 00-helpers REDEFINES %as-list, with a clause for its Janet
; data's array records, and its definition is the one every caller then
; reaches — the menus included, whose items can come back from a session
; file as one.

(define (%list-ref/default xs i default)
  "Element I of the list XS, or DEFAULT when I is past either end or is not
an integer. An index that comes from a click or a stored id may name an
object that has since gone, and that is an answer, not an error."
  (if (and (integer? i) (>= i 0))
      (let loop ((xs xs) (k (exact i)))
        (cond ((not (pair? xs)) default)
              ((zero? k) (car xs))
              (else (loop (cdr xs) (- k 1)))))
      default))

(define (%text x)
  "X as text, the way a label or a tip wants it: a string is itself, a
keyword or symbol its NAME (no colon), a number its digits, #f the empty
string."
  (cond ((string? x) x)
        ((not x) "")
        ((keyword? x) (keyword->string x))
        ((symbol? x) (symbol->string x))
        ((number? x) (number->string x))
        (else (format "~a" x))))

(define %pi (* 4 (atan 1)))

(define (%make-state . kvs)
  "A small MUTABLE record — a hash table from alternating keys and values.
For the in-flight state a gesture edits as it goes (an open menu's
selection, a pie's armed sector); a record built once and read is an alist."
  (let ((t (make-hash-table equal?)))
    (let loop ((p kvs))
      (unless (or (null? p) (null? (cdr p)))
        (hash-table-set! t (car p) (cadr p))
        (loop (cddr p))))
    t))

(define (%state-ref t k) (hash-table-ref/default t k #f))

;; What echo, the input editor and M-x are written in, from the modeler's
;; 00-helpers and 99-scheme-only (E3c).

(define (%as-keyword x)
  "X as a keyword: a keyword is itself, a string or symbol is named, #f stays
#f. A name arrives as any of the three — :curve, \"curve\", 'curve."
  (cond ((keyword? x) x)
        ((string? x) (string->keyword x))
        ((symbol? x) (string->keyword (symbol->string x)))
        (else x)))

(define (%alist-ref a k default)
  "Field K of the alist A, or DEFAULT when A has none (or is not a list). A
record read back from a file, or answered by a ray verb, is an alist."
  (let ((p (and (pair? a) (assq k a)))) (if p (cdr p) default)))

(define (%number-text x)
  "X as a status line prints a number: a whole value without a trailing .0
(2.0 is \"2\"), anything else as Scheme writes it. Janet had one number type
and printed 2.0 as 2; every echo, readout and transcript line the prelude
writes was made under that rule, so this keeps it."
  (if (and (inexact? x) (integer? x) (< (abs x) 1e15))
      (number->string (exact x))
      (number->string x)))

(define (%describe-part x)
  "X as one PART of a status line — what `echo` and the transcripts join:
a string as it is, #f as nothing, #t as \"true\", a keyword or symbol by
NAME (no colon), a number by %number-text, anything else as `display` would
show it. The compat layer's `describe`, whose rules every message in the
prelude was written against."
  (cond ((string? x) x)
        ((not x) "")
        ((eq? x #t) "true")
        ((keyword? x) (keyword->string x))
        ((symbol? x) (symbol->string x))
        ((number? x) (%number-text x))
        ((char? x) (make-string 1 x))
        (else (format "~a" x))))

(define (eval-string src)
  "Every form in the Scheme string SRC, in order, in this image; the last
form's value is the answer — Janet's `eval-string`, which the prelude was
first written against, over Scheme source.

A Janet scene cannot run here: the reader stops at the first Janet-only
form it meets. scripts/dev/janet_to_scheme.py converts one."
  (let ((port (open-input-string src)))
    (let loop ((form (read port)) (last #f))
      (if (eof-object? form)
          last
          (loop (read port) (eval form (interaction-environment)))))))
