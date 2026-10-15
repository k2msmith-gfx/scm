;;; (srfi 133) — Vector Library (R7RS-compatible).
;;;
;;; Written against the SRFI 133 specification rather than copied from its
;;; reference implementation, like the other libraries here. The reference
;;; implementation was consulted on exactly three points the prose leaves
;;; open, each noted at the procedure.
;;;
;;; **Sixteen of the forty-two exports are R7RS base procedures that scm's
;;; prelude already defines to SRFI 133's specification** -- including the
;;; optional `start`/`end` on `vector-copy`, `vector-copy!`, `vector-fill!`,
;;; `vector->list`, `string->vector` and `vector->string`, and the
;;; stop-at-the-shortest rule for `vector-map` and `vector-for-each`. They
;;; are *re-exported*, not redefined: `(export vector-ref)` with no local
;;; definition falls back to the global binding. Rewriting them here would
;;; add nothing and invite the two copies to drift.
;;;
;;; This is 160's counterpart for the case where the elements are not
;;; numbers: 160 is the same surface over packed numeric storage, and the
;;; two agree on argument order wherever they overlap.
(define-library (srfi 133)
  (export
    ;; constructors
    make-vector vector vector-unfold vector-unfold-right
    vector-copy vector-reverse-copy vector-append vector-concatenate
    vector-append-subvectors
    ;; predicates
    vector? vector-empty? vector=
    ;; selectors
    vector-ref vector-length
    ;; iteration
    vector-fold vector-fold-right vector-map vector-map! vector-for-each
    vector-count vector-cumulate
    ;; searching
    vector-index vector-index-right vector-skip vector-skip-right
    vector-binary-search vector-any vector-every vector-partition
    ;; mutators
    vector-set! vector-swap! vector-fill! vector-reverse!
    vector-copy! vector-reverse-copy!
    vector-unfold! vector-unfold-right!
    ;; conversion
    vector->list reverse-vector->list list->vector reverse-list->vector
    string->vector vector->string)
  (import (scheme base))
  (begin

    ;; ── the shapes every procedure here is built from ─────────────────
    ;;
    ;; The SRFI's optional `[start [end]]` tail, resolved against `v`.
    (define (%range v opt)
      (values (if (pair? opt) (car opt) 0)
              (if (and (pair? opt) (pair? (cdr opt)))
                  (cadr opt)
                  (vector-length v))))

    ;; The length of the shortest of `vs`, which is where every parallel
    ;; iteration in this SRFI stops.
    (define (%shortest vs)
      (let loop ((rest (cdr vs)) (n (vector-length (car vs))))
        (if (null? rest)
            n
            (loop (cdr rest) (min n (vector-length (car rest)))))))

    ;; The elements at `i` across `vs`, as the argument list to `apply`.
    (define (%refs vs i)
      (if (null? vs)
          '()
          (cons (vector-ref (car vs) i) (%refs (cdr vs) i))))

    ;; ── constructors ──────────────────────────────────────────────────
    (define (vector-unfold f len . seeds)
      (let ((v (make-vector len)))
        (apply vector-unfold! f v 0 len seeds)
        v))

    (define (vector-unfold-right f len . seeds)
      (let ((v (make-vector len)))
        (apply vector-unfold-right! f v 0 len seeds)
        v))

    ;; `f` returns the element followed by one new value per seed. **With
    ;; no seeds it returns the element alone** -- the specification's
    ;; `initial-seed ...` admits zero, and the reference implementation
    ;; has a `tabulate!` case that calls `(f i)` and takes one value,
    ;; which is the reading adopted here.
    (define (vector-unfold! f v start end . seeds)
      (cond
       ((null? seeds)
        (let loop ((i start))
          (if (< i end)
              (begin (vector-set! v i (f i))
                     (loop (+ i 1))))))
       ((null? (cdr seeds))
        (let loop ((i start) (seed (car seeds)))
          (if (< i end)
              (call-with-values
                (lambda () (f i seed))
                (lambda (elt next)
                  (vector-set! v i elt)
                  (loop (+ i 1) next))))))
       (else
        (let loop ((i start) (ss seeds))
          (if (< i end)
              (call-with-values
                (lambda () (apply f i ss))
                (lambda (elt . next)
                  (vector-set! v i elt)
                  (loop (+ i 1) next))))))))

    ;; Right to left: the first element `f` yields lands in `end - 1`.
    (define (vector-unfold-right! f v start end . seeds)
      (cond
       ((null? seeds)
        (let loop ((i (- end 1)))
          (if (>= i start)
              (begin (vector-set! v i (f i))
                     (loop (- i 1))))))
       ((null? (cdr seeds))
        (let loop ((i (- end 1)) (seed (car seeds)))
          (if (>= i start)
              (call-with-values
                (lambda () (f i seed))
                (lambda (elt next)
                  (vector-set! v i elt)
                  (loop (- i 1) next))))))
       (else
        (let loop ((i (- end 1)) (ss seeds))
          (if (>= i start)
              (call-with-values
                (lambda () (apply f i ss))
                (lambda (elt . next)
                  (vector-set! v i elt)
                  (loop (- i 1) next))))))))

    (define (vector-reverse-copy v . opt)
      (let-values (((start end) (%range v opt)))
        (let* ((n (- end start))
               (out (make-vector n)))
          (let loop ((i 0))
            (if (= i n)
                out
                (begin (vector-set! out i (vector-ref v (- end i 1)))
                       (loop (+ i 1))))))))

    (define (vector-concatenate vs)
      (apply vector-append vs))

    ;; `vector start end` triples, so the argument list is read three at a
    ;; time.
    (define (vector-append-subvectors . args)
      (let loop ((rest args) (parts '()))
        (if (null? rest)
            (apply vector-append (reverse parts))
            (loop (cdddr rest)
                  (cons (vector-copy (car rest) (cadr rest) (caddr rest))
                        parts)))))

    ;; ── predicates ────────────────────────────────────────────────────
    (define (vector-empty? v)
      (= 0 (vector-length v)))

    ;; Adjacent pairs, so N vectors cost N-1 comparisons; zero or one
    ;; vector is `#t` by the specification rather than by accident.
    (define (vector= elt=? . vs)
      (or (null? vs)
          (null? (cdr vs))
          (and (%vector=2 elt=? (car vs) (cadr vs))
               (apply vector= elt=? (cdr vs)))))

    (define (%vector=2 elt=? a b)
      (let ((n (vector-length a)))
        (and (= n (vector-length b))
             (let loop ((i 0))
               (cond ((= i n) #t)
                     ((elt=? (vector-ref a i) (vector-ref b i))
                      (loop (+ i 1)))
                     (else #f))))))

    ;; ── iteration ─────────────────────────────────────────────────────
    ;;
    ;; `kons` takes the **state first**: `(kons state elt ...)`. This is
    ;; the opposite of SRFI 1's `fold`, which takes `(f elem acc)`, and the
    ;; tests pin it with a non-commutative `kons` for that reason.
    (define (vector-fold kons knil v . more)
      (if (null? more)
          (let ((n (vector-length v)))
            (let loop ((i 0) (acc knil))
              (if (= i n)
                  acc
                  (loop (+ i 1) (kons acc (vector-ref v i))))))
          (let* ((vs (cons v more))
                 (n (%shortest vs)))
            (let loop ((i 0) (acc knil))
              (if (= i n)
                  acc
                  (loop (+ i 1) (apply kons acc (%refs vs i))))))))

    ;; Right to left, and the state is still the first argument.
    (define (vector-fold-right kons knil v . more)
      (if (null? more)
          (let loop ((i (- (vector-length v) 1)) (acc knil))
            (if (negative? i)
                acc
                (loop (- i 1) (kons acc (vector-ref v i)))))
          (let* ((vs (cons v more))
                 (n (%shortest vs)))
            (let loop ((i (- n 1)) (acc knil))
              (if (negative? i)
                  acc
                  (loop (- i 1) (apply kons acc (%refs vs i))))))))

    ;; Writes into the first vector; the result is unspecified.
    (define (vector-map! f v . more)
      (if (null? more)
          (let ((n (vector-length v)))
            (let loop ((i 0))
              (if (< i n)
                  (begin (vector-set! v i (f (vector-ref v i)))
                         (loop (+ i 1))))))
          (let* ((vs (cons v more))
                 (n (%shortest vs)))
            (let loop ((i 0))
              (if (< i n)
                  (begin (vector-set! v i (apply f (%refs vs i)))
                         (loop (+ i 1))))))))

    (define (vector-count pred? v . more)
      (if (null? more)
          (let ((n (vector-length v)))
            (let loop ((i 0) (c 0))
              (cond ((= i n) c)
                    ((pred? (vector-ref v i)) (loop (+ i 1) (+ c 1)))
                    (else (loop (+ i 1) c)))))
          (let* ((vs (cons v more))
                 (n (%shortest vs)))
            (let loop ((i 0) (c 0))
              (cond ((= i n) c)
                    ((apply pred? (%refs vs i)) (loop (+ i 1) (+ c 1)))
                    (else (loop (+ i 1) c)))))))

    ;; Element `i` is `(f result[i-1] vec[i])`, with `knil` standing in
    ;; for `result[-1]`; the result is as long as the source.
    (define (vector-cumulate f knil v)
      (let* ((n (vector-length v))
             (out (make-vector n)))
        (let loop ((i 0) (acc knil))
          (if (= i n)
              out
              (let ((r (f acc (vector-ref v i))))
                (vector-set! out i r)
                (loop (+ i 1) r))))))

    ;; ── searching ─────────────────────────────────────────────────────
    ;;
    ;; `skip` is `index` with the predicate negated, which is how the SRFI
    ;; defines it -- so there is one search in each direction here, not
    ;; four.
    (define (%index-left pred vs)
      (let ((n (%shortest vs)))
        (let loop ((i 0))
          (cond ((= i n) #f)
                ((apply pred (%refs vs i)) i)
                (else (loop (+ i 1)))))))

    (define (%index-right pred vs)
      (let loop ((i (- (%shortest vs) 1)))
        (cond ((negative? i) #f)
              ((apply pred (%refs vs i)) i)
              (else (loop (- i 1))))))

    (define (vector-index pred? v . more)
      (%index-left pred? (cons v more)))

    (define (vector-index-right pred? v . more)
      (%index-right pred? (cons v more)))

    (define (vector-skip pred? v . more)
      (%index-left (lambda elts (not (apply pred? elts))) (cons v more)))

    (define (vector-skip-right pred? v . more)
      (%index-right (lambda elts (not (apply pred? elts))) (cons v more)))

    ;; `cmp` is applied as `(cmp element value)` -- element first -- and
    ;; its sign says where the element sits relative to the value.
    ;; `start`/`end` are the reference implementation's optional tail; the
    ;; signature in the specification's body omits them, so accepting them
    ;; is a superset.
    (define (vector-binary-search v value cmp . opt)
      (let-values (((start end) (%range v opt)))
        (let loop ((lo start) (hi end))
          (if (>= lo hi)
              #f
              (let* ((mid (quotient (+ lo hi) 2))
                     (c (cmp (vector-ref v mid) value)))
                (cond ((= c 0) mid)
                      ((negative? c) (loop (+ mid 1) hi))
                      (else (loop lo mid))))))))

    ;; **Empty vectors: `vector-any` is `#f` and `vector-every` is `#t`.**
    ;; The prose says neither; the reference implementation's loops begin
    ;; `(and (not (= i len)) …)` and `(or (= i len) …)` respectively, which
    ;; is where these come from. `vector-every` returns the value of the
    ;; *last* call, not `#t`.
    (define (vector-any pred? v . more)
      (let* ((vs (cons v more))
             (n (%shortest vs)))
        (let loop ((i 0))
          (and (< i n)
               (or (apply pred? (%refs vs i))
                   (loop (+ i 1)))))))

    (define (vector-every pred? v . more)
      (let* ((vs (cons v more))
             (n (%shortest vs)))
        (let loop ((i 0) (last #t))
          (if (= i n)
              last
              (let ((r (apply pred? (%refs vs i))))
                (and r (loop (+ i 1) r)))))))

    ;; Satisfying elements first, each group in its original order, plus
    ;; the count that satisfied. The specification calls the second value
    ;; the number satisfying and the reference implementation's comment
    ;; calls it the index of the leftmost element that does not -- the
    ;; same number, since the satisfying ones come first.
    ;;
    ;; One pass, so `pred?` is applied exactly once per element; the
    ;; reference implementation applies it twice, once through
    ;; `vector-count`, which a predicate with a side effect would see.
    (define (vector-partition pred? v)
      (let ((n (vector-length v)))
        (let loop ((i 0) (yes '()) (no '()) (cnt 0))
          (if (= i n)
              (let ((out (make-vector n)))
                (let fill ((j 0) (xs (append (reverse yes) (reverse no))))
                  (if (null? xs)
                      (values out cnt)
                      (begin (vector-set! out j (car xs))
                             (fill (+ j 1) (cdr xs))))))
              (let ((x (vector-ref v i)))
                (if (pred? x)
                    (loop (+ i 1) (cons x yes) no (+ cnt 1))
                    (loop (+ i 1) yes (cons x no) cnt)))))))

    ;; ── mutators ──────────────────────────────────────────────────────
    (define (vector-swap! v i j)
      (let ((x (vector-ref v i)))
        (vector-set! v i (vector-ref v j))
        (vector-set! v j x)))

    (define (vector-reverse! v . opt)
      (let-values (((start end) (%range v opt)))
        (let loop ((i start) (j (- end 1)))
          (if (< i j)
              (begin (vector-swap! v i j)
                     (loop (+ i 1) (- j 1)))))))

    ;; From a snapshot of the source range, so `to` and `from` may be the
    ;; same vector with overlapping ranges -- which is what reversing a
    ;; slice in place is, and the SRFI permits it.
    (define (vector-reverse-copy! to at from . opt)
      (let-values (((start end) (%range from opt)))
        (let* ((src (vector-copy from start end))
               (n (vector-length src)))
          (let loop ((i 0))
            (if (< i n)
                (begin (vector-set! to (+ at i) (vector-ref src (- n i 1)))
                       (loop (+ i 1))))))))

    ;; ── conversion ────────────────────────────────────────────────────
    (define (reverse-vector->list v . opt)
      (let-values (((start end) (%range v opt)))
        (let loop ((i start) (acc '()))
          (if (= i end)
              acc
              (loop (+ i 1) (cons (vector-ref v i) acc))))))

    (define (reverse-list->vector lst)
      (let* ((n (length lst))
             (out (make-vector n)))
        (let loop ((i (- n 1)) (rest lst))
          (if (null? rest)
              out
              (begin (vector-set! out i (car rest))
                     (loop (- i 1) (cdr rest)))))))))
