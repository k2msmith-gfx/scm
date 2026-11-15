;## 21-ray.scm — a Scheme file written for ray: a scene, or a part of one.
;## Its verbs (add-box, set-camera, render) live in ray's image, not in
;## scm's, so the modeline says so: "Scheme (ray)" (Kevin, 2026-11-15).
;## The modeler, where every Scheme file runs in ray's image, says it of
;## them all (its own detector, asked first).

(define *ray-verbs*
  ; Calls that only a ray scene makes. A part of a scene (Fallingwater's
  ; masonry-04.scm) may make none of them; it is found by where it lives.
  '("(set-camera " "(render " "(render)" "(load-scene " "(clear-scene)" "(set-sky "
    "(add-sphere " "(add-box " "(add-cylinder " "(add-plane " "(add-mesh " "(add-light "
    "(add-sun " "(make-material "))

(define (ray-checkout-file? path)
  "Is PATH inside a ray checkout — under a directory holding ray's prelude,
scripts/scm/prelude.scm?"
  (let loop ((dir (path-parent path)) (n 0))
    (and dir (< n 32)
         (or (file-exists? (string-append dir "/scripts/scm/prelude.scm"))
             (loop (path-parent dir) (+ n 1))))))

(define (path-parent path)
  "PATH's directory, or #f at the top."
  (let loop ((i (- (string-length path) 1)))
    (cond ((< i 1) #f)
          ((char=? (string-ref path i) #\/) (substring path 0 i))
          (else (loop (- i 1))))))

(define (ray-text? path text)
  "Is the Scheme file at PATH, holding TEXT, written for ray?"
  (or (any (lambda (v) (string-contains text v)) *ray-verbs*)
      (and path (ray-checkout-file? path))))

(add-dialect-detector! "ray" ray-text?)
