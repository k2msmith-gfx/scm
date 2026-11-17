;## 20-rust.scm — Rust, the editor's first language that is not Scheme.
;## A .rs file highlights and indents as Rust (edit's language/rust.rs,
;## piece 1, Kevin 2026-11-14), with nothing running beside the editor.
;## Rust has no REPL of its own, so a send from here goes nowhere and says
;## so — it must not fall through to this image, where Rust would be read as
;## Scheme. (26-rust-repl gives it one, evcxr, and takes the sends.)
;##
;## When rust-analyzer is on the PATH (rustup puts it there) it is the
;## language server (E7, edit's lsp.rs): it starts with the first .rs
;## buffer of a project and from then on errors are underlined as you type
;## (and from cargo check on save), TAB completes, M-. jumps, C-c C-d d
;## describes and eldoc gives the argument hint; the modeline says how it
;## is (starting, indexing, ready), and C-x k closes the file to it.
;## Nothing is configured.
;## Without it — or after M-x rust-analyzer-stop — completion offers the
;## buffer's own words and the rest says nothing.

(define *rust-analyzer-program* "rust-analyzer")   ; what serves Rust

(define (rust-path? path text)
  "Is the file at PATH a Rust source file?"
  (and path (string-suffix? ".rs" (string-downcase path))))

(add-language-detector! "rust" rust-path?)
(set! *non-scheme-languages* (cons "rust" *non-scheme-languages*))

; DESCRIBE WITHOUT A SERVER (Kevin, 2026-11-17: "if the function has a doc
; string, make describe work"). rust-analyzer answers for a file in a
; Cargo project once it is ready; a scratch .rs, or a project still being
; indexed, it does not. But the doc is in the text: the kernel reads an
; item's signature and the /// comment above it straight off the tokens
; (language/rust.rs), from this buffer as it stands — unsaved edits and all
; — then the other Rust files open, then the ones beside this one on disk.
(define (%rust-sources)
  ; Rust texts to look a name up in, nearest first — each a thunk, so a
  ; file is not read unless the ones before it had nothing to say.
  (let* ((eid (editor-pane-id))
         (here (%editor-active-buffer eid))
         (path (buffer-path here))
         (dir (and path (let loop ((i (- (string-length path) 1)))
                          (cond ((< i 1) #f)
                                ((char=? (string-ref path i) #\/) (substring path 0 i))
                                (else (loop (- i 1)))))))
         (rust? (lambda (p) (and (string? p) (string-suffix? ".rs" (string-downcase p)))))
         (open (filter (lambda (p) (and (rust? p) (not (equal? p path))))
                       (map (lambda (row) (buffer-path (car row))) (%editor-buffer-list eid))))
         (beside (if dir
                     (filter (lambda (p) (and (rust? p) (not (equal? p path)) (not (member p open))))
                             (map (lambda (name) (string-append dir "/" name))
                                  (list-sort string<? (or (guard (e (#t #f)) ((%env-ref '%os-dir (lambda (d) #f)) dir)) '()))))
                     '())))
    (cons (lambda () (%editor-text eid))
          (map (lambda (p) (lambda () (or (guard (e (#t #f)) (slurp p)) "")))
               (append open beside)))))

(define (rust-describe name)
  "(rust-describe NAME)  ->  (signature doc) or #f

What the Rust source at hand says of the item called NAME — a fn, struct,
enum, trait, type, const, static, mod or macro: its signature on one line
and its doc comment (\"\" when it has none). Looked for in the buffer on
screen, then the other Rust buffers' files, then the .rs files beside it."
  (and (string? name) (not (equal? name ""))
       (let loop ((sources (%rust-sources)))
         (and (pair? sources)
              (or (%rust-describe ((car sources)) name)
                  (loop (cdr sources)))))))

(define (rust-backend)
  ; A question about the point goes to rust-analyzer, by the pane's cursor
  ; — and, when it has no answer or its answer is about something else than
  ; the NAME asked for (describe on an argument asks for the call's head),
  ; to the source itself.
  (define (eid) (editor-pane-id))
  (define (hover name)
    ; rust-analyzer's (signature doc) when it is NAME's.
    (match (%lsp-hover (eid))
      ((sig doc) (and (string? sig) (string-contains sig name) (list sig doc)))
      (_ #f)))
  (define (described name) (or (hover name) (rust-describe name)))
  (list (cons :name "nothing (Rust has no REPL)")
        (cons :eval (lambda (src)
                      (list (cons :status 'error)
                            (cons :message "rust: a send goes nowhere — run cargo in a terminal (M-x term)")
                            (cons :output ""))))
        ; #f when there is no server to ask: completion falls back to the
        ; buffer's words.
        (cons :complete (lambda (prefix) (%lsp-complete (eid))))
        (cons :signature (lambda (name) (and-let* ((d (described name))) (car d))))
        (cons :doc (lambda (name)
                     (let ((d (described name)))
                       (cond ((not d) "")
                             ((equal? (string-trim-both (cadr d)) "")
                               (string-append (car d) "\n\n(no doc comment — a /// line above it is one)"))
                             (else (string-append (car d) "\n\n" (cadr d)))))))
        (cons :definition-sites (lambda (name) (or (%lsp-definition (eid)) '())))
        ; The argument hint: the server's, which arrives later; with no
        ; server to ask, the signature of the call the point is in, now.
        (cons :eldoc (lambda ()
                       (unless (positive? (%lsp-ask-hint (eid)))
                         (and-let* ((head (%editor-head (eid)))
                                    (d (rust-describe (%text head))))
                           (echo (car d))))))))

(hash-table-set! *language-backends* "rust" rust-backend)

; The argument hint arrives on the reader thread and wakes the host.
(set! *wake-hooks*
  (cons (lambda () (when-let ((hint (%lsp-take-hint))) (echo hint))) *wake-hooks*))

; rust-analyzer runs cargo check when a file is saved.
(set! *save-hooks* (cons (lambda (path) (%lsp-saved path)) *save-hooks*))

(define (rust-analyzer-start)
  "Use rust-analyzer for the Rust buffers again — M-x rust-analyzer-start,
after a stop, or to try again once it is installed. It starts with the next
.rs buffer drawn."
  (%lsp-register "rust" *rust-analyzer-program* '("Cargo.toml"))
  (echo *rust-analyzer-program* " serves the Rust buffers"))

(define (rust-analyzer-stop)
  "Stop rust-analyzer and use no language server for Rust — M-x
rust-analyzer-stop. M-x rust-analyzer-start brings it back."
  (%lsp-register "rust" "" '())
  (echo "rust-analyzer stopped"))

(define (rust-analyzer-status)
  "Say how rust-analyzer is for this buffer — starting, indexing, ready,
not on the PATH — M-x rust-analyzer-status."
  (echo (%lsp-status (editor-pane-id))))

(define (rust-mode)
  "Treat this buffer as Rust — M-x rust-mode: Rust's highlighting and
indentation, rust-analyzer when it is on the PATH, and sends go nowhere. A
.rs file is set so when opened."
  (%editor-set-language (editor-pane-id) "rust")
  (echo "rust mode"))

(%lsp-register "rust" *rust-analyzer-program* '("Cargo.toml"))
