### Time ONLY fruit-bowl's scene build (textures included), no render.
(def t0 (os/clock))
(dofile "examples/still-life/fruit-bowl.janet" :env (curenv))
(printf "fruit-bowl scene build total: %.2f s" (- (os/clock) t0))
