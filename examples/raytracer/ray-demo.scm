;;;; examples/raytracer/ray-demo.scm -- the demo scene for ray itself, from ray's
;;;; src/api/scheme/store_tests.rs DEMO_SCENE, less its two mesh spheres (raylisp has
;;;; none), at one sample per pixel.  ray --scene ray-demo.scm --out ray.png --batch
(set-film 1280 720)
(set-camera :at [0.0 0.0 1.6] :look-at [0.0 0.0 0.6] :fov 60.0)
(set-ambient [0.1 0.1 0.1])

(defshape key  (add-point-light [3.0 3.0 1.0]  [1.0 0.92 0.80]))
(defshape fill (add-point-light [-2.5 1.5 0.5] [0.30 0.40 0.55]))
(defshape spot (add-spot-light  [1.2 3.5 0.5] [1.2 -0.1 -2.3]
                                [0.9 0.95 1.3] 18.0 6.0))


(defshape sphere-blue
  (add-sphere [-2.45 -0.15 -2.6] 0.35
              (material :albedo [0.3 0.5 0.9] :specular [1.0 1.0 1.0]
                        :shininess 16.0)))
(defshape sphere-glass
  (add-sphere [-1.3 0.0 -2.1] 0.5
              (material :albedo [0.9 0.3 0.3] :specular [1.0 1.0 1.0]
                        :shininess 128.0 :transparency [0.9 0.9 0.9]
                        :ior 1.5 :abbe 15.0 :shader :phong)))
(defshape sphere-orange
  (add-sphere [-1.72 0.0 -3.31] 0.5 (material :albedo [1.0 0.65 0.1])))
(defshape sphere-green
  (add-sphere [1.2 -0.1 -2.3] 0.4
              (material :albedo [0.4 0.8 0.4] :specular [1.0 1.0 1.0]
                        :shininess 180.0 :shader :blinn-phong)))
(defshape sphere-mirror
  (add-sphere [0.0 0.3 -2.8] 0.8
              (material :albedo [0.05 0.05 0.05] :reflectivity [0.9 0.9 0.9])))
(defshape sphere-normal
  (add-sphere [2.5 -0.1 -2.4] 0.4
              (material :albedo [1.0 1.0 1.0] :shader :normal-as-color)))
(defshape ground (add-plane -0.5 (material :albedo [0.6 0.6 0.6])))
(defshape backdrop
  (add-rect [3.0 1.0 1.5] 90.0 [0.0 1.3 -5.0] (material :albedo [0.45 0.25 0.55])))
(defshape box
  (add-cuboid [0.35 0.25 0.25] [0.0 30.0 0.0] [1.5 -0.25 -1.3]
              (material :albedo [0.2 0.6 0.6] :specular [1.0 1.0 1.0]
                        :shininess 32.0)))
(set-samples 1)
