;;; (srfi 194) — Random data generators.
;;;
;;; The SRFI that earns its place in a renderer. Sampling a direction on a
;;; sphere is the inner loop of a path tracer and scattering instances over
;;; a surface is the inner loop of a modeller, and both are easy to get
;;; subtly wrong by hand: rejection sampling written badly is a silent bias
;;; rather than a crash, and the usual "normalise three uniforms" trick for
;;; a direction concentrates points toward the cube's corners.
;;;
;;; Built on `(srfi 27)`, as the SRFI says to be. A generator here is a
;;; plain thunk, which is `(srfi 158)`'s notion, so `generator->list` and
;;; friends consume these once that library exists.
;;;
;;; **The source is read at each draw, not captured when the generator is
;;; made.** So `with-random-source` scopes the source for generators built
;;; inside it *and* for draws taken inside it, which is the literal reading
;;; of "this parameter provides the random source for all procedures" and
;;; the one that keeps a seeded region reproducible.
;;;
;;; Two absences, both the numeric tower again (plan §2.3).
;;; `make-random-rectangular-generator` and `make-random-polar-generator`
;;; yield *complex* numbers, and there are none, so they are not here.
;;; `make-random-u64-generator` stops at 2^63-1 rather than 2^64-1, because
;;; the top half of that range has no exact integer to hold it.
(define-library (srfi 194)
  (export current-random-source with-random-source
          make-random-source-generator
          make-random-integer-generator make-random-real-generator
          make-random-u1-generator
          make-random-u8-generator make-random-s8-generator
          make-random-u16-generator make-random-s16-generator
          make-random-u32-generator make-random-s32-generator
          make-random-u64-generator make-random-s64-generator
          clamp-real-number
          make-random-boolean-generator make-random-char-generator
          make-random-string-generator
          make-bernoulli-generator make-binomial-generator
          make-categorical-generator
          make-normal-generator make-exponential-generator
          make-geometric-generator make-poisson-generator
          make-zipf-generator
          make-sphere-generator make-ellipsoid-generator
          make-ball-generator
          gsampling)
  (import (scheme base) (scheme inexact) (srfi 27))
  (begin

    ;; Both read `current-random-source`, so a `with-random-source` or a
    ;; `parameterize` around either construction or use is honoured.
    (define (%real) (random-real))
    (define (%int n) (random-integer n))

    (define (with-random-source source thunk)
      (parameterize ((current-random-source source)) (thunk)))

    ;; Successive substreams of one seed, which is exactly what
    ;; `random-source-pseudo-randomize!` indexes by a pair.
    (define (make-random-source-generator i)
      (let ((j 0))
        (lambda ()
          (let ((s (make-random-source)))
            (random-source-pseudo-randomize! s i j)
            (set! j (+ j 1))
            s))))

    ;; ── uniform ──────────────────────────────────────────────────────
    (define (make-random-integer-generator lower upper)
      (unless (< lower upper)
        (error "make-random-integer-generator: empty range" lower upper))
      (let ((span (- upper lower)))
        (lambda () (+ lower (%int span)))))

    ;; The SRFI's interval here is CLOSED at both ends, unlike `(srfi 27)`'s
    ;; `random-real`, so the endpoints have to be reachable.
    (define (make-random-real-generator lower upper)
      (unless (< lower upper)
        (error "make-random-real-generator: empty range" lower upper))
      (let ((span (- upper lower))
            ;; One more than the largest draw, so dividing by it gives a
            ;; value that can be exactly 0 or exactly 1.
            (steps 9007199254740992))
        (lambda ()
          (+ lower (* span (/ (%int (+ steps 1)) (exact->inexact steps)))))))

    (define (%uint bits) (lambda () (%int (expt 2 bits))))
    (define (%sint bits)
      (let ((half (expt 2 (- bits 1))))
        (lambda () (- (%int (expt 2 bits)) half))))

    (define (make-random-u1-generator) (%uint 1))
    (define (make-random-u8-generator) (%uint 8))
    (define (make-random-s8-generator) (%sint 8))
    (define (make-random-u16-generator) (%uint 16))
    (define (make-random-s16-generator) (%sint 16))
    (define (make-random-u32-generator) (%uint 32))
    (define (make-random-s32-generator) (%sint 32))
    ;; 2^64 does not fit an exact integer here, so this is the widest
    ;; unsigned range that does. See the header.
    (define (make-random-u64-generator)
      (lambda () (%int 9223372036854775807)))
    ;; The full signed range, built in two 32-bit halves. The high half is
    ;; shifted into the signed range *before* being scaled, because the
    ;; unsigned value 2^63 has no exact integer to pass through -- the
    ;; literal does not even read -- while every value of this expression
    ;; lands inside [-2^63, 2^63-1].
    (define (make-random-s64-generator)
      (lambda ()
        (+ (* (- (%int 4294967296) 2147483648) 4294967296)
           (%int 4294967296))))

    (define (clamp-real-number lower upper value)
      (cond ((< value lower) lower)
            ((> value upper) upper)
            (else value)))

    (define (make-random-boolean-generator)
      (lambda () (= 1 (%int 2))))

    (define (make-random-char-generator string)
      (let* ((n (string-length string)))
        (when (= n 0)
          (error "make-random-char-generator: the string is empty"))
        (lambda () (string-ref string (%int n)))))

    ;; `k` bounds the length, which is uniform in [0,k). The specification
    ;; text says the bound is the *string's* length; that reads as an
    ;; erratum, since it would leave `k` with nothing to do.
    (define (make-random-string-generator k string)
      (let ((chars (make-random-char-generator string)))
        (lambda ()
          (let ((len (%int k)))
            (let loop ((i 0) (acc '()))
              (if (= i len)
                  (list->string (reverse acc))
                  (loop (+ i 1) (cons (chars) acc))))))))

    ;; ── nonuniform ───────────────────────────────────────────────────
    ;; 1 and 0 rather than #t and #f: this one yields integers, so that a
    ;; run of them sums to a binomial variate.
    (define (make-bernoulli-generator p)
      (unless (and (>= p 0) (<= p 1))
        (error "make-bernoulli-generator: p must be between 0 and 1" p))
      (lambda () (if (< (%real) p) 1 0)))

    ;; Conceptually the sum of n Bernoulli-p variables, and implemented as
    ;; exactly that: linear in n, which is fine for the small n a scene
    ;; uses and worth knowing before passing a million.
    (define (make-binomial-generator n p)
      (unless (and (exact? n) (>= n 0))
        (error "make-binomial-generator: n must be a non-negative exact integer" n))
      (let ((bernoulli (make-bernoulli-generator p)))
        (lambda ()
          (let loop ((i 0) (sum 0))
            (if (= i n) sum (loop (+ i 1) (+ sum (bernoulli))))))))

    ;; A cumulative table built once, then a binary search per draw, so the
    ;; weights may be any non-negative reals and the draw is O(log n).
    (define (make-categorical-generator weight-vec)
      (let ((table (%cumulative weight-vec)))
        (lambda () (%search table (* (%real) (vector-ref table (- (vector-length table) 1)))))))

    (define (%cumulative weights)
      (let* ((n (vector-length weights))
             (out (make-vector n 0.0)))
        (when (= n 0) (error "a categorical distribution needs at least one weight"))
        (let loop ((i 0) (sum 0.0))
          (if (= i n)
              (if (<= sum 0.0)
                  (error "the weights must not all be zero")
                  out)
              (let ((w (vector-ref weights i)))
                (when (< w 0) (error "a weight must not be negative" w))
                (let ((sum (+ sum (exact->inexact w))))
                  (vector-set! out i sum)
                  (loop (+ i 1) sum)))))))

    ;; The first index whose cumulative weight exceeds `x`.
    (define (%search table x)
      (let loop ((lo 0) (hi (- (vector-length table) 1)))
        (if (>= lo hi)
            lo
            (let ((mid (quotient (+ lo hi) 2)))
              (if (< x (vector-ref table mid))
                  (loop lo mid)
                  (loop (+ mid 1) hi))))))

    ;; Box-Muller. Both of a pair are kept rather than thrown away, so this
    ;; costs one `log` and one `sqrt` per two draws.
    (define (make-normal-generator . opt)
      (let ((mean (if (pair? opt) (car opt) 0.0))
            (deviation (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) 1.0))
            (spare #f))
        (lambda ()
          (if spare
              (let ((z spare))
                (set! spare #f)
                (+ mean (* deviation z)))
              ;; `random-real` is open at zero, so the log is always finite.
              (let* ((r (sqrt (* -2.0 (log (%real)))))
                     (theta (* 8.0 (atan 1.0) (%real))))
                (set! spare (* r (sin theta)))
                (+ mean (* deviation (* r (cos theta)))))))))

    (define (make-exponential-generator mean)
      (unless (> mean 0)
        (error "make-exponential-generator: the mean must be positive" mean))
      (lambda () (* (- (exact->inexact mean)) (log (%real)))))

    ;; The number of trials up to and including the first success, so the
    ;; support is 1, 2, 3, … and the mean is 1/p.
    (define (make-geometric-generator p)
      (unless (and (> p 0) (<= p 1))
        (error "make-geometric-generator: p must be in (0,1]" p))
      (if (= p 1)
          (lambda () 1)
          (let ((denominator (log (- 1.0 (exact->inexact p)))))
            (lambda () (+ 1 (exact (floor (/ (log (%real)) denominator))))))))

    ;; Summed exponential interarrivals until they pass 1. That is Knuth's
    ;; method moved into log space, which matters: the product form needs
    ;; `exp(-L)`, and that underflows to zero for a mean past about 745,
    ;; where it would loop forever. Cost is linear in the mean, which is
    ;; the right trade for the small means a scene uses.
    (define (make-poisson-generator L)
      (unless (> L 0)
        (error "make-poisson-generator: the mean must be positive" L))
      (let ((mean (exact->inexact L)))
        (lambda ()
          (let loop ((k 0) (sum 0.0))
            (let ((sum (- sum (log (%real)))))
              (if (> sum mean) k (loop (+ k 1) sum)))))))

    ;; Generalised Zipf over [1,N] with weights (k+q)^-s. A cumulative
    ;; table costs O(N) once and makes every draw O(log N) and exact for
    ;; any s and q, where the usual rejection methods assume q is zero.
    (define (make-zipf-generator n . opt)
      (unless (and (exact? n) (> n 0))
        (error "make-zipf-generator: N must be a positive exact integer" n))
      (let* ((s (if (pair? opt) (car opt) 1.0))
             (q (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) 0.0))
             (weights (make-vector n 0.0)))
        (let loop ((k 1))
          (when (<= k n)
            (let ((base (+ k (exact->inexact q))))
              (when (<= base 0.0)
                (error "make-zipf-generator: q makes a weight undefined" q))
              (vector-set! weights (- k 1) (expt base (- (exact->inexact s)))))
            (loop (+ k 1))))
        (let ((categorical (make-categorical-generator weights)))
          (lambda () (+ 1 (categorical))))))

    ;; ── the geometric ones, which is why this library matters here ───
    ;; `(make-sphere-generator n)` gives points on the surface of an
    ;; n-sphere, which live in n+1 dimensions -- so 2 is the ordinary
    ;; sphere in 3-space and the generator yields 3-element vectors.
    ;;
    ;; Normalised Gaussians, not normalised uniforms. A vector of uniforms
    ;; divided by its length is NOT uniform on the sphere: it concentrates
    ;; toward the cube's corners, which in a renderer shows up as a faint
    ;; eight-lobed bias in the lighting rather than as an error.
    (define (make-sphere-generator n)
      (unless (and (exact? n) (>= n 0))
        (error "make-sphere-generator: n must be a non-negative exact integer" n))
      (%normalised-gaussian (+ n 1)))

    (define (%normalised-gaussian dim)
      (let ((normal (make-normal-generator 0.0 1.0)))
        (lambda ()
          ;; A zero vector has no direction; it needs 53 coincident bits to
          ;; happen, and redrawing is exact where nudging would not be.
          (let retry ()
            (let ((v (make-vector dim 0.0)))
              (let loop ((i 0) (sum 0.0))
                (if (= i dim)
                    (if (= sum 0.0)
                        (retry)
                        (let ((len (sqrt sum)))
                          (let scale ((i 0))
                            (if (= i dim)
                                v
                                (begin (vector-set! v i (/ (vector-ref v i) len))
                                       (scale (+ i 1)))))))
                    (let ((x (normal)))
                      (vector-set! v i x)
                      (loop (+ i 1) (+ sum (* x x)))))))))))

    ;; A sphere point scaled per axis. That is the SRFI's construction and
    ;; it is worth being honest about: the points lie on the ellipsoid but
    ;; are not uniform with respect to its surface *area*, because the
    ;; scaling stretches some patches more than others.
    (define (make-ellipsoid-generator axes)
      (let* ((dim (vector-length axes))
             (on-sphere (%normalised-gaussian dim)))
        (when (= dim 0)
          (error "make-ellipsoid-generator: axes must not be empty"))
        (lambda ()
          (let ((v (on-sphere)))
            (let loop ((i 0))
              (if (= i dim)
                  v
                  (begin (vector-set! v i (* (vector-ref v i)
                                             (exact->inexact (vector-ref axes i))))
                         (loop (+ i 1)))))))))

    ;; Inside the ball, uniformly by volume. The radius is scaled by
    ;; u^(1/dim) rather than by u, because volume grows as r^dim and a
    ;; linear radius would pile the points up at the centre.
    (define (make-ball-generator dimensions)
      (let* ((axes (if (vector? dimensions) dimensions #f))
             (dim (if axes (vector-length axes) dimensions)))
        (unless (or axes (and (exact? dim) (> dim 0)))
          (error "make-ball-generator: expects a positive exact integer or a vector of axes"
                 dimensions))
        (let ((on-surface (if axes
                              (make-ellipsoid-generator axes)
                              (%normalised-gaussian dim))))
          (lambda ()
            (let ((v (on-surface))
                  (r (expt (%real) (/ 1.0 dim))))
              (let loop ((i 0))
                (if (= i dim)
                    v
                    (begin (vector-set! v i (* r (vector-ref v i)))
                           (loop (+ i 1))))))))))

    ;; ── combining ────────────────────────────────────────────────────
    ;; Pick one of the generators uniformly and call it. An exhausted one
    ;; is dropped rather than picked again, so the result ends only when
    ;; every input has.
    (define (gsampling . generators)
      (let ((live (list->vector generators)))
        (lambda ()
          (let loop ()
            (let ((n (vector-length live)))
              (if (= n 0)
                  (eof-object)
                  (let* ((i (%int n))
                         (x ((vector-ref live i))))
                    (if (eof-object? x)
                        (begin (set! live (%drop live i)) (loop))
                        x))))))))

    (define (%drop v i)
      (let* ((n (vector-length v))
             (out (make-vector (- n 1) #f)))
        (let loop ((j 0) (k 0))
          (cond ((= j n) out)
                ((= j i) (loop (+ j 1) k))
                (else (vector-set! out k (vector-ref v j))
                      (loop (+ j 1) (+ k 1)))))))))
