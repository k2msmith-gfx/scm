;## 26-rust-repl.scm — Rust's REPL (Kevin, 2026-11-17: "launch something
;## like evcxr and connect to it like with scm or guile and send rust
;## expressions, functions, buffer to it to run"). A Rust buffer's sends go
;## to evcxr, which the editor starts on the first one, as a Scheme file's
;## go to an scm and a Guile file's to a Guile (14-remote, 15-guile): the
;## same keys, the same transcript, the same REPL panel.
;##
;##   C-x C-e   the expression before the cursor      sq(3)   p.x   v.len()
;##   C-M-x     the statement or item around it        a whole fn, a let
;##   C-c C-r   the selection
;##   C-c C-b   the buffer — and its main(), when it has one
;##
;## evcxr keeps what it is sent: a `let` or a `fn` sent once is there for
;## the next send, and `:dep serde = "1"` — its own command, typed at the
;## REPL — adds a crate. It is not Cargo: a file that is one module of a
;## crate (`use crate::…`, `mod x;`) does not load into it whole, though
;## its functions do one at a time.
;##
;## evcxr has no socket and no protocol but its output: started with
;## --ide-mode it ends each answer with a mark (src/pipe.rs reads them), so
;## answers are structured — a value, or an error with rustc's report —
;## where a terminal attached with M-x term-attach would only show them.

