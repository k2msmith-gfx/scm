;;; serve.scm — a Guile REPL server speaking scm's REPL protocol
;;; (docs/editor-crate-plan.md, E6; the protocol is ~/devel/scm/docs/repl.md).
;;;
;;;     guile --no-auto-compile -s serve.scm 127.0.0.1:PORT
;;;
;;; The editor's socket backend (edit's scm_binding/remote.rs, E5b) is a
;;; client of `scm --serve`. Answering the same requests here makes Guile a
;;; backend without a second client: a send evaluates in THIS Guile and
;;; answers as an scm one does — the value written, what the forms printed,
;;; or an error's message — and completion and describe read this image.
;;;
;;; A request is one datum, `(:op eval :id 7 :code "…")`; Guile's reader
;;; takes `:op` as a symbol, which is all this needs of it. The answer is one
;;; datum too, written by hand in the wire's own spelling (`:status ok`).
;;; Each connection is served on its own thread, so `:interrupt` from a
;;; second connection reaches an evaluation running on the first — the
;;; protocol's rule, because an interrupt queued behind the form it is meant
;;; to stop would never arrive.

(use-modules (system base compile)
             (ice-9 rdelim)
             (ice-9 regex)
             (ice-9 session)
             (ice-9 threads)
             (ice-9 documentation)
             (srfi srfi-1))

