;;; prelude.scm — the derived forms, in Scheme.
;;;
;;; Everything here could have been a special form in Rust. None of it is,
;;; which is the whole return on `syntax-rules`: these are the shapes a scene
;;; and a model both write constantly, and they are now 60 lines of Scheme
;;; instead of 600 of matcher.
;;;
;;; Two notes on shape:
;;;
;;;   * Bracket bindings work everywhere -- `(let ([x 1]) …)` as well as
;;;     `(let ((x 1)) …)` -- and not because of anything written here. The
;;;     matcher treats a bracket vector as a list (plan §4.9, syntax.rs), so
;;;     every form below inherits it.
;;;   * `t` and the other temporaries are safe. A template-introduced
;;;     identifier is renamed to a symbol no source text can name, so a user's
;;;     own `t` cannot be captured.

(define-syntax let
  (syntax-rules ()
    ;; Named let, which is also how every loop in a converted scene will be
    ;; written. It must come first: `(let loop ((i 0)) …)` would otherwise
    ;; match the ordinary pattern with `loop` as a binding list.
    [(_ name ((var init) ...) body ...)
     ((letrec ((name (lambda (var ...) body ...))) name) init ...)]
    [(_ ((var init) ...) body ...)
     ((lambda (var ...) body ...) init ...)]))

(define-syntax let*
  (syntax-rules ()
    [(_ () body ...) (let () body ...)]
    [(_ ((var init) rest ...) body ...)
     (let ((var init)) (let* (rest ...) body ...))]))

