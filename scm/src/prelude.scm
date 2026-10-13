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

(define-syntax when
  (syntax-rules ()
    [(_ test body ...) (if test (begin body ...) #f)]))

(define-syntax unless
  (syntax-rules ()
    [(_ test body ...) (if test #f (begin body ...))]))

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
            #f
            (begin (f (car lst)) (loop (cdr lst)))))
      (let loop ((ls (cons lst more)))
        (if (any-null? ls)
            #f
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
