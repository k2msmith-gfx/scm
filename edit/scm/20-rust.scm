;## 20-rust.scm — Rust, the editor's first language that is not Scheme.
;## A .rs file highlights and indents as Rust (edit's language/rust.rs,
;## piece 1, Kevin 2026-11-14), with nothing running beside the editor.
;## Rust has no REPL, so a send goes nowhere and says so — it must not fall
;## through to this image, where Rust would be read as Scheme.
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

(define (rust-backend)
  ; Every question about the point goes to rust-analyzer, by the pane's
  ; cursor — the NAME the shell passes is Scheme's idea of the word there.
  (define (eid) (editor-pane-id))
  (list (cons :name "nothing (Rust has no REPL)")
        (cons :eval (lambda (src)
                      (list (cons :status 'error)
                            (cons :message "rust: a send goes nowhere — run cargo in a terminal (M-x term)")
                            (cons :output ""))))
        ; #f when there is no server to ask: completion falls back to the
        ; buffer's words.
        (cons :complete (lambda (prefix) (%lsp-complete (eid))))
        (cons :signature (lambda (name) (match (%lsp-hover (eid)) ((sig doc) sig) (_ #f))))
        (cons :doc (lambda (name) (match (%lsp-hover (eid)) ((sig doc) doc) (_ #f))))
        (cons :definition-sites (lambda (name) (or (%lsp-definition (eid)) '())))
        (cons :eldoc (lambda () (%lsp-ask-hint (eid))))))

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
