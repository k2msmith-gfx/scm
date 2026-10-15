;;; (srfi 42) — Eager comprehensions.
;;;
;;; The standard answer to Janet's `seq`, which the scene-language survey
;;; counted 253 times as the second most common non-trivial form. A grid
;;; of instances, a lattice, a swept profile: all of them are
;;; `(list-ec (: i n) …)`.
;;;
;;; ## How it fits scm's keyword syntax
;;;
;;; Every generator in this SRFI is spelled `:list`, `:range`, `:while`.
;;; Under plan §4 those read as **keywords**, not identifiers, so they
;;; cannot be macro names and cannot even be `syntax-rules` literals.
;;;
;;; They do not need to be either. A generator is only ever a *qualifier*
;;; inside a comprehension, and a keyword in a pattern matches as a
;;; constant -- so the comprehension macros below recognise the generator
;;; forms directly, and nothing named `:list` is bound anywhere. Ordinary
;;; SRFI 42 code works unchanged, with no change to the reader and none to
;;; `syntax-rules`.
;;;
;;; Three names in the SRFI cannot survive that, because they are used in
;;; *operator* position where a keyword is an error:
;;;
;;;   * **`:generator-proc` is absent.** Turning a comprehension inside out
;;;     into a pull-driven procedure also wants a re-entrant continuation,
;;;     and plan §3.1 makes continuations escape-only. Use a comprehension.
;;;   * **`:-dispatch-ref` and `:-dispatch-set!` are spelled
;;;     `dispatch-ref` and `dispatch-set!`.** Same procedures, a spelling
;;;     that reads as an identifier.
;;;
;;; ## What is here
;;;
;;; All seventeen comprehensions, all the generators including `:parallel`,
;;; `:while`, `:until`, `:do` and the dispatching `:`, and every qualifier
;;; form: `(if t)`, `(not t)`, `(and …)`, `(or …)`, `(begin …)` and
;;; `(nested …)`. `<vars>` may be `v` or `(v (index i))` for any generator.
;;;
;;; ## How it works
;;;
;;; Every generator normalises to one internal form,
;;;
;;;     (%loop (outer …) ((var init step) …) continue? (inner …))
;;;
;;; which is a `do` loop in pieces: bindings evaluated once, loop variables
;;; with their steps, the test, and bindings evaluated per iteration. That
;;; normal form is the whole reason `:parallel` is possible -- running two
;;; generators in lockstep is appending their pieces, where nesting two
;;; loops could not express it. Generators hand their normal form to a
;;; continuation *macro*, because a macro cannot return a value.
;;;
;;; Accumulation is a mutable variable rather than a threaded one, so the
;;; loop body is a command and every comprehension shares one walker. The
;;; four comprehensions that stop early -- `any?-ec`, `every?-ec`,
;;; `first-ec`, and `:while`/`:until` -- escape with a continuation, which
;;; is the one kind plan §3.1 supports.
(define-library (srfi 42)
  (export do-ec list-ec append-ec string-ec string-append-ec
          vector-ec vector-of-length-ec
          sum-ec product-ec min-ec max-ec
          any?-ec every?-ec first-ec last-ec fold-ec fold3-ec
          dispatch-ref dispatch-set! make-initial-:-dispatch dispatch-union)
  (import (scheme base) (scheme inexact))
  (begin

    ;; ── the qualifier walker ─────────────────────────────────────────
    ;; `(%ec body (qualifier …))` nests one loop or guard per qualifier
    ;; around `body`, left to right, so the rightmost generator spins
    ;; fastest. `body` is a command; comprehensions accumulate into a
    ;; variable it closes over.
    (define-syntax %ec
      (syntax-rules (if not and or begin nested)
        ;; nothing left to qualify
        ((_ body ()) body)

        ;; filters and side effects
        ((_ body ((if test) q ...)) (if test (%ec body (q ...))))
        ((_ body ((not test) q ...)) (if test #f (%ec body (q ...))))
        ((_ body ((and test ...) q ...)) (if (and test ...) (%ec body (q ...))))
        ((_ body ((or test ...) q ...)) (if (or test ...) (%ec body (q ...))))
        ((_ body ((begin cmd ...) q ...)) (begin cmd ... (%ec body (q ...))))
        ;; grouping: splice, which is all `nested` means
        ((_ body ((nested p ...) q ...)) (%ec body (p ... q ...)))

        ;; `:while` stops its generator when the test fails; `:until`
        ;; stops after the iteration where it holds. Each escapes only its
        ;; own loop, so outer generators keep going.
        ((_ body ((:while gen test) q ...))
         (%while-check body (q ...) gen test))
        ((_ body ((:until gen test) q ...))
         (%until-check body (q ...) gen test))

        ;; any other qualifier is a generator
        ((_ body (gen q ...)) (%norm (%nest body (q ...)) gen))))

    ;; ── the loop emitters ────────────────────────────────────────────
    ;; A normalised loop, with the remaining qualifiers inside it.
    (define-syntax %nest
      (syntax-rules (%loop)
        ((_ body (q ...) (%loop (ob ...) ((lv li ls) ...) ne1 (ib ...)))
         (let* (ob ...)
           (let %lp ((lv li) ...)
             (if ne1
                 (begin (let* (ib ...) (%ec body (q ...)))
                        (%lp ls ...))
                 #f))))))

    ;; The same, with a counting variable exposed as `i`.
    (define-syntax %indexed
      (syntax-rules (%loop)
        ((_ (k ...) i (%loop (ob ...) ((lv li ls) ...) ne1 (ib ...)))
         (k ... (%loop (ob ...) ((lv li ls) ... (%ix 0 (+ %ix 1))) ne1
                       (ib ... (i %ix)))))))

    ;; ── normalising a generator ──────────────────────────────────────
    ;; `(%norm (k …) generator)` expands to `(k … (%loop …))`. A macro
    ;; cannot return a value, so the result is handed to a continuation
    ;; macro -- which is also what lets `:parallel` collect several.
    ;; `:` is a plain symbol, unlike every other generator head, so it has
    ;; to be declared a literal -- undeclared it would be a pattern
    ;; VARIABLE and its rule would swallow every other generator form.
    (define-syntax %norm
      (syntax-rules (: index)
        ;; `(v (index i))` for ANY generator: normalise with the plain
        ;; variable, then add a counting loop variable. One rule serves
        ;; every generator, and it lives here rather than in the walker so
        ;; that `:parallel`, which normalises its own sub-generators, gets
        ;; it too. Nothing else matches: a two-element second position
        ;; whose own second element starts with `index` is this and
        ;; nothing else -- `:do`'s bindings and `:parallel`'s generators
        ;; are either the wrong length or not headed by `index`.
        ((_ (k ...) (g (v (index i)) rest ...))
         (%norm (%indexed (k ...) i) (g v rest ...)))

        ;; :list — the elements of every argument in turn
        ((_ (k ...) (:list v arg ...))
         (k ... (%loop ((%xs (append arg ...)))
                       ((%t %xs (cdr %t)))
                       (pair? %t)
                       ((v (car %t))))))

        ;; :string — indexed rather than converted to a list, so a long
        ;; string costs no allocation per character
        ((_ (k ...) (:string v arg ...))
         (k ... (%loop ((%s (string-append arg ...)) (%n (string-length %s)))
                       ((%i 0 (+ %i 1)))
                       (< %i %n)
                       ((v (string-ref %s %i))))))

        ;; :vector
        ((_ (k ...) (:vector v arg ...))
         (k ... (%loop ((%v (%vector-concat (list arg ...)))
                        (%n (vector-length %v)))
                       ((%i 0 (+ %i 1)))
                       (< %i %n)
                       ((v (vector-ref %v %i))))))

        ;; :range, in its three forms. The variable *is* the loop
        ;; variable, so there is no per-iteration binding at all.
        ((_ (k ...) (:range v stop)) (%norm (k ...) (:range v 0 stop 1)))
        ((_ (k ...) (:range v start stop)) (%norm (k ...) (:range v start stop 1)))
        ((_ (k ...) (:range v start stop step))
         (k ... (%loop ((%a start) (%b stop) (%s (%nonzero step)))
                       ((v %a (+ v %s)))
                       (if (> %s 0) (< v %b) (> v %b))
                       ())))

        ;; :real-range — computed as start + i*step rather than by
        ;; repeated addition, so the error does not accumulate across a
        ;; long range and the count is fixed before the loop starts.
        ((_ (k ...) (:real-range v stop)) (%norm (k ...) (:real-range v 0 stop 1)))
        ((_ (k ...) (:real-range v start stop))
         (%norm (k ...) (:real-range v start stop 1)))
        ((_ (k ...) (:real-range v start stop step))
         (k ... (%loop ((%a start) (%b stop) (%s (%nonzero step))
                        (%n (%range-count %a %b %s)))
                       ((%i 0 (+ %i 1)))
                       (< %i %n)
                       ((v (+ %a (* %i %s)))))))

        ;; :char-range — inclusive of the upper bound, unlike :range
        ((_ (k ...) (:char-range v lo hi))
         (k ... (%loop ((%lo (char->integer lo)) (%hi (char->integer hi)))
                       ((%c %lo (+ %c 1)))
                       (<= %c %hi)
                       ((v (integer->char %c))))))

        ;; :integers — unbounded, so something else has to stop it
        ((_ (k ...) (:integers v))
         (k ... (%loop () ((v 0 (+ v 1))) #t ())))

        ;; :port
        ((_ (k ...) (:port v port)) (%norm (k ...) (:port v port read)))
        ((_ (k ...) (:port v port reader))
         (k ... (%loop ((%p port) (%rd reader))
                       ((%x (%rd %p) (%rd %p)))
                       (not (eof-object? %x))
                       ((v %x)))))

        ;; :let — one iteration, which is how a comprehension binds a
        ;; local without leaving the qualifier list
        ((_ (k ...) (:let v expr))
         (k ... (%loop ((%x expr)) ((%first #t #f)) %first ((v %x)))))

        ;; :do — the low-level form, mapped straight onto the normal form
        ((_ (k ...) (:do ((lv li) ...) ne1 (ls ...)))
         (k ... (%loop () ((lv li ls) ...) ne1 ())))

        ;; :dispatched and `:` — the argument types are only known at run
        ;; time, so the generator is a procedure called once per value.
        ;; This is the one generator that allocates a closure per use.
        ((_ (k ...) (:dispatched v dispatch arg ...))
         (k ... (%loop ((%g (%dispatched-proc dispatch (list arg ...))))
                       ((%x (%g) (%g)))
                       (not (eof-object? %x))
                       ((v %x)))))
        ((_ (k ...) (: v arg ...))
         (%norm (k ...) (:dispatched v (dispatch-ref) arg ...)))

        ;; :parallel — the reason the normal form exists. Lockstep, not
        ;; nesting, so the pieces of each generator are appended and the
        ;; loop ends when the first of them does.
        ((_ (k ...) (:parallel)) (k ... (%loop () ((%p #t #f)) %p ())))
        ((_ (k ...) (:parallel gen)) (%norm (k ...) gen))
        ((_ (k ...) (:parallel gen rest ...))
         (%norm (%merge-first (k ...) (rest ...)) gen))))

    ;; `:parallel`, continued: normalise the rest, then append the pieces.
    (define-syntax %merge-first
      (syntax-rules (%loop)
        ((_ (k ...) (rest ...) (%loop obs lvs ne1 ibs))
         (%norm (%merge-rest (k ...) obs lvs ne1 ibs) (:parallel rest ...)))))

    (define-syntax %merge-rest
      (syntax-rules (%loop)
        ((_ (k ...) (ob1 ...) ((lv1 li1 ls1) ...) ne1a (ib1 ...)
            (%loop (ob2 ...) ((lv2 li2 ls2) ...) ne1b (ib2 ...)))
         (k ... (%loop (ob1 ... ob2 ...)
                       ((lv1 li1 ls1) ... (lv2 li2 ls2) ...)
                       (and ne1a ne1b)
                       (ib1 ... ib2 ...))))))

    ;; ── :while and :until ────────────────────────────────────────────
    ;; The guard has to sit *inside* the generator's per-iteration
    ;; bindings, because the test reads the variable the generator binds.
    (define-syntax %while-check
      (syntax-rules ()
        ((_ body (q ...) gen test)
         (call-with-current-continuation
          (lambda (%brk) (%norm (%nest-while body (q ...) %brk test) gen))))))

    (define-syntax %nest-while
      (syntax-rules (%loop)
        ((_ body (q ...) brk test (%loop (ob ...) ((lv li ls) ...) ne1 (ib ...)))
         (let* (ob ...)
           (let %lp ((lv li) ...)
             (if ne1
                 (begin (let* (ib ...)
                          (if test (%ec body (q ...)) (brk #f)))
                        (%lp ls ...))
                 #f))))))

    (define-syntax %until-check
      (syntax-rules ()
        ((_ body (q ...) gen test)
         (call-with-current-continuation
          (lambda (%brk) (%norm (%nest-until body (q ...) %brk test) gen))))))

    ;; `:until` runs the body first and stops *after* the iteration in
    ;; which the test holds, which is the whole difference from `:while`.
    (define-syntax %nest-until
      (syntax-rules (%loop)
        ((_ body (q ...) brk test (%loop (ob ...) ((lv li ls) ...) ne1 (ib ...)))
         (let* (ob ...)
           (let %lp ((lv li) ...)
             (if ne1
                 (begin (let* (ib ...)
                          (%ec body (q ...))
                          (if test (brk #f) #f))
                        (%lp ls ...))
                 #f))))))

    ;; ── the comprehensions ───────────────────────────────────────────
    (define-syntax do-ec
      (syntax-rules ()
        ((_ cmd) cmd)
        ((_ q ... cmd) (%ec cmd (q ...)))))

    ;; Accumulated in reverse and reversed once, rather than appended per
    ;; element, so a comprehension over n values costs O(n).
    (define-syntax list-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc '()))
           (%ec (set! %acc (cons expr %acc)) (q ...))
           (reverse %acc)))))

    (define-syntax append-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc '()))
           (%ec (set! %acc (cons expr %acc)) (q ...))
           (apply append (reverse %acc))))))

    (define-syntax string-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc '()))
           (%ec (set! %acc (cons expr %acc)) (q ...))
           (list->string (reverse %acc))))))

    (define-syntax string-append-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc '()))
           (%ec (set! %acc (cons expr %acc)) (q ...))
           (apply string-append (reverse %acc))))))

    (define-syntax vector-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc '()))
           (%ec (set! %acc (cons expr %acc)) (q ...))
           (list->vector (reverse %acc))))))

    ;; The length is known, so this fills in place -- and a value count
    ;; that disagrees with it is an error rather than a silent truncation.
    (define-syntax vector-of-length-ec
      (syntax-rules ()
        ((_ k q ... expr)
         (let* ((%n k) (%v (make-vector %n)) (%i 0))
           (%ec (begin
                  (when (>= %i %n)
                    (error "vector-of-length-ec: more values than the length" %n))
                  (vector-set! %v %i expr)
                  (set! %i (+ %i 1)))
                (q ...))
           (if (< %i %n)
               (error "vector-of-length-ec: fewer values than the length" %i %n)
               %v)))))

    (define-syntax sum-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc 0)) (%ec (set! %acc (+ %acc expr)) (q ...)) %acc))))

    (define-syntax product-ec
      (syntax-rules ()
        ((_ q ... expr)
         (let ((%acc 1)) (%ec (set! %acc (* %acc expr)) (q ...)) %acc))))

    ;; The first value seeds the comparison, so these need no infinity --
    ;; which matters, because an exact one does not exist here.
    (define-syntax min-ec
      (syntax-rules ()
        ((_ q ... expr) (%extremum-ec min "min-ec" (q ...) expr))))

    (define-syntax max-ec
      (syntax-rules ()
        ((_ q ... expr) (%extremum-ec max "max-ec" (q ...) expr))))

    (define-syntax %extremum-ec
      (syntax-rules ()
        ((_ pick who (q ...) expr)
         (let ((%acc #f) (%any #f))
           (%ec (let ((%x expr))
                  (if %any (set! %acc (pick %acc %x)) (begin (set! %acc %x) (set! %any #t))))
                (q ...))
           (if %any %acc (error (string-append who ": no values")))))))

    ;; Stops at the first true value, which is the point of having it
    ;; rather than folding `or` over everything.
    (define-syntax any?-ec
      (syntax-rules ()
        ((_ q ... test)
         (call-with-current-continuation
          (lambda (%return)
            (%ec (if test (%return #t)) (q ...))
            #f)))))

    (define-syntax every?-ec
      (syntax-rules ()
        ((_ q ... test)
         (call-with-current-continuation
          (lambda (%return)
            (%ec (if test #f (%return #f)) (q ...))
            #t)))))

    (define-syntax first-ec
      (syntax-rules ()
        ((_ default q ... expr)
         (call-with-current-continuation
          (lambda (%return)
            (%ec (%return expr) (q ...))
            default)))))

    (define-syntax last-ec
      (syntax-rules ()
        ((_ default q ... expr)
         (let ((%acc default)) (%ec (set! %acc expr) (q ...)) %acc))))

    ;; `(f2 value accumulator)`.
    (define-syntax fold-ec
      (syntax-rules ()
        ((_ x0 q ... expr f2)
         (let ((%acc x0)) (%ec (set! %acc (f2 expr %acc)) (q ...)) %acc))))

    ;; `f1` makes the accumulator from the first value, so an empty
    ;; sequence yields `x0` and a non-empty one never sees it.
    (define-syntax fold3-ec
      (syntax-rules ()
        ((_ x0 q ... expr f1 f2)
         (let ((%acc #f) (%any #f))
           (%ec (let ((%x expr))
                  (if %any
                      (set! %acc (f2 %x %acc))
                      (begin (set! %acc (f1 %x)) (set! %any #t))))
                (q ...))
           (if %any %acc x0)))))

    ;; ── the run-time helpers ─────────────────────────────────────────
    (define (%vector-concat vs)
      (if (null? (cdr vs))
          (car vs)
          (let* ((n (apply + (map vector-length vs)))
                 (out (make-vector n)))
            (let loop ((vs vs) (at 0))
              (if (null? vs)
                  out
                  (let ((v (car vs)))
                    (let copy ((i 0))
                      (if (< i (vector-length v))
                          (begin (vector-set! out (+ at i) (vector-ref v i))
                                 (copy (+ i 1)))
                          (loop (cdr vs) (+ at (vector-length v))))))))))) 

    ;; A zero step would never terminate, so it is refused rather than
    ;; hung on.
    (define (%nonzero step)
      (if (and (number? step) (not (= step 0)))
          step
          (error "a range step must be a non-zero number" step)))

    ;; How many values a real range holds. Computed once, so the loop
    ;; cannot drift past its end through accumulated rounding.
    (define (%range-count a b s)
      (let ((n (ceiling (/ (- b a) s))))
        (if (> n 0) (exact (floor n)) 0)))

    ;; ── the dispatcher ───────────────────────────────────────────────
    ;; `:` evaluates its arguments and then asks a dispatcher for a
    ;; procedure that yields successive values and then the end-of-file
    ;; object. A dispatcher takes the argument list and returns such a
    ;; procedure, or #f if it does not recognise them.
    (define (%dispatched-proc dispatch args)
      (let ((proc (dispatch args)))
        (if proc
            proc
            (error "(: …) does not know how to generate from these arguments" args))))

    (define (%list-proc lst)
      (lambda ()
        (if (pair? lst)
            (let ((x (car lst))) (set! lst (cdr lst)) x)
            (eof-object))))

    (define (%count-proc n f)
      (let ((i 0))
        (lambda ()
          (if (< i n)
              (let ((x (f i))) (set! i (+ i 1)) x)
              (eof-object)))))

    (define (make-initial-:-dispatch)
      (lambda (args)
        (cond
         ((null? args) #f)
         ;; lists, strings and vectors: every argument must be the same
         ;; kind, and they are taken in order
         ((%all list? args) (%list-proc (apply append args)))
         ((%all string? args)
          (let ((s (apply string-append args)))
            (%count-proc (string-length s) (lambda (i) (string-ref s i)))))
         ((%all vector? args)
          (let ((v (%vector-concat args)))
            (%count-proc (vector-length v) (lambda (i) (vector-ref v i)))))
         ;; two characters is a character range, before the number cases
         ((and (= 2 (length args)) (%all char? args))
          (let ((lo (char->integer (car args))) (hi (char->integer (cadr args))))
            (%count-proc (max 0 (+ 1 (- hi lo)))
                         (lambda (i) (integer->char (+ lo i))))))
         ;; one to three numbers is a range; exactness decides which kind
         ((and (<= 1 (length args) 3) (%all number? args))
          (let* ((a (if (> (length args) 1) (car args) 0))
                 (b (if (> (length args) 1) (cadr args) (car args)))
                 (s (if (> (length args) 2) (caddr args) 1)))
            (%nonzero s)
            (%count-proc (%range-count a b s) (lambda (i) (+ a (* i s))))))
         ;; a port, with an optional reader
         ((and (= 1 (length args)) (input-port? (car args)))
          (%port-proc (car args) read))
         ((and (= 2 (length args)) (input-port? (car args)) (procedure? (cadr args)))
          (%port-proc (car args) (cadr args)))
         (else #f))))

    (define (%port-proc port reader)
      (lambda () (reader port)))

    (define (%all pred xs)
      (cond ((null? xs) #t) ((pred (car xs)) (%all pred (cdr xs))) (else #f)))

    ;; Try `d1`, then `d2`. That is all a union is, and it makes the
    ;; later-installed dispatcher take precedence.
    (define (dispatch-union d1 d2)
      (lambda (args) (or (d1 args) (d2 args))))

    (define %dispatch (make-initial-:-dispatch))
    (define (dispatch-ref) %dispatch)
    (define (dispatch-set! d)
      (let ((old %dispatch)) (set! %dispatch d) old))))
