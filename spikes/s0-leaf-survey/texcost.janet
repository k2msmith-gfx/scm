### What does an authored texel function actually cost? Shapes copied from
### examples/still-life/fruit-bowl.janet so the body is representative.

(defn- hash2 [ix iy]
  (def s (* 43758.5453 (math/sin (+ (* ix 127.1) (* iy 311.7)))))
  (- s (math/floor s)))

(defn- fade [t] (* t t t (+ (* t (- (* t 6) 15)) 10)))

(defn- vnoise [x y]
  (def ix (math/floor x)) (def iy (math/floor y))
  (def fx (fade (- x ix))) (def fy (fade (- y iy)))
  (def a (hash2 ix iy))       (def b (hash2 (+ ix 1) iy))
  (def c (hash2 ix (+ iy 1))) (def d (hash2 (+ ix 1) (+ iy 1)))
  (def top (+ a (* (- b a) fx)))
  (def bot (+ c (* (- d c) fx)))
  (+ top (* (- bot top) fy)))

(defn- fbm [x y]
  (var sum 0) (var amp 0) (var freq 1) (var w 0.5)
  (repeat 4
    (+= sum (* w (vnoise (* x freq) (* y freq))))
    (+= amp w)
    (set freq (* freq 2))
    (set w (* w 0.5)))
  (/ sum amp))

(defn- body [u v]
  (def wander (* 0.45 (- (fbm (* 4 u) (* 3 v)) 0.5)))
  (var col (map (fn [r g] (+ r (* (- g r) (max 0 (min 1 wander)))))
                [0.79 0.69 0.15] [0.55 0.60 0.14]))
  (def pit (fbm (* 46 u) (* 30 v)))
  (def k (max 0 (* 2.4 (- pit 0.50))))
  (set col (map (fn [c p] (+ c (* (- p c) (min 1 k)))) col [0.62 0.50 0.07]))
  col)

(defn scene []
  (set-film 8 8) (set-samples 1)
  (set-camera :at [0 0 3] :look-at [0 0 0] :fov 40)
  (each n [128 256 512 1024 2048]
    (def t0 (os/clock))
    (make-texture n n body (string "t" n))
    (def dt (- (os/clock) t0))
    (printf "%5d x %-5d = %9d texels   %7.3f s   %6.0f ns/texel"
            n n (* n n) dt (/ (* dt 1e9) (* n n))))

  # And the field case, for comparison: field-bake at the doc's :res 96.
  (defn ripple-blob [x y z]
    (- (math/sqrt (+ (* x x) (* y y) (* z z)))
       (+ 0.78 (* 0.11 (math/sin (* 9 x)) (math/sin (* 7 y)) (math/sin (* 8 z))))))
  (each res [48 96 128]
    (def t0 (os/clock))
    (field-bake ripple-blob :bounds [[-1.2 -1.2 -1.2] [1.2 1.2 1.2]] :res res)
    (def dt (- (os/clock) t0))
    (printf "field-bake :res %-4d = %9d samples %7.3f s   %6.0f ns/sample"
            res (* res res res) dt (/ (* dt 1e9) (* res res res))))
  (add-sphere :at [0 0 0] :radius 1))
(scene)
