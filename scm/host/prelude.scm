;## host/prelude.scm — what ray's prelude gives the shell in the modeler,
;## for a host without it: the standalone program (docs/editor-crate-plan.md,
;## E4). 00-base.scm lists what a host supplies; this is the Scheme half of
;## that list, loaded before the shell.
;##
;## COPIES, byte for byte, of ray's scripts/scm/prelude.scm — not a slice
;## of it loaded from there, because `edit` must build and run without ray.
;## `the_host_prelude_is_rays_verbatim` (scm_binding.rs) holds every form
;## here to ray's text, so a fix there turns it red until it is copied.

; SRFI-1 (filter, find, any, append-map …), which ray's image imports.
(import (srfi 1))

(define-syntax when-let
  (syntax-rules ()
    ((_ () body ...) (let () body ...))
    ((_ ((name expr) more ...) body ...)
     (let ((name expr))
       (if name (when-let (more ...) body ...) #f)))))

(define-syntax if-let
  (syntax-rules ()
    ((_ bindings then) (if-let bindings then #f))
    ((_ () then else) then)
    ((_ ((name expr) more ...) then else)
     (let ((name expr))
       (if name (if-let (more ...) then else) else)))))

(define (string-contains s pattern . start)
  "SRFI-13 (string-contains s pattern [start]) -> the index in S where
PATTERN first occurs at or after START, or #f. The haystack comes FIRST.
A START past the end of S is no match. Native: a search costs the distance
it scans, however long S is."
  (let ((from (if (pair? start) (car start) 0)))
    (and (<= from (string-length s))
         (%string-search s pattern from))))

(define (string-prefix? prefix s)
  "SRFI-13 (string-prefix? prefix s): does S start with PREFIX?"
  (and (<= (string-length prefix) (string-length s))
       (string=? (substring s 0 (string-length prefix)) prefix)))

(define (string-suffix? suffix s)
  "SRFI-13 (string-suffix? suffix s): does S end with SUFFIX?"
  (let ((n (string-length s)) (m (string-length suffix)))
    (and (<= m n) (string=? (substring s (- n m) n) suffix))))

(define (list-sort less? xs)
  "SRFI-132 (list-sort less? list) -> a new sorted list; stable. The
comparator comes FIRST; (sort list less?) is the same sort the other way
round."
  (define (merge a b)
    (cond ((null? a) b)
          ((null? b) a)
          ((less? (car b) (car a)) (cons (car b) (merge a (cdr b))))
          (else (cons (car a) (merge (cdr a) b)))))
  (define (split xs)
    (if (or (null? xs) (null? (cdr xs)))
        (values xs '())
        (let-values (((a b) (split (cddr xs))))
          (values (cons (car xs) a) (cons (cadr xs) b)))))
  (let ((xs (if (vector? xs) (vector->list xs) xs)))
    (if (or (null? xs) (null? (cdr xs)))
        xs
        (let-values (((a b) (split xs)))
          (merge (list-sort less? a) (list-sort less? b))))))

(define (string-pad-right s n . fill)
  "SRFI-13 (string-pad-right s n [char]) -> S left-aligned in N columns,
padded on the RIGHT; longer than N keeps the FIRST N characters."
  (let ((c (if (pair? fill) (car fill) #\space)) (m (string-length s)))
    (if (>= m n)
        (substring s 0 n)
        (string-append s (make-string (- n m) c)))))

(define (string-split s delimiter)
  "SRFI-152 (string-split s delimiter) -> every field of S between
occurrences of the DELIMITER string, empty ones included: (string-split
\"a  b\" \" \") is (\"a\" \"\" \"b\"). The string comes FIRST. An empty
delimiter answers (S)."
  (let ((m (string-length delimiter)) (n (string-length s)))
    (if (zero? m)
        (list s)
        (let loop ((start 0) (acc '()))
          (let ((hit (string-contains s delimiter start)))
            (if hit
                (loop (+ hit m) (cons (substring s start hit) acc))
                (reverse (cons (substring s start n) acc))))))))

(define (string-trim s)
  "SRFI-13 (string-trim s): S without its LEADING whitespace."
  (let loop ((i 0))
    (if (and (< i (string-length s)) (char-whitespace? (string-ref s i)))
        (loop (+ i 1))
        (substring s i (string-length s)))))

(define (string-trim-right s)
  "SRFI-13 (string-trim-right s): S without its TRAILING whitespace."
  (let loop ((j (string-length s)))
    (if (and (> j 0) (char-whitespace? (string-ref s (- j 1))))
        (loop (- j 1))
        (substring s 0 j))))

(define (string-trim-both s)
  "SRFI-13 (string-trim-both s): S without whitespace at either end."
  (string-trim (string-trim-right s)))

; The modeler's file I/O (its 00-helpers), which the shell's save and
; visit use.
(define (slurp path)
  "The whole of the file at PATH as a string, or #f when there is none."
  (and (file-exists? path)
       (call-with-input-file path
         (lambda (in)
           (let loop ((acc '()))
             (let ((s (read-string 65536 in)))
               (if (eof-object? s) (apply string-append (reverse acc)) (loop (cons s acc)))))))))

(define (spit path text)
  "Write TEXT to the file at PATH, replacing what was there."
  (call-with-output-file path (lambda (out) (write-string text out))))
