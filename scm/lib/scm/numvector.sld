;;; (scm numvector) — the SRFI 160 operations, written once.
;;;
;;; SRFI 160 specifies about sixty operations for *each* of ten element
;;; types: `f64vector-map`, `s32vector-map`, and so on. Written out that
;;; would be six hundred procedures differing only in a name.
;;;
;;; They do not have to be. A numeric vector carries its own element type,
;;; which `numvector-tag` reports, so a result vector can be made the same
;;; type as its input without anyone naming the type -- and then the
;;; operation is written once here and the per-type libraries are aliases
;;; onto it. `(srfi 160 f64)` costs a name, not an implementation.
;;;
;;; The few operations that genuinely need the type are the ones with no
;;; input vector to ask: the constructors, the element predicate and the
;;; comparator. Those take the tag as their first argument, and each
;;; per-type library supplies it.
(define-library (scm numvector)
  (export numvector-element?
          numvector-empty? numvector=
          numvector-unfold numvector-unfold-right
          numvector-reverse-copy numvector-append numvector-concatenate
          numvector-append-subvectors
          numvector-take numvector-take-right
          numvector-drop numvector-drop-right
          numvector-segment
          numvector-fold numvector-fold-right
          numvector-map numvector-map! numvector-for-each
          numvector-count numvector-cumulate
          numvector-take-while numvector-take-while-right
          numvector-drop-while numvector-drop-while-right
          numvector-index numvector-index-right
          numvector-skip numvector-skip-right
          numvector-any numvector-every
          numvector-partition numvector-filter numvector-remove
          numvector-swap! numvector-reverse!
          numvector-reverse-copy! numvector-unfold! numvector-unfold-right!
          reverse-numvector->list reverse-list->numvector
          numvector->vector vector->numvector
          make-numvector-generator make-numvector-comparator
          write-numvector)
  (import (scheme base) (scheme write) (srfi 128))
  (begin

    ;; A new vector of the same element type as `v`.
    (define (%new v n) (make-numvector (numvector-tag v) n))

    (define (%len v) (numvector-length v))

    ;; Several vectors stop at the shortest, as `map` does on lists.
    (define (%shortest vs) (apply min (map numvector-length vs)))

    (define (%refs vs i) (map (lambda (v) (numvector-ref v i)) vs))

    (define (%start opt) (if (pair? opt) (car opt) 0))
    (define (%end v opt)
      (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) (%len v)))

    ;; ── the element types ────────────────────────────────────────────
    ;; `@?` asks whether a value can be stored without loss. The integer
    ;; types are a range check; the float types take any real. This is the
    ;; one place that knows what the ten tags mean.
    (define (numvector-element? tag x)
      (case tag
        ((f32 f64) (and (number? x) (real? x)))
        ((c64 c128) (number? x))
        (else
         (and (number? x) (exact? x) (integer? x)
              (let ((r (case tag
                         ((s8) '(-128 . 127))
                         ((u8) '(0 . 255))
                         ((s16) '(-32768 . 32767))
                         ((u16) '(0 . 65535))
                         ((s32) '(-2147483648 . 2147483647))
                         ((u32) '(0 . 4294967295))
                         ((s64) '(-9223372036854775808 . 9223372036854775807))
                         ;; u64's top half does not fit a fixnum, so the
                         ;; upper bound is the largest value that does
                         ;; rather than 2^64-1 (plan §2.3: no bignums).
                         ((u64) '(0 . 9223372036854775807))
                         (else #f))))
                (and r (>= x (car r)) (<= x (cdr r))))))))

    (define (numvector-empty? v) (= 0 (%len v)))

    ;; Elementwise `=`, so the comparison is numeric and two vectors of
    ;; different element types can be equal. Variadic, and vacuously true
    ;; of fewer than two vectors.
    (define (numvector= . vs)
      (let loop ((vs vs))
        (cond ((or (null? vs) (null? (cdr vs))) #t)
              ((%two= (car vs) (cadr vs)) (loop (cdr vs)))
              (else #f))))

    (define (%two= a b)
      (and (= (%len a) (%len b))
           (let loop ((i 0))
             (cond ((= i (%len a)) #t)
                   ((= (numvector-ref a i) (numvector-ref b i)) (loop (+ i 1)))
                   (else #f)))))

    ;; ── constructors ────────────────────────────────────────────────
    ;; `f` is applied to the index and the seed and returns two values:
    ;; the element, and the seed for the next index.
    (define (numvector-unfold tag f len seed)
      (let ((out (make-numvector tag len)))
        (numvector-unfold! f out 0 len seed)
        out))

    (define (numvector-unfold-right tag f len seed)
      (let ((out (make-numvector tag len)))
        (numvector-unfold-right! f out 0 len seed)
        out))

    (define (numvector-unfold! f v start end seed)
      (let loop ((i start) (seed seed))
        (when (< i end)
          (call-with-values (lambda () (f i seed))
            (lambda (x next)
              (numvector-set! v i x)
              (loop (+ i 1) next))))))

    ;; Right to left, so the last index is filled from the first seed.
    (define (numvector-unfold-right! f v start end seed)
      (let loop ((i (- end 1)) (seed seed))
        (when (>= i start)
          (call-with-values (lambda () (f i seed))
            (lambda (x next)
              (numvector-set! v i x)
              (loop (- i 1) next))))))

    (define (numvector-reverse-copy v . opt)
      (let* ((start (%start opt)) (end (%end v opt))
             (out (%new v (- end start))))
        (let loop ((i start) (j (- end start 1)))
          (if (< i end)
              (begin (numvector-set! out j (numvector-ref v i))
                     (loop (+ i 1) (- j 1)))
              out))))

    ;; The tag is given because the list may be empty, and then there is
    ;; no vector to ask.
    (define (numvector-append tag vs)
      (let* ((n (apply + (map numvector-length vs)))
             (out (make-numvector tag n)))
        (let loop ((vs vs) (at 0))
          (if (null? vs)
              out
              (begin (numvector-copy! out at (car vs))
                     (loop (cdr vs) (+ at (%len (car vs)))))))))

    (define (numvector-concatenate tag vs) (numvector-append tag vs))

    ;; `args` is a flat list of vector, start, end, repeated.
    (define (numvector-append-subvectors tag args)
      (let loop ((as args) (pieces '()))
        (if (null? as)
            (numvector-append tag (reverse pieces))
            (loop (cdddr as)
                  (cons (numvector-copy (car as) (cadr as) (caddr as))
                        pieces)))))

    ;; ── prefixes and suffixes ───────────────────────────────────────
    (define (numvector-take v n) (numvector-copy v 0 n))
    (define (numvector-take-right v n) (numvector-copy v (- (%len v) n) (%len v)))
    (define (numvector-drop v n) (numvector-copy v n (%len v)))
    (define (numvector-drop-right v n) (numvector-copy v 0 (- (%len v) n)))

    ;; Consecutive pieces of length `n`, the last one short if the length
    ;; does not divide.
    (define (numvector-segment v n)
      (let loop ((i 0) (acc '()))
        (if (>= i (%len v))
            (reverse acc)
            (loop (+ i n)
                  (cons (numvector-copy v i (min (+ i n) (%len v))) acc)))))

    ;; ── iteration ───────────────────────────────────────────────────
    ;; `(kons state elt ...)`, left to right.
    (define (numvector-fold kons knil v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0) (acc knil))
          (if (= i n) acc (loop (+ i 1) (apply kons acc (%refs vs i)))))))

    (define (numvector-fold-right kons knil v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i (- n 1)) (acc knil))
          (if (< i 0) acc (loop (- i 1) (apply kons acc (%refs vs i)))))))

    (define (numvector-map f v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)) (out (%new v n)))
        (let loop ((i 0))
          (if (= i n)
              out
              (begin (numvector-set! out i (apply f (%refs vs i)))
                     (loop (+ i 1)))))))

    ;; In place into the first vector, which is why it stops at the
    ;; shortest rather than complaining.
    (define (numvector-map! f v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0))
          (if (= i n)
              (if #f #f)
              (begin (numvector-set! v i (apply f (%refs vs i)))
                     (loop (+ i 1)))))))

    (define (numvector-for-each f v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0))
          (if (= i n)
              (if #f #f)
              (begin (apply f (%refs vs i)) (loop (+ i 1)))))))

    (define (numvector-count pred v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0) (k 0))
          (cond ((= i n) k)
                ((apply pred (%refs vs i)) (loop (+ i 1) (+ k 1)))
                (else (loop (+ i 1) k))))))

    ;; `out[i] = (f out[i-1] v[i])`, with `knil` standing in for out[-1].
    (define (numvector-cumulate f knil v)
      (let* ((n (%len v)) (out (%new v n)))
        (let loop ((i 0) (acc knil))
          (if (= i n)
              out
              (let ((next (f acc (numvector-ref v i))))
                (numvector-set! out i next)
                (loop (+ i 1) next))))))

    ;; ── searching ───────────────────────────────────────────────────
    ;; The longest prefix whose elements all satisfy `pred`, and the three
    ;; other corners of the same idea. `-right` works on the *suffix*: it
    ;; takes or drops from the far end.
    (define (numvector-take-while pred v)
      (numvector-copy v 0 (%fail-from pred v)))

    (define (numvector-drop-while pred v)
      (numvector-copy v (%fail-from pred v) (%len v)))

    (define (numvector-take-while-right pred v)
      (numvector-copy v (%fail-upto pred v) (%len v)))

    (define (numvector-drop-while-right pred v)
      (numvector-copy v 0 (%fail-upto pred v)))

    ;; The first index that fails, or the length.
    (define (%fail-from pred v)
      (let loop ((i 0))
        (cond ((= i (%len v)) i)
              ((pred (numvector-ref v i)) (loop (+ i 1)))
              (else i))))

    ;; One past the last index that fails, or 0.
    (define (%fail-upto pred v)
      (let loop ((i (- (%len v) 1)))
        (cond ((< i 0) 0)
              ((pred (numvector-ref v i)) (loop (- i 1)))
              (else (+ i 1)))))

    (define (numvector-index pred v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0))
          (cond ((= i n) #f)
                ((apply pred (%refs vs i)) i)
                (else (loop (+ i 1)))))))

    (define (numvector-index-right pred v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i (- n 1)))
          (cond ((< i 0) #f)
                ((apply pred (%refs vs i)) i)
                (else (loop (- i 1)))))))

    ;; `skip` is `index` of the complement, which is how the SRFI
    ;; defines it and why there is no separate loop.
    (define (numvector-skip pred v . rest)
      (apply numvector-index (%not pred) v rest))

    (define (numvector-skip-right pred v . rest)
      (apply numvector-index-right (%not pred) v rest))

    (define (%not pred) (lambda args (not (apply pred args))))

    ;; The first true result, not the index -- so a predicate that
    ;; returns something useful is worth calling through this.
    (define (numvector-any pred v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0))
          (if (= i n)
              #f
              (or (apply pred (%refs vs i)) (loop (+ i 1)))))))

    ;; The last result when every element satisfies, `#t` when empty.
    (define (numvector-every pred v . rest)
      (let* ((vs (cons v rest)) (n (%shortest vs)))
        (let loop ((i 0) (last #t))
          (if (= i n)
              last
              (let ((r (apply pred (%refs vs i))))
                (and r (loop (+ i 1) r)))))))

    ;; Two values: the satisfying elements followed by the rest, and how
    ;; many satisfied -- which is where the one vector divides.
    (define (numvector-partition pred v)
      (let* ((n (%len v)) (out (%new v n)))
        (let loop ((i 0) (yes 0) (no '()))
          (cond ((= i n)
                 (let fill ((xs (reverse no)) (j yes))
                   (if (null? xs)
                       (values out yes)
                       (begin (numvector-set! out j (car xs))
                              (fill (cdr xs) (+ j 1))))))
                ((pred (numvector-ref v i))
                 (numvector-set! out yes (numvector-ref v i))
                 (loop (+ i 1) (+ yes 1) no))
                (else (loop (+ i 1) yes (cons (numvector-ref v i) no)))))))

    (define (numvector-filter pred v) (%keep pred v #t))
    (define (numvector-remove pred v) (%keep pred v #f))

    (define (%keep pred v want)
      (let* ((xs (let loop ((i (- (%len v) 1)) (acc '()))
                   (if (< i 0)
                       acc
                       (let ((x (numvector-ref v i)))
                         (loop (- i 1)
                               (if (eq? want (if (pred x) #t #f))
                                   (cons x acc)
                                   acc))))))
             (out (%new v (length xs))))
        (let fill ((xs xs) (i 0))
          (if (null? xs)
              out
              (begin (numvector-set! out i (car xs)) (fill (cdr xs) (+ i 1)))))))

    ;; ── mutation ────────────────────────────────────────────────────
    (define (numvector-swap! v i j)
      (let ((x (numvector-ref v i)))
        (numvector-set! v i (numvector-ref v j))
        (numvector-set! v j x)))

    (define (numvector-reverse! v . opt)
      (let loop ((i (%start opt)) (j (- (%end v opt) 1)))
        (when (< i j)
          (numvector-swap! v i j)
          (loop (+ i 1) (- j 1)))))

    ;; Copies `from` into `to` backwards. A fresh copy of the source is
    ;; taken first, so that a target overlapping its own source -- the
    ;; usual reason to call this -- does not read what it has written.
    (define (numvector-reverse-copy! to at from . opt)
      (let* ((start (%start opt)) (end (%end from opt))
             (src (numvector-copy from start end)))
        (let loop ((i 0))
          (when (< i (%len src))
            (numvector-set! to (+ at i) (numvector-ref src (- (%len src) i 1)))
            (loop (+ i 1))))))

    ;; ── conversion ──────────────────────────────────────────────────
    (define (reverse-numvector->list v . opt)
      (reverse (numvector->list v (%start opt) (%end v opt))))

    (define (reverse-list->numvector tag lst)
      (list->numvector tag (reverse lst)))

    (define (numvector->vector v . opt)
      (list->vector (numvector->list v (%start opt) (%end v opt))))

    (define (vector->numvector tag vec . opt)
      (let* ((start (%start opt))
             (end (if (and (pair? opt) (pair? (cdr opt)))
                      (cadr opt)
                      (vector-length vec))))
        (list->numvector tag (vector->list vec start end))))

    ;; ── the rest ────────────────────────────────────────────────────
    ;; A SRFI 158 generator: a thunk giving each element in turn and then
    ;; the end-of-file object, which is that SRFI's end marker.
    (define (make-numvector-generator v)
      (let ((i 0))
        (lambda ()
          (if (>= i (%len v))
              (eof-object)
              (let ((x (numvector-ref v i)))
                (set! i (+ i 1))
                x)))))

    ;; Lexicographic by element, then by length, which is the order a
    ;; sorted container of vectors wants. `equal-hash` is the hash
    ;; because within one element type it agrees with `numvector=`.
    (define (make-numvector-comparator tag)
      (make-comparator
       (lambda (x) (numvector? x tag))
       numvector=
       (lambda (a b) (eq? 'less (%compare a b)))
       equal-hash))

    (define (%compare a b)
      (let ((n (min (%len a) (%len b))))
        (let loop ((i 0))
          (cond ((= i n)
                 (cond ((< (%len a) (%len b)) 'less)
                       ((> (%len a) (%len b)) 'greater)
                       (else 'equal)))
                ((< (numvector-ref a i) (numvector-ref b i)) 'less)
                ((> (numvector-ref a i) (numvector-ref b i)) 'greater)
                (else (loop (+ i 1)))))))

    ;; The external representation the reader accepts: `#f64(1.0 2.0)`.
    (define (write-numvector v . opt)
      (let ((port (if (pair? opt) (car opt) (current-output-port))))
        (write-string "#" port)
        (write-string (symbol->string (numvector-tag v)) port)
        (write-string "(" port)
        (let loop ((i 0))
          (when (< i (%len v))
            (when (> i 0) (write-string " " port))
            (write (numvector-ref v i) port)
            (loop (+ i 1))))
        (write-string ")" port)))))
