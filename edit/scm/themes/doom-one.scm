;; Doom One — Doom Emacs's default (hlissner/emacs-doom-themes,
;; doom-one-theme.el): bg #282c34, bg-alt #21242b, base0–8 the greys,
;; keywords blue, functions magenta, strings green, numbers orange.
(define-theme :doom-one "Doom Emacs's default dark theme"
  '((:editor-bg . #x282c34)           ; bg
    (:panel-bg . #x21242b)            ; bg-alt
    (:border-focused . #x3f444a)      ; base4
    (:status-bg . #x1c1f24)           ; base1, the mode line
    (:status-ink . #xbbc2cf)          ; fg
    (:text-ink . #xbbc2cf)
    (:text-dim . #x5b6268)            ; base5
    (:text-accent . #x51afef)         ; blue
    (:text-error . #xff6c6b)          ; red
    (:text-warning . #xecbe7b)        ; yellow
    (:popup-bg . #x2f343d)
    (:menu-ink . #xf5f7fa)
    (:menu-key-ink . #xcdd3dc)
    (:code-comment . #x5b6268)        ; base5
    (:code-string . #x98be65)         ; green
    (:code-keyword . #x51afef)        ; blue
    (:code-number . #xda8548)         ; orange
    (:code-call . #xc678dd)           ; magenta, a function's name
    (:code-paren . #x73797e)))        ; base6
