;;; (srfi 158) — Generators and accumulators.
;;;
;;; A generator is a thunk that yields its next value and then the
;;; end-of-file object for ever after. That is the whole protocol, which is
;;; why `(srfi 194)`'s samplers and `(srfi 160)`'s
;;; `make-f64vector-generator` were already generators before this library
;;; existed -- they had nothing to be consumed by until now.
;;;
;;; For procedural generation the point is that a pipeline costs no
;;; intermediate lists. Scattering a million points, filtering them against
;;; a surface and taking the first few hundred that pass should not build a
;;; million-element list to then discard it, and with generators it does
;;; not: `gfilter` and `gtake` pull one value at a time.
;;;
;;; ## `make-coroutine-generator` is eager here, and capped
;;;
;;; The SRFI's version suspends the producer at each `yield` and resumes it
;;; on the next call. That needs a **re-entrant** continuation, and plan
;;; §3.1 makes continuations escape-only, so it cannot be done.
;;;
;;; What is here instead runs the producer to completion when the generator
;;; is *made*, collecting what it yields, and then hands the values out.
;;; Two consequences, and they are real:
;;;
;;;   * **A producer that never returns cannot work.** Rather than hanging,
;;;     it raises once it has yielded `%coroutine-limit` values. A hang
;;;     with no message is the worst failure to debug, so it is converted
;;;     into one that says what happened.
;;;   * **Side effects run at construction, not at consumption.** A
;;;     producer that prints as it yields prints everything up front.
;;;
;;; `make-for-each-generator` is built on it and is unaffected, because a
;;; `for-each` over a collection always terminates.
(define-library (srfi 158)
  (export generator circular-generator make-iota-generator
          make-range-generator make-coroutine-generator
          list->generator vector->generator reverse-vector->generator
          string->generator bytevector->generator
          make-for-each-generator make-unfold-generator
          gcons* gappend gflatten ggroup gmerge gmap gcombine
          gfilter gremove gstate-filter
          gtake gdrop gtake-while gdrop-while
          gdelete gdelete-neighbor-dups gindex gselect
          generator->list generator->reverse-list generator->vector
          generator->vector! generator->string
          generator-fold generator-for-each generator-map->list
          generator-find generator-count generator-any generator-every
          generator-unfold
          make-accumulator count-accumulator
          list-accumulator reverse-list-accumulator
          vector-accumulator reverse-vector-accumulator vector-accumulator!
          string-accumulator bytevector-accumulator bytevector-accumulator!
          sum-accumulator product-accumulator
          %coroutine-limit)
  (import (scheme base))
  (begin

    ;; ── constructors ─────────────────────────────────────────────────
    (define (generator . args) (list->generator args))

    (define (list->generator lst)
      (lambda ()
        (if (pair? lst)
            (let ((x (car lst))) (set! lst (cdr lst)) x)
            (eof-object))))

    (define (circular-generator . args)
      (when (null? args)
        (error "circular-generator: needs at least one value"))
      (let ((rest args))
        (lambda ()
          (when (null? rest) (set! rest args))
          (let ((x (car rest))) (set! rest (cdr rest)) x))))

    (define (make-iota-generator count . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (step (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) 1))
             (i 0))
        (lambda ()
          (if (>= i count)
              (eof-object)
              (let ((x (+ start (* i step)))) (set! i (+ i 1)) x)))))

    ;; With no end this is unbounded, which is legal and is why `gtake`
    ;; and `gtake-while` exist.
    (define (make-range-generator start . opt)
      (let* ((end (if (pair? opt) (car opt) #f))
             (step (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) 1))
             (x start))
        (lambda ()
          (if (and end (if (> step 0) (>= x end) (<= x end)))
              (eof-object)
              (let ((v x)) (set! x (+ x step)) v)))))

    ;; How many values an eager coroutine will take before deciding the
    ;; producer does not terminate. Exported so a program that genuinely
    ;; needs more can say so.
    (define %coroutine-limit (make-parameter 1000000))

    (define (make-coroutine-generator proc)
      (let ((acc '()) (n 0) (limit (%coroutine-limit)))
        (proc (lambda (v)
                (set! n (+ n 1))
                (when (> n limit)
                  (error (string-append
                          "make-coroutine-generator: the producer yielded more than "
                          (number->string limit)
                          " values. This implementation runs the producer eagerly,"
                          " because continuations are escape-only (plan §3.1), so a"
                          " producer that never returns cannot be suspended."
                          " Use make-unfold-generator or make-range-generator for an"
                          " unbounded sequence, or raise %coroutine-limit.")))
                (set! acc (cons v acc))))
        (list->generator (reverse acc))))

    (define (make-for-each-generator for-each obj)
      (make-coroutine-generator (lambda (yield) (for-each yield obj))))

    (define (make-unfold-generator stop? mapper successor seed)
      (let ((state seed) (done #f))
        (lambda ()
          (if (or done (stop? state))
              (begin (set! done #t) (eof-object))
              (let ((x (mapper state)))
                (set! state (successor state))
                x)))))

    (define (vector->generator vec . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (end (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) (vector-length vec)))
             (i start))
        (lambda ()
          (if (>= i end)
              (eof-object)
              (let ((x (vector-ref vec i))) (set! i (+ i 1)) x)))))

    (define (reverse-vector->generator vec . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (end (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) (vector-length vec)))
             (i end))
        (lambda ()
          (if (<= i start)
              (eof-object)
              (begin (set! i (- i 1)) (vector-ref vec i))))))

    (define (string->generator str . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (end (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) (string-length str)))
             (i start))
        (lambda ()
          (if (>= i end)
              (eof-object)
              (let ((x (string-ref str i))) (set! i (+ i 1)) x)))))

    (define (bytevector->generator bv . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (end (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) (bytevector-length bv)))
             (i start))
        (lambda ()
          (if (>= i end)
              (eof-object)
              (let ((x (bytevector-u8-ref bv i))) (set! i (+ i 1)) x)))))

    ;; ── operations ───────────────────────────────────────────────────
    (define (gcons* . args)
      (let loop ((args args))
        (cond ((null? args) (error "gcons*: needs a generator"))
              ((null? (cdr args)) (car args))
              (else (let ((head (car args)) (rest (loop (cdr args))) (sent #f))
                      (lambda ()
                        (if sent (rest) (begin (set! sent #t) head))))))))

    (define (gappend . gens)
      (lambda ()
        (let loop ()
          (if (null? gens)
              (eof-object)
              (let ((x ((car gens))))
                (if (eof-object? x)
                    (begin (set! gens (cdr gens)) (loop))
                    x))))))

    ;; The values are lists, and their elements come out one at a time.
    (define (gflatten gen)
      (let ((pending '()))
        (lambda ()
          (let loop ()
            (if (pair? pending)
                (let ((x (car pending))) (set! pending (cdr pending)) x)
                (let ((lst (gen)))
                  (if (eof-object? lst)
                      (eof-object)
                      (begin (set! pending lst) (loop)))))))))

    ;; Lists of `k`. Without padding the last list is short; with it, the
    ;; last list is padded to `k`, so every list has the same length.
    (define (ggroup gen k . opt)
      (let ((padding (if (pair? opt) (car opt) #f))
            (padded? (pair? opt)))
        (lambda ()
          (let loop ((i 0) (acc '()))
            (if (= i k)
                (reverse acc)
                (let ((x (gen)))
                  (cond ((not (eof-object? x)) (loop (+ i 1) (cons x acc)))
                        ((null? acc) (eof-object))
                        (padded?
                         (let pad ((i i) (acc acc))
                           (if (= i k) (reverse acc) (pad (+ i 1) (cons padding acc)))))
                        (else (reverse acc)))))))))

    ;; Merges sorted inputs. One value is held back from each generator, so
    ;; the smallest can be chosen without reading ahead twice.
    (define (gmerge less . gens)
      (let ((heads (map (lambda (g) (g)) gens))
            (gs gens))
        (lambda ()
          (let loop ()
            ;; drop any generator that has run out
            (let prune ((h heads) (g gs) (h* '()) (g* '()))
              (cond ((null? h)
                     (set! heads (reverse h*))
                     (set! gs (reverse g*)))
                    ((eof-object? (car h)) (prune (cdr h) (cdr g) h* g*))
                    (else (prune (cdr h) (cdr g) (cons (car h) h*) (cons (car g) g*)))))
            (if (null? heads)
                (eof-object)
                ;; pick the smallest head, then refill just that one
                (let pick ((i 0) (best 0) (h (cdr heads)) (bv (car heads)))
                  (if (null? h)
                      (let ((v bv))
                        (set! heads (%replace heads best ((list-ref gs best))))
                        v)
                      (if (less (car h) bv)
                          (pick (+ i 1) (+ i 1) (cdr h) (car h))
                          (pick (+ i 1) best (cdr h) bv)))))))))

    (define (%replace lst i v)
      (if (= i 0) (cons v (cdr lst)) (cons (car lst) (%replace (cdr lst) (- i 1) v))))

    ;; Several generators stop at the first exhausted one, as `map` on
    ;; lists stops at the shortest.
    (define (gmap proc . gens)
      (lambda ()
        (let ((vs (map (lambda (g) (g)) gens)))
          (if (%any-eof? vs) (eof-object) (apply proc vs)))))

    (define (%any-eof? vs)
      (cond ((null? vs) #f) ((eof-object? (car vs)) #t) (else (%any-eof? (cdr vs)))))

    ;; `proc` returns two values: what to yield, and the next seed.
    (define (gcombine proc seed . gens)
      (let ((state seed))
        (lambda ()
          (let ((vs (map (lambda (g) (g)) gens)))
            (if (%any-eof? vs)
                (eof-object)
                (call-with-values (lambda () (apply proc (append vs (list state))))
                  (lambda (value next) (set! state next) value)))))))

    (define (gfilter pred gen)
      (lambda ()
        (let loop ()
          (let ((x (gen)))
            (cond ((eof-object? x) x) ((pred x) x) (else (loop)))))))

    (define (gremove pred gen) (gfilter (lambda (x) (not (pred x))) gen))

    ;; `proc` returns two values: whether to keep the item, and the next
    ;; state. So the decision may depend on everything seen so far.
    (define (gstate-filter proc seed gen)
      (let ((state seed))
        (lambda ()
          (let loop ()
            (let ((x (gen)))
              (if (eof-object? x)
                  x
                  (call-with-values (lambda () (proc x state))
                    (lambda (keep next)
                      (set! state next)
                      (if keep x (loop))))))))))

    ;; At most `k`, or exactly `k` when padding is given.
    (define (gtake gen k . opt)
      (let ((padding (if (pair? opt) (car opt) #f))
            (padded? (pair? opt))
            (i 0))
        (lambda ()
          (if (>= i k)
              (eof-object)
              (let ((x (gen)))
                (set! i (+ i 1))
                (if (eof-object? x) (if padded? padding (eof-object)) x))))))

    (define (gdrop gen k)
      (let ((dropped #f))
        (lambda ()
          (unless dropped
            (set! dropped #t)
            (let loop ((i 0))
              (when (< i k)
                (if (eof-object? (gen)) #f (loop (+ i 1))))))
          (gen))))

    (define (gtake-while pred gen)
      (let ((done #f))
        (lambda ()
          (if done
              (eof-object)
              (let ((x (gen)))
                (cond ((eof-object? x) (set! done #t) x)
                      ((pred x) x)
                      (else (set! done #t) (eof-object))))))))

    (define (gdrop-while pred gen)
      (let ((dropping #t))
        (lambda ()
          (if (not dropping)
              (gen)
              (let loop ()
                (let ((x (gen)))
                  (cond ((eof-object? x) (set! dropping #f) x)
                        ((pred x) (loop))
                        (else (set! dropping #f) x))))))))

    (define (gdelete item gen . opt)
      (let ((same? (if (pair? opt) (car opt) equal?)))
        (gfilter (lambda (x) (not (same? item x))) gen)))

    ;; Only *consecutive* duplicates, so this needs no memory of anything
    ;; but the last value emitted.
    (define (gdelete-neighbor-dups gen . opt)
      (let ((same? (if (pair? opt) (car opt) equal?))
            (have #f)
            (last #f))
        (lambda ()
          (let loop ()
            (let ((x (gen)))
              (cond ((eof-object? x) x)
                    ((and have (same? last x)) (loop))
                    (else (set! have #t) (set! last x) x)))))))

    ;; The indices must increase, which is what lets this make one pass
    ;; rather than remembering the values it has seen.
    (define (gindex value-gen index-gen)
      (let ((pos 0))
        (lambda ()
          (let ((want (index-gen)))
            (if (eof-object? want)
                (eof-object)
                (let loop ()
                  (let ((v (value-gen)))
                    (cond ((eof-object? v) v)
                          ((= pos want) (set! pos (+ pos 1)) v)
                          (else (set! pos (+ pos 1)) (loop))))))))))

    (define (gselect value-gen truth-gen)
      (lambda ()
        (let loop ()
          (let ((t (truth-gen)))
            (if (eof-object? t)
                (eof-object)
                (let ((v (value-gen)))
                  (cond ((eof-object? v) v)
                        (t v)
                        (else (loop)))))))))

    ;; ── consumers ────────────────────────────────────────────────────
    (define (generator->reverse-list gen . opt)
      (let ((k (if (pair? opt) (car opt) #f)))
        (let loop ((i 0) (acc '()))
          (if (and k (>= i k))
              acc
              (let ((x (gen)))
                (if (eof-object? x) acc (loop (+ i 1) (cons x acc))))))))

    (define (generator->list gen . opt)
      (reverse (apply generator->reverse-list gen opt)))

    (define (generator->vector gen . opt)
      (list->vector (apply generator->list gen opt)))

    ;; Returns how many it wrote, which is the only way to tell a full
    ;; vector from an exhausted generator.
    (define (generator->vector! vec at gen)
      (let loop ((i at) (n 0))
        (if (>= i (vector-length vec))
            n
            (let ((x (gen)))
              (if (eof-object? x)
                  n
                  (begin (vector-set! vec i x) (loop (+ i 1) (+ n 1))))))))

    (define (generator->string gen . opt)
      (list->string (apply generator->list gen opt)))

    ;; `(proc value … accumulated)`, the accumulator last.
    (define (generator-fold proc seed . gens)
      (let loop ((acc seed))
        (let ((vs (map (lambda (g) (g)) gens)))
          (if (%any-eof? vs)
              acc
              (loop (apply proc (append vs (list acc))))))))

    (define (generator-for-each proc . gens)
      (let loop ()
        (let ((vs (map (lambda (g) (g)) gens)))
          (if (%any-eof? vs)
              (if #f #f)
              (begin (apply proc vs) (loop))))))

    (define (generator-map->list proc . gens)
      (reverse
       (let loop ((acc '()))
         (let ((vs (map (lambda (g) (g)) gens)))
           (if (%any-eof? vs) acc (loop (cons (apply proc vs) acc)))))))

    ;; These four stop as soon as they can, which is the point of pulling
    ;; rather than building a list first.
    (define (generator-find pred gen)
      (let loop ()
        (let ((x (gen)))
          (cond ((eof-object? x) #f) ((pred x) x) (else (loop))))))

    (define (generator-count pred gen)
      (let loop ((n 0))
        (let ((x (gen)))
          (cond ((eof-object? x) n) ((pred x) (loop (+ n 1))) (else (loop n))))))

    (define (generator-any pred gen)
      (let loop ()
        (let ((x (gen)))
          (if (eof-object? x)
              #f
              (let ((r (pred x))) (if r r (loop)))))))

    ;; #t for an empty generator, the last result when every value passes,
    ;; and #f at the first that does not.
    (define (generator-every pred gen)
      (let loop ((last #t))
        (let ((x (gen)))
          (if (eof-object? x)
              last
              (let ((r (pred x))) (if r (loop r) #f))))))

    ;; Hands the generator to any SRFI-1-shaped `unfold`, so a generator
    ;; can fill whatever collection that unfold builds.
    (define (generator-unfold gen unfold . args)
      (apply unfold eof-object? (lambda (x) x) (lambda (x) (gen)) (gen) args))

    ;; ── accumulators ─────────────────────────────────────────────────
    ;; An accumulator is the mirror of a generator: called with a value it
    ;; takes it in, and called with the end-of-file object it returns the
    ;; result.
    (define (make-accumulator kons knil finalizer)
      (let ((state knil))
        (lambda (x)
          (if (eof-object? x)
              (finalizer state)
              (begin (set! state (kons x state)) (if #f #f))))))

    (define (count-accumulator)
      (make-accumulator (lambda (x n) (+ n 1)) 0 (lambda (n) n)))

    (define (reverse-list-accumulator)
      (make-accumulator cons '() (lambda (acc) acc)))

    (define (list-accumulator)
      (make-accumulator cons '() reverse))

    (define (vector-accumulator)
      (make-accumulator cons '() (lambda (acc) (list->vector (reverse acc)))))

    (define (reverse-vector-accumulator)
      (make-accumulator cons '() list->vector))

    (define (vector-accumulator! vec at)
      (let ((i at))
        (lambda (x)
          (if (eof-object? x)
              vec
              (begin (vector-set! vec i x) (set! i (+ i 1)) (if #f #f))))))

    (define (string-accumulator)
      (make-accumulator cons '() (lambda (acc) (list->string (reverse acc)))))

    (define (bytevector-accumulator)
      (make-accumulator cons '()
                        (lambda (acc) (apply bytevector (reverse acc)))))

    (define (bytevector-accumulator! bv at)
      (let ((i at))
        (lambda (x)
          (if (eof-object? x)
              bv
              (begin (bytevector-u8-set! bv i x) (set! i (+ i 1)) (if #f #f))))))

    (define (sum-accumulator) (make-accumulator + 0 (lambda (n) n)))
    (define (product-accumulator) (make-accumulator * 1 (lambda (n) n)))))
