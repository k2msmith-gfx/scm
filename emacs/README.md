# scm-mode for Emacs

A client of scm's REPL protocol ([docs/repl.md](../docs/repl.md)): edit
`.scm` files on top of the built-in `scheme-mode`, with a REPL in a window
and the subset of SLIME that gets used. Opening the first `.scm` file starts
`scm --serve`, connects, and shows the REPL.

| key | in a `.scm` buffer |
|---|---|
| `C-x C-e` | eval the sexp before point (value in the echo area and the REPL) |
| `C-c C-c`, `C-M-x` | eval the top-level form around point |
| `C-c C-r` | eval the region |
| `C-c C-k` | eval the whole buffer |
| `C-c C-z` | go to the REPL |
| `C-c C-d` | describe the symbol at point: signature, kind, docstring, value |
| `C-c C-b` | interrupt a runaway (out of band, on a second connection) |
| `C-c C-q` | stop the server |
| `TAB`, `M-x completion-at-point` | complete over the server's global names |

In the `*scm*` REPL: `RET` sends a form once the server says it is complete
and inserts a newline when it says more is coming; `M-p`/`M-n` walk the
history; `TAB` completes. Errors show the message and the backtrace. Eldoc
shows a procedure's signature as you type.

Highlighting is `scheme-mode`'s, plus scm's own spellings: `[…]` vectors,
`&opt`/`&named`/`&keys`, and `:keywords`.

## Doom Emacs

`packages.el`:

```elisp
(package! scm-mode :recipe (:local-repo "~/devel/scm/emacs" :files ("scm-mode.el")))
```

`config.el`:

```elisp
(use-package! scm-mode
  :mode "\\.scm\\'"
  :config
  (setq scm-program "~/devel/scm/target/release/scm"))   ; or `cargo install --path scm-cli`
```

Then `doom sync`. Leave Doom's `:lang scheme` module off, or geiser will
also claim `.scm` buffers. For evil, the bindings above are in the mode
map and work from normal and insert state; add `map!` entries under a
leader if you prefer.

`M-x scm-repl` starts or shows the REPL by hand; `M-x scm-start` restarts
the server.

## Checking it

```
SCM=../target/release/scm emacs --batch -l test-scm-mode.el
```

drives the real server from batch Emacs: eval, output, framing, completion,
describe, an error with its backtrace, string escaping both ways, the REPL
buffer, and an interrupt of a runaway.