(define *rust-repl-program* "evcxr")   ; what rust-repl-start runs
(define *rust-repl-args* '("--ide-mode" "--disable-readline"))
(define *rust-repl* #f)                ; the REPL's id, once started
; request -> (callback . the jiffy it was sent at)
(define *rust-repl-callbacks* (make-hash-table equal?))

; evcxr's framing (evcxr_repl's main, read 2026-11-17 and measured against
; 0.21.1): U+0091 ends an answer that succeeded, U+0092 one that failed, the
; prompt is ">> ", and U+2028 in a line it reads is a newline of the code.
(define %rust-ok (string (integer->char #x91)))
(define %rust-failed (string (integer->char #x92)))
(define %rust-newline (string (integer->char #x2028)))

(define (rust-repl-running?)
  (and *rust-repl* (%pipe-alive? *rust-repl*) #t))

(define (%rust-repl-dir)
  ; Where it runs: the buffer's project, so a relative path in the code
  ; means what it means to cargo run there.
  (let* ((eid (editor-pane-id))
         (path (and eid (buffer-path (%editor-active-buffer eid)))))
    (and path
         (let ((dir (let loop ((i (- (string-length path) 1)))
                      (cond ((< i 1) #f)
                            ((char=? (string-ref path i) #\/) (substring path 0 i))
                            (else (loop (- i 1)))))))
           (and dir (or (%git-root dir) dir))))))

(define (rust-repl-start)
  "Start evcxr for the Rust buffers' sends, or answer the one running —
M-x rust-repl-start. The first send from a Rust buffer starts it; this is
for starting it before then. It needs evcxr: `cargo install evcxr_repl`."
  (unless (rust-repl-running?)
    (let ((program (%pipe-which *rust-repl-program*)))
      (unless program
        (raise (string-append "no " *rust-repl-program* " — `cargo install evcxr_repl` gives Rust a REPL")))
      (set! *rust-repl*
        (%pipe-start program *rust-repl-args* (%rust-repl-dir) %rust-ok %rust-failed ">> "))
      (echo "evcxr running — the first send compiles, and takes a few seconds")))
  *rust-repl*)

(define (rust-repl-stop)
  "Stop the evcxr the editor started — M-x rust-repl-stop. Everything sent
to it is forgotten; the next send starts a fresh one."
  (if *rust-repl*
      (begin (%pipe-stop *rust-repl*)
             (set! *rust-repl* #f)
             (echo "evcxr stopped"))
      (echo "no evcxr running")))

(define (rust-repl-restart)
  "Stop evcxr and start a fresh one, with nothing defined — M-x
rust-repl-restart."
  (when *rust-repl* (%pipe-stop *rust-repl*) (set! *rust-repl* #f))
  (rust-repl-start))

(define (rust-repl-interrupt)
  "Stop what evcxr is running — a send that runs away — as Ctrl-C at its
own prompt would: M-x rust-repl-interrupt (and M-x remote-interrupt, in a
Rust buffer). evcxr stays up, with what was defined before."
  (if (and (rust-repl-running?) (%pipe-interrupt *rust-repl*))
      (echo "interrupted")
      (echo "evcxr is not running anything")))

(define (%rust-lines text)
  (filter (lambda (l) (not (equal? (string-trim-both l) ""))) (string-split text "\n")))

(define (rust-result src ok output secs)
  ; evcxr's answer as a backend RESULT (07-backend). It prints a value the
  ; way it prints anything, so the LAST line it wrote is taken for the
  ; value and what came before for output; a send that printed nothing
  ; answers what it defined — `fn area`, `let x` — as a Scheme define
  ; answers its name.
  (let ((lines (string-split (string-trim-right output) "\n")))
    (if ok
        (let* ((said (not (equal? (string-trim-both output) "")))
               (value (if said
                          (car (last-pair lines))
                          (%rust-defined-name src))))
          (list (cons :status 'ok)
                (cons :value value)
                (cons :printed (or value "(no value)"))
                (cons :output (if said (%agent-join (reverse (cdr (reverse lines))) "\n") ""))
                (cons :secs secs)))
        (list (cons :status 'error)
              ; rustc's report in the transcript, its first line as the
              ; message: "[E0425] Error: cannot find value `x` in this scope".
              (cons :message (let ((ls (%rust-lines output))) (if (null? ls) "evcxr gave no reason" (car ls))))
              (cons :output (let ((ls (%rust-lines output))) (if (or (null? ls) (null? (cdr ls))) "" output)))))))

(define (rust-repl-poll)
  ; Deliver evcxr's answers: each to the callback its send left here.
  (for-each
    (lambda (row)
      (let* ((request (cadr row))
             (waiting (hash-table-ref/default *rust-repl-callbacks* request #f)))
        (hash-table-delete! *rust-repl-callbacks* request)
        (when waiting
          ((car waiting) (caddr row) (cadddr row)
                         (/ (- (current-jiffy) (cdr waiting)) (jiffies-per-second))))))
    (%pipe-poll)))

(set! *wake-hooks* (cons rust-repl-poll *wake-hooks*))

(define (%rust-runs-main src)
  ; SRC, and a call of its main() when it defines one: sending a program
  ; RUNS it, as C-c C-b on a Scheme file runs the file.
  (if (any (lambda (span) (equal? (%rust-defined-name (substring src (car span) (cadr span))) "fn main"))
           (%rust-statements src))
      ; (As a statement: main's () is not a value worth a line.)
      (string-append src "\nmain();")
      src))

(define (rust-repl-send src k)
  "Send Rust SRC to evcxr, starting it if need be, and call K with its
RESULT when it answers. Returns at once."
  (let* ((id (rust-repl-start))
         ; One line: evcxr reads a line, and U+2028 is its newline.
         (line (%agent-join (string-split (%rust-runs-main src) "\n") %rust-newline))
         (request (%pipe-send id line)))
    (hash-table-set! *rust-repl-callbacks* request
                     (cons (lambda (ok output secs) (k (rust-result src ok output secs)))
                           (current-jiffy)))
    request))

(define (rust-repl-backend)
  "A Rust buffer's backend: sends go to evcxr, started on the first one;
completion, describe and M-. stay rust-analyzer's (20-rust). With no evcxr
installed a send FAILS, saying how to get one — it must not fall through to
this image, where Rust would be read as Scheme."
  (let ((lsp (filter (lambda (op) (not (memq (car op) '(:name :eval)))) (rust-backend))))
    (if (or (rust-repl-running?) (%pipe-which *rust-repl-program*))
        (append
          (list (cons :name "evcxr")
                (cons :eval-async
                      (lambda (src k)
                        (guard (e (#t (k (list (cons :status 'error)
                                               (cons :message (string-append "evcxr: " (if (string? e) e (*show-value* e))))
                                               (cons :output "")))))
                          (rust-repl-send src k))))
                ; (Nothing here waits: a first send compiles for seconds.)
                (cons :eval
                      (lambda (src)
                        (list (cons :status 'error)
                              (cons :message "evcxr answers later — this command needs a value now")
                              (cons :output ""))))
                (cons :interrupt rust-repl-interrupt)
                (cons :whole? %rust-complete?))
          lsp)
        (append
          (list (cons :name "nothing (no evcxr)")
                (cons :eval
                      (lambda (src)
                        (list (cons :status 'error)
                              (cons :message
                                    (string-append "rust: a send goes nowhere without a REPL — `cargo install evcxr_repl` gives Rust one"
                                                   " (evcxr), or run cargo in a terminal (M-x term)"))
                              (cons :output "")))))
          lsp))))

(hash-table-set! *language-backends* "rust" rust-repl-backend)

; What a send reads from a Rust buffer: the statement or item at the point,
; and the expression before it (language/rust.rs, off the tokens).
(define-language-forms! "rust"
  (lambda (text)
    (let ((at (%editor-cursor (editor-pane-id))))
      (%rust-statement-at text (car at) (cadr at))))
  (lambda (text)
    (let ((at (%editor-cursor (editor-pane-id))))
      (%rust-expression-before text (car at) (cadr at)))))
