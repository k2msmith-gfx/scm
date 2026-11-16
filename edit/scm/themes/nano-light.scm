;; N Λ N O (rougier/nano-emacs, nano-theme-light.el): white, and nine
;; faces where most themes have thirty. Syntax is nearly monochrome on
;; purpose — keywords and constants "salient", strings "popout", comments
;; "faded", a definition's name "strong". The mode line is the "subtle"
;; band NANO puts in its header line.
(define-theme :nano-light "NANO Emacs, light: white and almost no colour"
  '((:editor-bg . #xffffff)           ; background
    (:panel-bg . #xfafafa)            ; highlight
    (:border-focused . #xb0bec5)      ; faded
    (:status-bg . #xeceff1)           ; subtle
    (:status-ink . #x37474f)          ; foreground
    (:text-ink . #x37474f)
    (:text-dim . #xb0bec5)            ; faded
    (:text-accent . #x673ab7)         ; salient
    (:text-error . #xff6f00)          ; critical
    (:text-warning . #xffab91)        ; popout
    (:popup-bg . #xeceff1)            ; subtle
    (:menu-ink . #x37474f)
    (:menu-key-ink . #x90a4ae)
    (:code-comment . #xb0bec5)        ; faded
    (:code-string . #xffab91)         ; popout
    (:code-keyword . #x673ab7)        ; salient
    (:code-number . #x673ab7)         ; salient (a constant)
    (:code-call . #x000000)           ; strong
    (:code-paren . #xb0bec5)))        ; faded
