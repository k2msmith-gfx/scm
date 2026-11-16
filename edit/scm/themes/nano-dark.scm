;; N Λ N O, dark (rougier/nano-emacs, nano-theme-dark.el): Nord's Polar
;; Night under Snow Storm, the same nine faces as nano-light.
(define-theme :nano-dark "NANO Emacs, dark: Nord, and almost no colour"
  '((:editor-bg . #x2e3440)           ; background, nord0
    (:panel-bg . #x3b4252)            ; highlight, nord1
    (:border-focused . #x677691)      ; faded
    (:status-bg . #x434c5e)           ; subtle, nord2
    (:status-ink . #xeceff4)          ; foreground, nord6
    (:text-ink . #xeceff4)
    (:text-dim . #x677691)            ; faded
    (:text-accent . #x81a1c1)         ; salient, nord9
    (:text-error . #xebcb8b)          ; critical
    (:text-warning . #xd08770)        ; popout, nord12
    (:popup-bg . #x3b4252)            ; highlight
    (:menu-ink . #xeceff4)
    (:menu-key-ink . #xd8dee9)        ; nord4
    (:code-comment . #x677691)        ; faded
    (:code-string . #xd08770)         ; popout
    (:code-keyword . #x81a1c1)        ; salient
    (:code-number . #x81a1c1)         ; salient (a constant)
    (:code-call . #xeceff4)           ; strong
    (:code-paren . #x677691)))        ; faded