(define-syntax letrec
  (syntax-rules ()
    [(_ ((var init) ...) body ...)
     (let ((var #f) ...)
       (set! var init) ...
       body ...)]))

(define-syntax letrec* (syntax-rules () [(_ bs body ...) (letrec bs body ...)]))

(define-syntax and
  (syntax-rules ()
    [(_) #t]
    [(_ e) e]
    [(_ e rest ...) (if e (and rest ...) #f)]))

(define-syntax or
  (syntax-rules ()
    [(_) #f]
    [(_ e) e]
    ;; `t` is the classic hygiene test: a user's own `t` must not be captured.
    [(_ e rest ...) (let ((t e)) (if t t (or rest ...)))]))

;; `(if #f #f)` is the idiom for the unspecified value, and R7RS says that is
;; what a `when` with a false test produces. Returning `#f` would be legal and
;; would also make `(when #f 1)` indistinguishable from a body that returned
;; false on purpose.
(define-syntax when
  (syntax-rules ()
    [(_ test body ...) (if test (begin body ...) (if #f #f))]))

(define-syntax unless
  (syntax-rules ()
    [(_ test body ...) (if test (if #f #f) (begin body ...))]))

(define-syntax cond
  (syntax-rules (else =>)
    [(_) #f]
    [(_ (else body ...)) (begin body ...)]
    [(_ (test => proc) rest ...)
     (let ((t test)) (if t (proc t) (cond rest ...)))]
    [(_ (test) rest ...) (let ((t test)) (if t t (cond rest ...)))]
    [(_ (test body ...) rest ...)
     (if test (begin body ...) (cond rest ...))]))

(define-syntax case
  (syntax-rules (else =>)
    [(_ key) #f]
    [(_ key (else => proc)) (proc key)]
    [(_ key (else body ...)) (begin body ...)]
    [(_ key ((datum ...) => proc) rest ...)
     (let ((k key)) (if (memv k '(datum ...)) (proc k) (case k rest ...)))]
    [(_ key ((datum ...) body ...) rest ...)
     (let ((k key)) (if (memv k '(datum ...)) (begin body ...) (case k rest ...)))]))

(define-syntax do
  (syntax-rules ()
    [(_ ((var init step ...) ...) (test expr ...) command ...)
     (let loop ((var init) ...)
       (if test
           (begin #f expr ...)
           (begin command ... (loop (do-step var step ...) ...))))]))

;; A `do` variable with no step expression keeps its value.
(define-syntax do-step
  (syntax-rules ()
    [(_ var) var]
    [(_ var step) step]))

;; ── the list vocabulary the scene corpus and R7RS share ──────────────────

(define (caar p) (car (car p)))
(define (cadr p) (car (cdr p)))
(define (cdar p) (cdr (car p)))
(define (cddr p) (cdr (cdr p)))
(define (caddr p) (car (cddr p)))
(define (cdddr p) (cdr (cddr p)))

(define (list? x)
  (let loop ((x x))
    (cond [(null? x) #t]
          [(pair? x) (loop (cdr x))]
          [else #f])))

(define (reverse lst)
  (let loop ((lst lst) (acc '()))
    (if (null? lst) acc (loop (cdr lst) (cons (car lst) acc)))))

(define (append2 a b)
  (if (null? a) b (cons (car a) (append2 (cdr a) b))))

(define (append . lists)
  (let loop ((ls lists))
    (cond [(null? ls) '()]
          [(null? (cdr ls)) (car ls)]
          [else (append2 (car ls) (loop (cdr ls)))])))

(define (list-tail lst k)
  (if (= k 0) lst (list-tail (cdr lst) (- k 1))))

(define (list-ref lst k) (car (list-tail lst k)))

(define (memv x lst)
  (cond [(null? lst) #f]
        [(eqv? x (car lst)) lst]
        [else (memv x (cdr lst))]))

(define (member x lst)
  (cond [(null? lst) #f]
        [(equal? x (car lst)) lst]
        [else (member x (cdr lst))]))

(define (memq x lst) (memv x lst))

(define (assv x alist)
  (cond [(null? alist) #f]
        [(eqv? x (car (car alist))) (car alist)]
        [else (assv x (cdr alist))]))

(define (assq x alist) (assv x alist))

(define (assoc x alist)
  (cond [(null? alist) #f]
        [(equal? x (car (car alist))) (car alist)]
        [else (assoc x (cdr alist))]))

;; `map` over one or more lists, and `for-each` for effect. Written with an
;; accumulate-and-reverse loop rather than recursion, because a scene maps over
;; thousands of points and this is the idiom a model writes (plan §4.9).
(define (map f lst . more)
  (if (null? more)
      (let loop ((lst lst) (acc '()))
        (if (null? lst)
            (reverse acc)
            (loop (cdr lst) (cons (f (car lst)) acc))))
      (let loop ((ls (cons lst more)) (acc '()))
        (if (any-null? ls)
            (reverse acc)
            (loop (map-cdr ls) (cons (apply f (map-car ls)) acc))))))

(define (any-null? ls)
  (cond [(null? ls) #f]
        [(null? (car ls)) #t]
        [else (any-null? (cdr ls))]))

(define (map-car ls) (if (null? ls) '() (cons (car (car ls)) (map-car (cdr ls)))))
(define (map-cdr ls) (if (null? ls) '() (cons (cdr (car ls)) (map-cdr (cdr ls)))))

(define (for-each f lst . more)
  (if (null? more)
      (let loop ((lst lst))
        (if (null? lst)
            (if #f #f)
            (begin (f (car lst)) (loop (cdr lst)))))
      (let loop ((ls (cons lst more)))
        (if (any-null? ls)
            (if #f #f)
            (begin (apply f (map-car ls)) (loop (map-cdr ls)))))))

(define (filter pred lst)
  (let loop ((lst lst) (acc '()))
    (cond [(null? lst) (reverse acc)]
          [(pred (car lst)) (loop (cdr lst) (cons (car lst) acc))]
          [else (loop (cdr lst) acc)])))

(define (reduce f init lst)
  (let loop ((lst lst) (acc init))
    (if (null? lst) acc (loop (cdr lst) (f acc (car lst))))))

(define (fold-left f init lst) (reduce f init lst))

(define (apply-to-list f args) (apply f args))

;; ── iteration, in the shapes a converted scene uses ─────────────────────
;;
;; `seq`, `for`, `each` and `repeat` keep Janet's names on purpose: the corpus
;; has 253 `seq`, 174 `for` and 156 `each`, and rewriting them into named lets
;; would be 580 chances to be subtly wrong (scene-language-survey §5).

(define-syntax each
  (syntax-rules ()
    [(_ var lst body ...) (for-each (lambda (var) body ...) lst)]))

(define-syntax for
  (syntax-rules ()
    [(_ var lo hi body ...)
     (let loop ((var lo))
       (when (< var hi)
         body ...
         (loop (+ var 1))))]))

(define-syntax repeat
  (syntax-rules ()
    [(_ n body ...)
     (let loop ((i 0))
       (when (< i n)
         body ...
         (loop (+ i 1))))]))

;; `(seq (i lo hi) expr)` collects `expr` for each `i`, and
;; `(seq (x in lst) expr)` collects over a list. Janet's `:range` spelling is a
;; conversion detail and belongs to whatever does the converting.
(define-syntax seq
  (syntax-rules (in)
    [(_ (var in lst) expr)
     (map (lambda (var) expr) lst)]
    [(_ (var lo hi) expr)
     (let loop ((var lo) (acc '()))
       (if (< var hi)
           (loop (+ var 1) (cons expr acc))
           (reverse acc)))]))

;; ── derived from the vector and string primitives ───────────────────────

(define (vector-map f v) (list->vector (map f (vector->list v))))
(define (vector-for-each f v) (for-each f (vector->list v)))
(define (vector-length* v) (vector-length v))

;; Janet's `inc`/`dec`, kept because the corpus uses them 151 times and they
;; cost two lines.
(define (inc n) (+ n 1))
(define (dec n) (- n 1))

(define (string-join strs sep)
  (if (null? strs)
      ""
      (let loop ((rest (cdr strs)) (acc (car strs)))
        (if (null? rest)
            acc
            (loop (cdr rest) (string-append acc sep (car rest)))))))

;; `->string` is what a converted `(string a b c)` needs: Janet's `string`
;; stringifies and concatenates, where R7RS `string` takes characters
;; (scene-language-survey, and plan §4.8's hazard table).
(define (->string x)
  (cond [(string? x) x]
        [(number? x) (number->string x)]
        [(symbol? x) (symbol->string x)]
        [(keyword? x) (string-append ":" (keyword->string x))]
        [(char? x) (string x)]
        [else (format "~a" x)]))

(define (string-concat . xs)
  (let loop ((xs xs) (acc ""))
    (if (null? xs) acc (loop (cdr xs) (string-append acc (->string (car xs)))))))

;; `assert`, which 11 corpus files use.
(define-syntax assert
  (syntax-rules ()
    [(_ test) (unless test (error "assertion failed:" 'test))]
    [(_ test msg) (unless test (error msg))]))

;; A growable vector, under the name a converted `array/push` reaches for. It
;; is the SAME vector type -- see prim.rs -- so there is nothing to convert
;; between, and `vector?` stays true of it.
(define (make-growable) (make-vector 0))

;; ── parameters and dynamic binding ──────────────────────────────────────
;;
;; `parameterize` wraps each value in its converter call *here*, in Scheme, so
;; that applying the converter is an ordinary call the machine already knows
;; how to do. `%parameterize` is the VM form underneath, and it restores the
;; old values both on a normal return and on an error unwind -- which is
;; `dynamic-wind`'s "after" half arriving early, because `parameterize` cannot
;; be correct without it.

;; R7RS: a parameter's initial value is `(converter init)`, so the converter
;; has to run at construction too -- not only on a later `parameterize`. The
;; raw constructor is `%make-parameter`, because applying the converter needs
;; the control stack and so has to happen here rather than in a primitive.
(define (make-parameter init . conv)
  (if (null? conv)
      (%make-parameter init)
      (let ([c (car conv)])
        (%make-parameter (c init) c))))

(define (%convert p v)
  (let ([c (%param-converter p)])
    (if c (c v) v)))

(define-syntax parameterize
  (syntax-rules ()
    [(_ ((p v) ...) body ...)
     (%parameterize ((p (%convert p v)) ...) body ...)]))

;; ── randomness ──────────────────────────────────────────────────────────
;;
;; The generator is a value behind a parameter, never a global, so a seed can
;; be scoped to one build:
;;
;;   (parameterize ([current-random-source (make-random-source 42)])
;;     (build-the-scene))
;;
;; which is ray's `:ray-rng` and why a `:seed` replays (survey §4).

(define current-random-source (make-parameter (make-random-source 20261012)))

;; Janet's spellings, for a converted scene.
(define (random) (random-real))
(define (random-int n) (random-integer n))

;; A fresh source from a seed, which is what `:seed` means.
(define (with-seed seed thunk)
  (parameterize ([current-random-source (make-random-source seed)])
    (thunk)))

;; ── exceptions ──────────────────────────────────────────────────────────
;;
;; `guard` is built on `%try`, which catches a condition and hands it back as
;; `(#f . condition)`. `with-exception-handler` is different in kind: its
;; handler runs *at the raise point*, before any unwinding, so it cannot be
;; built on a catch. It works instead by keeping the handler stack in a
;; parameter and having `raise` call the top of it — which means the whole of
;; it is ordinary Scheme, and `parameterize`'s unwind behaviour gives the
;; "handler is removed while it runs" rule for free.

(define %handlers (make-parameter '()))

(define (with-exception-handler handler thunk)
  (parameterize ([%handlers (cons handler (%handlers))])
    (thunk)))

;; `raise-continuable`: the handler's value becomes the value of the raise.
(define (raise-continuable obj)
  (let ([hs (%handlers)])
    (if (null? hs)
        (%raise obj)
        (parameterize ([%handlers (cdr hs)])
          ((car hs) obj)))))

;; `raise`: non-continuable, so if the handler returns, that is itself an
;; error -- R7RS says a secondary exception is raised in the same dynamic
;; environment as the handler.
(define (raise obj)
  (let ([hs (%handlers)])
    (if (null? hs)
        (%raise obj)
        (begin
          (parameterize ([%handlers (cdr hs)])
            ((car hs) obj))
          (%raise obj)))))

(define-syntax guard
  (syntax-rules ()
    [(_ (var clause ...) body ...)
     (let ([r (%try (begin body ...))])
       (if (car r)
           (cdr r)
           (let ([var (cdr r)])
             (%guard-clauses var clause ...))))]))

;; Like `cond`, except that falling off the end RE-RAISES rather than
;; returning #f -- a `guard` that matches nothing must not swallow the
;; condition.
(define-syntax %guard-clauses
  (syntax-rules (else =>)
    [(_ var) (raise var)]
    [(_ var (else body ...)) (begin body ...)]
    [(_ var (test => proc) rest ...)
     (let ([t test]) (if t (proc t) (%guard-clauses var rest ...)))]
    [(_ var (test) rest ...)
     (let ([t test]) (if t t (%guard-clauses var rest ...)))]
    [(_ var (test body ...) rest ...)
     (if test (begin body ...) (%guard-clauses var rest ...))]))

;; `error-message` is not R7RS; it is what a readout wants, and it works on a
;; raised object of any kind rather than only on an error object.
(define (error-message obj)
  (if (error-object? obj) (error-object-message obj) (format "~a" obj)))

;; ── the current ports ───────────────────────────────────────────────────
;;
;; Parameters, not globals, so output can be redirected for a dynamic extent:
;;
;;   (let ([buf (open-output-string)])
;;     (parameterize ([current-output-port buf]) (describe-the-scene))
;;     (get-output-string buf))
;;
;; which is what ray writes as `(with-dyns [:out buf] (snapshot))`.

(define current-output-port (make-parameter (%stdout-port)))
(define current-error-port (make-parameter (%stderr-port)))
(define current-input-port (make-parameter (open-input-string "")))

(define (call-with-port port proc)
  (let ([r (proc port)])
    (close-port port)
    r))

;; Not R7RS, and the single most useful thing a readout wants.
(define (with-output-to-string thunk)
  (let ([buf (open-output-string)])
    (parameterize ([current-output-port buf]) (thunk))
    (get-output-string buf)))

(define (read-string-all port)
  (let loop ((acc '()))
    (let ([c (read-char port)])
      (if (eof-object? c)
          (list->string (reverse acc))
          (loop (cons c acc))))))

;; ── multiple values ─────────────────────────────────────────────────────
;;
;; `call-with-values` is the primitive and everything here derives from it.
;; `%vals` collects a producer's values as a list, which is what makes the
;; binding forms expressible as macros.

(define (%vals-of thunk) (call-with-values thunk list))

(define-syntax %vals
  (syntax-rules ()
    [(_ expr) (call-with-values (lambda () expr) list)]))

;; `let*-values` is the nesting expansion, and nesting IS its semantics: each
;; expression may see the bindings of the ones before it.
(define-syntax let*-values
  (syntax-rules ()
    [(_ () body ...) (let () body ...)]
    [(_ ((formals expr) rest ...) body ...)
     (call-with-values (lambda () expr)
       (lambda formals (let*-values (rest ...) body ...)))]))

;; `let-values` is NOT that: all the expressions are evaluated before any of
;; the formals are bound. So each one's values are collected into a temporary
;; first, and only then are the formals bound. The temporary is spelled `t` at
;; every level and that is safe -- each recursive expansion renames it, which
;; is hygiene doing real work rather than decoration.
(define-syntax let-values
  (syntax-rules ()
    [(_ bindings body ...) (%lv () bindings body ...)]))

(define-syntax %lv
  (syntax-rules ()
    [(_ (acc ...) () body ...) (%lv-bind (acc ...) body ...)]
    [(_ (acc ...) ((formals expr) rest ...) body ...)
     (let ([t (%vals expr)])
       (%lv (acc ... (formals t)) (rest ...) body ...))]))

(define-syntax %lv-bind
  (syntax-rules ()
    [(_ () body ...) (let () body ...)]
    [(_ ((formals t) rest ...) body ...)
     (apply (lambda formals (%lv-bind (rest ...) body ...)) t)]))

;; `define-values` defines at the current level, so it cannot be a `lambda`
;; binding -- the names are defined first and then assigned.
(define-syntax define-values
  (syntax-rules ()
    [(_ (var ...) expr)
     (begin
       (define var #f) ...
       (%set-each! (var ...) (%vals expr)))]))

(define-syntax %set-each!
  (syntax-rules ()
    [(_ () vals) (if #f #f)]
    [(_ (v rest ...) vals)
     (let ([vs vals])
       (set! v (car vs))
       (%set-each! (rest ...) (cdr vs)))]))

;; SRFI-8, which reads better than let-values for the one-binding case and
;; which a model will reach for.
(define-syntax receive
  (syntax-rules ()
    [(_ formals expr body ...)
     (call-with-values (lambda () expr) (lambda formals body ...))]))

;; ── promises, case-lambda, and the last of the list/string vocabulary ───
;;
;; All of this is Scheme because none of it needs the machine. A promise is a
;; mutable cell plus a thunk, and `force` is an ordinary procedure -- no VM
;; support at all, which is the test of whether `delay` really is a macro.

(define (%make-promise-thunk thunk) (vector 'promise #f thunk))

(define-syntax delay
  (syntax-rules ()
    [(_ expr) (%make-promise-thunk (lambda () expr))]))

(define-syntax delay-force
  (syntax-rules ()
    [(_ expr) (%make-promise-thunk (lambda () (force expr)))]))

(define (make-promise v)
  (if (promise? v) v (vector 'promise #t v)))

(define (promise? x)
  (and (vector? x)
       (= (vector-length x) 3)
       (eq? (vector-ref x 0) 'promise)))

;; Memoising: a promise is forced at most once, which is the whole point.
(define (force p)
  (if (promise? p)
      (if (vector-ref p 1)
          (vector-ref p 2)
          (let ([v ((vector-ref p 2))])
            ;; Re-check: forcing may have forced this same promise.
            (if (vector-ref p 1)
                (vector-ref p 2)
                (begin
                  (vector-set! p 1 #t)
                  (vector-set! p 2 v)
                  v))))
      p))

;; `case-lambda` dispatches on the number of arguments. The proper-list
;; pattern has to come first, so a clause with a rest formal is the catch-all
;; it looks like.
(define-syntax case-lambda
  (syntax-rules ()
    [(_ clause ...) (lambda args (%cl args clause ...))]))

(define-syntax %cl
  (syntax-rules ()
    [(_ args) (error "case-lambda: no clause accepts this many arguments")]
    [(_ args ((p ...) body ...) rest ...)
     (if (= (length args) (length '(p ...)))
         (apply (lambda (p ...) body ...) args)
         (%cl args rest ...))]
    [(_ args (formals body ...) rest ...)
     (apply (lambda formals body ...) args)]))

(define (list-copy lst)
  (if (pair? lst)
      (cons (car lst) (list-copy (cdr lst)))
      lst))

(define (string-map f s . more)
  (if (null? more)
      (list->string (map f (string->list s)))
      (list->string (apply map f (string->list s) (map string->list more)))))

(define (string-for-each f s . more)
  (if (null? more)
      (for-each f (string->list s))
      (apply for-each f (string->list s) (map string->list more))))

(define (vector-for-each* f v) (for-each f (vector->list v)))

(define (bytevector-copy! to at from . range)
  (let* ([start (if (null? range) 0 (car range))]
         [end (if (or (null? range) (null? (cdr range)))
                  (bytevector-length from)
                  (cadr range))])
    (let loop ((i start))
      (if (< i end)
          (begin
            (bytevector-u8-set! to (+ at (- i start)) (bytevector-u8-ref from i))
            (loop (+ i 1)))
          (if #f #f)))))

(define (make-list n . fill)
  (let ([v (if (null? fill) #f (car fill))])
    (let loop ((i 0) (acc '()))
      (if (= i n) acc (loop (+ i 1) (cons v acc))))))

(define (read-string k port)
  (let loop ((i 0) (acc '()))
    (if (= i k)
        (list->string (reverse acc))
        (let ([c (read-char port)])
          (if (eof-object? c)
              (if (null? acc) c (list->string (reverse acc)))
              (loop (+ i 1) (cons c acc)))))))

;; `interaction-environment` is the only environment there is, so it is a
;; token rather than a structure -- enough for `(eval expr (interaction-
;; environment))`, which is what the suite and ray's 6 `eval` uses need.
(define (interaction-environment) 'interaction-environment)
(define (scheme-report-environment n) 'interaction-environment)
(define (null-environment n) 'interaction-environment)
