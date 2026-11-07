;;;; examples/raytracer/leaves.scm -- the ray/shape tests as leaf-compiled code
;;;;
;;;; Loaded after raytracer.scm by scm-llvm/examples/raytrace_hybrid.rs. The
;;;; leaf compiler takes only pure arithmetic on f64s -- no vectors built, no
;;;; records, booleans only in tests -- so the scene's shapes are flattened
;;;; into SCENE-TABLE, a vector of numbers it snapshots as a constant array:
;;;; per shape a geometry code and the first three rows of local-from-world.
;;;;
;;;;   (nearest-shape ox oy oz dx dy dz)  index of the closest hit, or -1
;;;;   (occluded ox oy oz dx dy dz)       1.0 if anything lies in (1e-3, 1)
;;;;
;;;; Everything else -- the hit on the winning shape, shading, recursion,
;;;; glass, output -- stays in Scheme, which reaches the two through host
;;;; functions and redefines scene-intersect and scene-occluded? to use them.
;;;; The arithmetic is raylisp's, in the same order as raytracer.scm, so the
;;;; image comes out the same.

(define stride 13)

;; Column-major indices of rows 0-2 of a mat4: m00 m01 m02 m03, m10 ..., m20 ...
(define row-major-3x4 #(0 4 8 12 1 5 9 13 2 6 10 14))

(define (scene->table scene)
  (let* ((shapes (scene-shapes scene))
         (table (make-vector (* stride (length shapes)) 0.0)))
    (let loop ((s shapes) (base 0))
      (if (null? s)
          table
          (let ((m (shape-local-from-world (car s))))
            (vector-set! table base
                         (case (shape-geometry (car s))
                           ((sphere) 0.0) ((plane) 1.0) ((rect) 2.0) (else 3.0)))
            (do ((k 0 (+ k 1))) ((= k 12))
              (vector-set! table (+ base 1 k) (vector-ref m (vector-ref row-major-3x4 k))))
            (loop (cdr s) (+ base stride)))))))

;; The table the leaves snapshot. Shapes do not depend on the aspect ratio.
(define scene-table (scene->table (demo-scene 1.0)))
(define shape-count (/ (vector-length scene-table) stride))

;;; --- per-geometry, in local space: the nearest forward t, or 1e300 ----------

(define (sphere-t ox oy oz dx dy dz)
  (let* ((a (+ (* dx dx) (* dy dy) (* dz dz)))
         (h (+ (* ox dx) (* oy dy) (* oz dz)))
         (c (- (+ (* ox ox) (* oy oy) (* oz oz)) 1.0))
         (disc (- (* h h) (* a c))))
    (if (>= disc 0.0)
        (let* ((sq (sqrt disc))
               (t0 (/ (- (- h) sq) a))
               (t (if (<= t0 0.0) (/ (+ (- h) sq) a) t0)))
          (if (> t 0.0) t 1e300))
        1e300)))

(define (plane-t ox oy oz dx dy dz)
  (if (>= (abs dy) 1e-6)
      (let ((t (/ (- oy) dy)))
        (if (> t 0.0) t 1e300))
      1e300))

(define (rect-t ox oy oz dx dy dz)
  (if (>= (abs dy) 1e-6)
      (let ((t (/ (- oy) dy)))
        (if (and (> t 0.0)
                 (<= (abs (+ ox (* dx t))) 1.0)
                 (<= (abs (+ oz (* dz t))) 1.0))
            t
            1e300))
      1e300))

;; The slab test, unrolled: no vectors to index. Each axis narrows
;; [enter, exit] exactly as raytracer.scm's loop does, NaN included.
(define (slab-lo o d enter)
  (let* ((inv (/ 1.0 d)) (a (* (- -1.0 o) inv)) (b (* (- 1.0 o) inv))
         (t0 (if (< inv 0.0) b a)))
    (if (> t0 enter) t0 enter)))
(define (slab-hi o d exit)
  (let* ((inv (/ 1.0 d)) (a (* (- -1.0 o) inv)) (b (* (- 1.0 o) inv))
         (t1 (if (< inv 0.0) a b)))
    (if (< t1 exit) t1 exit)))

(define (cuboid-t ox oy oz dx dy dz)
  (let* ((enter (slab-lo oz dz (slab-lo oy dy (slab-lo ox dx -1e300))))
         (exit (slab-hi oz dz (slab-hi oy dy (slab-hi ox dx 1e300)))))
    (if (and (> exit enter) (> exit 0.0))
        (if (<= enter 0.0) exit enter)
        1e300)))

;;; --- per-geometry occlusion over (1e-3, 1), as raytracer.scm tests it ------

;; A macro, not a procedure: a leaf may test a boolean but not return one.
(define-syntax in-range
  (syntax-rules () ((_ t) (let ((u t)) (and (> u 1e-3) (< u 1.0))))))

(define (sphere-blocks ox oy oz dx dy dz)
  (let* ((a (+ (* dx dx) (* dy dy) (* dz dz)))
         (h (+ (* ox dx) (* oy dy) (* oz dz)))
         (c (- (+ (* ox ox) (* oy oy) (* oz oz)) 1.0))
         (disc (- (* h h) (* a c))))
    (if (>= disc 0.0)
        (let ((sq (sqrt disc)))
          ;; Nested ifs, not `or`: its expansion binds a boolean to a variable.
          (if (in-range (/ (- (- h) sq) a))
              1.0
              (if (in-range (/ (+ (- h) sq) a)) 1.0 0.0)))
        0.0)))

(define (plane-blocks ox oy oz dx dy dz)
  (if (and (>= (abs dy) 1e-6) (in-range (/ (- oy) dy))) 1.0 0.0))

(define (rect-blocks ox oy oz dx dy dz)
  (if (>= (abs dy) 1e-6)
      (let ((t (/ (- oy) dy)))
        (if (and (in-range t)
                 (<= (abs (+ ox (* dx t))) 1.0)
                 (<= (abs (+ oz (* dz t))) 1.0))
            1.0 0.0))
      0.0))

(define (cuboid-blocks ox oy oz dx dy dz)
  (let* ((enter (slab-lo oz dz (slab-lo oy dy (slab-lo ox dx -1e300))))
         (exit (slab-hi oz dz (slab-hi oy dy (slab-hi ox dx 1e300)))))
    (if (and (> exit enter) (< enter 1.0) (> exit 1e-3)) 1.0 0.0)))

;;; --- over the scene -----------------------------------------------------------

;; Row R of shape I's local-from-world applied to (x y z w).
(define (row i r x y z w)
  (let ((b (+ (* i stride) 1 (* r 4))))
    (+ (* (vector-ref scene-table b) x) (* (vector-ref scene-table (+ b 1)) y)
       (* (vector-ref scene-table (+ b 2)) z) (* (vector-ref scene-table (+ b 3)) w))))

(define (local-t i ox oy oz dx dy dz)
  (let ((code (vector-ref scene-table (* i stride)))
        (lox (row i 0 ox oy oz 1.0)) (loy (row i 1 ox oy oz 1.0)) (loz (row i 2 ox oy oz 1.0))
        (ldx (row i 0 dx dy dz 0.0)) (ldy (row i 1 dx dy dz 0.0)) (ldz (row i 2 dx dy dz 0.0)))
    (cond ((= code 0.0) (sphere-t lox loy loz ldx ldy ldz))
          ((= code 1.0) (plane-t lox loy loz ldx ldy ldz))
          ((= code 2.0) (rect-t lox loy loz ldx ldy ldz))
          (else (cuboid-t lox loy loz ldx ldy ldz)))))

(define (local-blocks i ox oy oz dx dy dz)
  (let ((code (vector-ref scene-table (* i stride)))
        (lox (row i 0 ox oy oz 1.0)) (loy (row i 1 ox oy oz 1.0)) (loz (row i 2 ox oy oz 1.0))
        (ldx (row i 0 dx dy dz 0.0)) (ldy (row i 1 dx dy dz 0.0)) (ldz (row i 2 dx dy dz 0.0)))
    (cond ((= code 0.0) (sphere-blocks lox loy loz ldx ldy ldz))
          ((= code 1.0) (plane-blocks lox loy loz ldx ldy ldz))
          ((= code 2.0) (rect-blocks lox loy loz ldx ldy ldz))
          (else (cuboid-blocks lox loy loz ldx ldy ldz)))))

;; Strictly nearer wins, so the first of two equal hits is kept.
(define (nearest-shape ox oy oz dx dy dz)
  (let loop ((i 0) (best-t 1e300) (best -1.0))
    (if (< i shape-count)
        (let ((t (local-t i ox oy oz dx dy dz)))
          (if (< t best-t)
              (loop (+ i 1) t i)
              (loop (+ i 1) best-t best)))
        best)))

(define (occluded ox oy oz dx dy dz)
  (let loop ((i 0))
    (if (< i shape-count)
        (if (> (local-blocks i ox oy oz dx dy dz) 0.5) 1.0 (loop (+ i 1)))
        0.0)))

;;; --- Scheme's side: the same two procedures, through the native ones ---------

(define (scene-intersect scene ray)
  (let* ((o (ray-origin ray)) (d (ray-direction ray))
         (i (native-nearest (vec3-x o) (vec3-y o) (vec3-z o)
                            (vec3-x d) (vec3-y d) (vec3-z d))))
    (and (>= i 0)
         (let* ((s (list-ref (scene-shapes scene) (exact i)))
                (lh (shape-hit-local s ray)))
           (shape-finish-hit s lh)))))

;; Only shade-lit calls this, always over (1e-3, 1); the leaf has those built in.
(define (scene-occluded? scene ray t-min t-max)
  (let ((o (ray-origin ray)) (d (ray-direction ray)))
    (> (native-occluded (vec3-x o) (vec3-y o) (vec3-z o)
                        (vec3-x d) (vec3-y d) (vec3-z d))
       0.5)))
