;## 20-rust.scm — Rust, the editor's first language that is not Scheme
;## (piece 1, Kevin 2026-11-14): a .rs file highlights and indents as Rust
;## (edit's language/rust.rs), with nothing running beside the editor.
;## Rust has no REPL, so a send goes nowhere and says so — it must not fall
;## through to this image, where Rust would be read as Scheme. Completion
;## offers the buffer's own words; eldoc and describe have nothing to say.
;## A language server (rust-analyzer) is a later, opt-in step
;## (docs/editor-crate-plan.md, E7).

(define (rust-path? path text)
  "Is the file at PATH a Rust source file?"
  (and path (string-suffix? ".rs" (string-downcase path))))

(add-language-detector! "rust" rust-path?)
(set! *non-scheme-languages* (cons "rust" *non-scheme-languages*))

(hash-table-set! *language-backends* "rust"
  (lambda ()
    (list (cons :name "nothing (Rust has no REPL)")
          (cons :eval (lambda (src)
                        (list (cons :status 'error)
                              (cons :message "rust: a send goes nowhere — run cargo in a terminal (M-x term)")
                              (cons :output ""))))
          (cons :complete (lambda (prefix) '()))
          (cons :doc (lambda (name) #f))
          (cons :signature (lambda (name) #f))
          (cons :definition-sites (lambda (name) '())))))

(define (rust-mode)
  "Treat this buffer as Rust — M-x rust-mode: Rust's highlighting and
indentation, and sends go nowhere. A .rs file is set so when opened."
  (%editor-set-language (editor-pane-id) "rust")
  (echo "rust mode"))
