;## 00-base.scm — what the editor's shell needs before anything else
;## (docs/editor-crate-plan.md, E3a). Loaded first of `edit/scm/`, which the
;## host loads before its own prelude: the modeler after its vocabulary
;## baseline and before 00-data, so these are its names too.
;##
;## What the shell leans on from the image beneath it, besides scm's own:
;## `list-sort`, `string-split`, `string-prefix?`, `filter`, `append-map` (ray's
;## prelude, which the modeler installs first; a standalone host must supply
;## them — E4; `the_shell_runs_keys_on_a_bare_interpreter` holds the list) and
;## the kernel prims `%focused-pane` / `%typing-pane`.

;; Hash tables by their SRFI-125 names — the keymaps are tables, made as
;; this loads, before the host's own files import it.
(import (srfi 125))

(define (%env-ref name default)
  "The value NAME is bound to in the live environment, or DEFAULT when it is
unbound. How a forward reference reads a name a LATER file defines, and how
a key bound to a symbol finds the verb's CURRENT definition at press time.
A name bound to #f answers #f, as an unbound one does when DEFAULT is #f."
  (guard (e (#t default)) (eval name (interaction-environment))))

(define (%key-string k)
  "A NAME as a prim takes it: `:select-ink` and \"select-ink\" both answer
\"select-ink\", as Janet's (string k) accepted either. A symbol is its name."
  (cond ((keyword? k) (keyword->string k))
        ((symbol? k) (symbol->string k))
        (else k)))
