;; Dracula (draculatheme.com) on its darkest backgrounds: the buffer on
;; "BGDarker" #191a21 and the REPL on "BGDark" #21222c, the near-black
;; Dracula's own VS Code sidebars use, rather than the usual #282a36.
;; Keywords pink, strings yellow, calls green, numbers purple.
(define-theme :dracula "Dracula, on its near-black backgrounds"
  '((:editor-bg . #x191a21)           ; BGDarker
    (:panel-bg . #x21222c)            ; BGDark
    (:border-focused . #x44475a)      ; current line
    (:status-bg . #x282a36)           ; background
    (:status-ink . #xf8f8f2)          ; foreground
    (:text-ink . #xf8f8f2)
    (:text-dim . #x6272a4)            ; comment
    (:text-accent . #xbd93f9)         ; purple
    (:text-error . #xff5555)          ; red
    (:text-warning . #xffb86c)        ; orange
    (:popup-bg . #x343746)            ; BGLight
    (:menu-ink . #xf8f8f2)
    (:menu-key-ink . #xbfc1d0)
    (:code-comment . #x6272a4)        ; comment
    (:code-string . #xf1fa8c)         ; yellow
    (:code-keyword . #xff79c6)        ; pink
    (:code-number . #xbd93f9)         ; purple
    (:code-call . #x50fa7b)           ; green
    (:code-paren . #x7e83a3)))