(define user (resolve-module '(guile-user)))

;; The thread evaluating right now, for :interrupt.
(define *evaluating* #f)
(define *evaluating-lock* (make-mutex))

(define (field req key)
  ;; KEY's value in the request, a plist of :key value.
  (let loop ((r req))
    (cond ((or (null? r) (null? (cdr r))) #f)
          ((eq? (car r) key) (cadr r))
          (else (loop (cddr r))))))

(define (wire-string s) (with-output-to-string (lambda () (write s))))

(define (written v) (with-output-to-string (lambda () (write v))))

(define (error-text key args)
  ;; An exception as one line, the way Guile's REPL says it.
  (cond ((and (list? args) (= (length args) 4) (string? (cadr args)))
         (let ((subr (car args)) (fmt (cadr args)) (fargs (caddr args)))
           (string-append (if subr (format #f "~a: " subr) "")
                          (apply format #f fmt (or fargs '())))))
        (else (format #f "~a: ~s" key args))))

(define (eval-code code)
  ;; Every form in CODE, in order, in guile-user; the last one's value.
  ;; COMPILED, not interpreted: a compiled procedure keeps its parameters'
  ;; names, which describe's signature shows ((sq x), not (sq a)).
  (let ((port (open-input-string code)))
    (let loop ((last (if #f #f)))
      (let ((form (read port)))
        (if (eof-object? form)
            last
            (loop (compile form #:env user)))))))

(define (answer-eval id code)
  (let* ((out (open-output-string))
         (result
           (catch #t
             (lambda ()
               (with-mutex *evaluating-lock* (set! *evaluating* (current-thread)))
               (let ((v (with-output-to-port out (lambda () (eval-code code)))))
                 (list 'ok v)))
             (lambda (key . args) (list 'error key args)))))
    (with-mutex *evaluating-lock* (set! *evaluating* #f))
    (case (car result)
      ((ok)
       (format #f "(:id ~a :status ok :value ~a :output ~a)"
               id (wire-string (written (cadr result))) (wire-string (get-output-string out))))
      (else
       (let ((key (cadr result)) (args (caddr result)))
         (format #f "(:id ~a :status error :message ~a :output ~a :host-stopped ~a :backtrace ())"
                 id
                 (wire-string (if (eq? key 'interrupted) "interrupted" (error-text key args)))
                 (wire-string (get-output-string out))
                 (if (eq? key 'interrupted) "#t" "#f")))))))

(define (answer-complete id prefix)
  (let ((names (sort (delete-duplicates
                       (map symbol->string
                            (apropos-internal (string-append "^" (regexp-quote prefix)))))
                     string<?)))
    (format #f "(:id ~a :status ok :candidates (~a))"
            id (string-join (map wire-string names) " "))))

(define (signature-of name proc)
  ;; "(f a b #:key c)" from the procedure's arguments, when Guile knows them.
  (let ((args (false-if-exception (procedure-arguments proc))))
    (and args
         (let ((req (or (assq-ref args 'required) '()))
               (opt (or (assq-ref args 'optional) '()))
               (key (or (assq-ref args 'keyword) '()))
               (rest (assq-ref args 'rest)))
           (string-append
             "(" name
             (apply string-append
                    (append (map (lambda (a) (string-append " " (symbol->string a))) req)
                            (if (null? opt) '() (cons " #:optional" (map (lambda (a) (string-append " " (symbol->string a))) opt)))
                            (if (null? key) '() (cons " #:key" (map (lambda (k) (string-append " " (symbol->string (keyword->symbol (car k))))) key)))
                            (if rest (list " . " (symbol->string rest)) '())))
             ")")))))

(define (answer-describe id name)
  (let* ((sym (string->symbol name))
         (var (module-variable user sym)))
    (if (not (and var (variable-bound? var)))
        (format #f "(:id ~a :status ok :bound #f)" id)
        (let* ((v (variable-ref var))
               (proc? (procedure? v))
               (sig (and proc? (signature-of name v)))
               (doc (or (and proc? (procedure-documentation v)) (object-documentation v))))
          (format #f "(:id ~a :status ok :bound #t :name ~a :kind ~a :value ~a~a~a)"
                  id (wire-string name)
                  (wire-string (if proc? "procedure" "variable"))
                  (wire-string (written v))
                  (if sig (string-append " :signature " (wire-string sig)) "")
                  (if (string? doc) (string-append " :doc " (wire-string doc)) ""))))))

(define (answer-parse id code)
  (format #f "(:id ~a :status ok :framing ~a)" id
          (catch #t
            (lambda ()
              (let ((p (open-input-string code)))
                (let loop () (if (eof-object? (read p)) "complete" (loop)))))
            (lambda (key . args)
              ;; Guile's reader raises read-error for both; an unterminated
              ;; form ends at the end of the text.
              (if (string-contains (error-text key args) "end of input") "incomplete" "invalid")))))

(define (interrupt!)
  (with-mutex *evaluating-lock*
    (when *evaluating*
      (system-async-mark (lambda () (throw 'interrupted)) *evaluating*))))

(define (answer req)
  (let ((id (or (field req ':id) 0))
        (op (let ((o (field req ':op))) (if (string? o) (string->symbol o) o))))
    (case op
      ((eval) (answer-eval id (or (field req ':code) "")))
      ((complete) (answer-complete id (or (field req ':prefix) "")))
      ((describe) (answer-describe id (let ((n (field req ':name))) (if (symbol? n) (symbol->string n) (or n "")))))
      ((parse) (answer-parse id (or (field req ':code) "")))
      ((interrupt) (interrupt!) (format #f "(:id ~a :status ok :version ~a)" id (wire-string (version))))
      ((ping stream) (format #f "(:id ~a :status ok :version ~a)" id (wire-string (string-append "guile " (version)))))
      (else (format #f "(:id ~a :status malformed :message ~a)" id
                    (wire-string (format #f "unknown op `~a`" op)))))))

(define (serve-connection port)
  (let loop ()
    (let ((req (catch #t (lambda () (read port)) (lambda _ (eof-object)))))
      (unless (eof-object? req)
        ;; A bug here answers malformed rather than killing the connection
        ;; — under the request's own :id, which the client is waiting for.
        (display (catch #t
                   (lambda () (answer req))
                   (lambda (key . args)
                     (format #f "(:id ~a :status malformed :message ~a)"
                             (or (false-if-exception (field req ':id)) 0)
                             (wire-string (error-text key args)))))
                 port)
        (newline port)
        (force-output port)
        (loop))))
  (close-port port))

(define (main addr)
  (let* ((colon (string-rindex addr #\:))
         (host (substring addr 0 colon))
         (port-no (string->number (substring addr (+ colon 1))))
         (sock (socket PF_INET SOCK_STREAM 0)))
    (setsockopt sock SOL_SOCKET SO_REUSEADDR 1)
    (bind sock AF_INET (inet-pton AF_INET host) port-no)
    (listen sock 8)
    (let loop ()
      (let ((conn (car (accept sock))))
        (call-with-new-thread (lambda () (serve-connection conn)))
        (loop)))))

(main (cadr (command-line)))
