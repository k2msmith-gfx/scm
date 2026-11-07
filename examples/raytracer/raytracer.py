"""examples/raytracer/raytracer.py -- raytracer.scm in CPython, for calibration.

The same Whitted tracer, scene and arithmetic as raytracer.scm, written the
way a Python programmer would for speed: tuples for vec3, rays and hits (the
counterpart of the Scheme's vectors), __slots__ classes for what is built once,
and plain functions. Floats are f64 on both sides and the operations run in
the same order, so the image is the same file.

    python3 examples/raytracer/raytracer.py [--size WxH] [--stripe K/N] [--out FILE]

Flags and output match raytracer.scm, so bench.sh drives it too:

    WORKER="python3 examples/raytracer/raytracer.py" examples/raytracer/bench.sh

One difference Python forces: float division by zero raises, where Scheme
gives an infinity. The slab test, the only place a zero divisor is expected,
spells the infinity out.
"""
import math
import sys
import time

INF = math.inf


def deg(d):
    return d * (math.pi / 180.0)


# --- vec3 ---------------------------------------------------------------------

def v_add(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def v_sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def v_mul(a, s):
    return (a[0] * s, a[1] * s, a[2] * s)


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0])


def normalize(a):
    return v_mul(a, 1.0 / math.sqrt(dot(a, a)))


def normalize_or_zero(a):
    length = math.sqrt(dot(a, a))
    return (0.0, 0.0, 0.0) if length < 1e-30 else v_mul(a, 1.0 / length)


def reflect(i, n):
    return v_sub(i, v_mul(n, 2.0 * dot(i, n)))


def refract(i, n, eta):
    n_dot_i = dot(n, i)
    k = 1.0 - eta * eta * (1.0 - n_dot_i * n_dot_i)
    if k >= 0.0:
        return v_sub(v_mul(i, eta), v_mul(n, eta * n_dot_i + math.sqrt(k)))
    return (0.0, 0.0, 0.0)


BLACK = (0.0, 0.0, 0.0)


def color_zero(c):
    return c[0] == 0.0 and c[1] == 0.0 and c[2] == 0.0


# A ray is (origin, direction), a local hit (t, point, normal), a hit
# (t, point, normal, material). The direction is not normalised.

# --- mat4: 16 floats, column-major (row i, column j at i + 4j) ---------------

def mat4(*rows):
    m = [0.0] * 16
    for i in range(4):
        for j in range(4):
            m[i + 4 * j] = float(rows[4 * i + j])
    return m


def mref(m, i, j):
    return m[i + 4 * j]


def mat4_from_translation(v):
    return mat4(1, 0, 0, v[0], 0, 1, 0, v[1], 0, 0, 1, v[2], 0, 0, 0, 1)


def mat4_from_scale(v):
    return mat4(v[0], 0, 0, 0, 0, v[1], 0, 0, 0, 0, v[2], 0, 0, 0, 0, 1)


def quat_from_axis_angle(axis, angle):
    n = normalize(axis)
    s = math.sin(0.5 * angle)
    return (n[0] * s, n[1] * s, n[2] * s, math.cos(0.5 * angle))


def mat4_from_quat(q):
    x, y, z, w = q
    xx, yy, zz = x * x, y * y, z * z
    xy, xz, yz = x * y, x * z, y * z
    wx, wy, wz = w * x, w * y, w * z
    return mat4(1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy), 0.0,
                2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx), 0.0,
                2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy), 0.0,
                0.0, 0.0, 0.0, 1.0)


def mat4_mul(a, b):
    out = [0.0] * 16
    for i in range(4):
        for j in range(4):
            out[i + 4 * j] = (mref(a, i, 0) * mref(b, 0, j) + mref(a, i, 1) * mref(b, 1, j)
                              + mref(a, i, 2) * mref(b, 2, j) + mref(a, i, 3) * mref(b, 3, j))
    return out


def mat4_from_scale_rotation_translation(scale, rotation, translation):
    return mat4_mul(mat4_from_translation(translation),
                    mat4_mul(mat4_from_quat(rotation), mat4_from_scale(scale)))


def mat4_inverse(m):
    """Gauss-Jordan with partial pivoting on a 4x8 augmented matrix."""
    a = []
    for i in range(4):
        row = [0.0] * 8
        for j in range(4):
            row[j] = mref(m, i, j)
        row[4 + i] = 1.0
        a.append(row)
    for col in range(4):
        pivot = col
        for r in range(col + 1, 4):
            if abs(a[r][col]) > abs(a[pivot][col]):
                pivot = r
        if abs(a[pivot][col]) < 1e-12:
            raise ValueError("mat4_inverse: matrix is singular")
        a[col], a[pivot] = a[pivot], a[col]
        prow = a[col]
        d = prow[col]
        for j in range(8):
            prow[j] = prow[j] / d
        for r in range(4):
            if r != col:
                row = a[r]
                f = row[col]
                for j in range(8):
                    row[j] = row[j] - f * prow[j]
    out = [0.0] * 16
    for i in range(4):
        for j in range(4):
            out[i + 4 * j] = a[i][4 + j]
    return out


def transform_point(m, v):
    x, y, z = v
    return (m[0] * x + m[4] * y + m[8] * z + m[12],
            m[1] * x + m[5] * y + m[9] * z + m[13],
            m[2] * x + m[6] * y + m[10] * z + m[14])


def transform_vector(m, v):
    x, y, z = v
    return (m[0] * x + m[4] * y + m[8] * z,
            m[1] * x + m[5] * y + m[9] * z,
            m[2] * x + m[6] * y + m[10] * z)


def transform_normal(local_from_world, v):
    m = local_from_world
    x, y, z = v
    return (m[0] * x + m[1] * y + m[2] * z,
            m[4] * x + m[5] * y + m[6] * z,
            m[8] * x + m[9] * y + m[10] * z)


# --- geometry: canonical primitives in their own local frame -----------------

UP = (0.0, 1.0, 0.0)


def sphere_intersect(o, d):
    a = dot(d, d)
    h = dot(o, d)
    c = dot(o, o) - 1.0
    disc = h * h - a * c
    if disc >= 0.0:
        sq = math.sqrt(disc)
        t = (-h - sq) / a
        if t <= 0.0:
            t = (-h + sq) / a
        if t > 0.0:
            p = v_add(o, v_mul(d, t))
            return (t, p, p)
    return None


def sphere_occluded(o, d, t_min, t_max):
    a = dot(d, d)
    h = dot(o, d)
    c = dot(o, o) - 1.0
    disc = h * h - a * c
    if disc >= 0.0:
        sq = math.sqrt(disc)
        t_near = (-h - sq) / a
        if t_min < t_near < t_max:
            return True
        t_far = (-h + sq) / a
        return t_min < t_far < t_max
    return False


def plane_intersect(o, d):
    dy = d[1]
    if abs(dy) >= 1e-6:
        t = -o[1] / dy
        if t > 0.0:
            return (t, v_add(o, v_mul(d, t)), UP)
    return None


def plane_occluded(o, d, t_min, t_max):
    dy = d[1]
    if abs(dy) >= 1e-6:
        t = -o[1] / dy
        return t_min < t < t_max
    return False


def rect_intersect(o, d):
    dy = d[1]
    if abs(dy) >= 1e-6:
        t = -o[1] / dy
        if t > 0.0:
            p = v_add(o, v_mul(d, t))
            if abs(p[0]) <= 1.0 and abs(p[2]) <= 1.0:
                return (t, p, UP)
    return None


def rect_occluded(o, d, t_min, t_max):
    dy = d[1]
    if abs(dy) >= 1e-6:
        t = -o[1] / dy
        if t_min < t < t_max:
            p = v_add(o, v_mul(d, t))
            return abs(p[0]) <= 1.0 and abs(p[2]) <= 1.0
    return False


def recip(d):
    """1/d, with the infinity Scheme's flonum division gives for a zero."""
    return 1.0 / d if d != 0.0 else math.copysign(INF, d)


def axis_normal(axis, sign):
    if axis == 0:
        return (sign, 0.0, 0.0)
    if axis == 1:
        return (0.0, sign, 0.0)
    return (0.0, 0.0, sign)


def cuboid_intersect(o, d):
    t_enter, t_exit, enter_axis, exit_axis = -INF, INF, 0, 0
    for axis in range(3):
        oc = o[axis]
        inv_d = recip(d[axis])
        a = (-1.0 - oc) * inv_d
        b = (1.0 - oc) * inv_d
        t0, t1 = (b, a) if inv_d < 0.0 else (a, b)
        if t0 > t_enter:
            t_enter, enter_axis = t0, axis
        if t1 < t_exit:
            t_exit, exit_axis = t1, axis
    if t_exit > t_enter and t_exit > 0.0:
        inside = t_enter <= 0.0
        t = t_exit if inside else t_enter
        axis = exit_axis if inside else enter_axis
        travel = -1.0 if d[axis] < 0.0 else 1.0
        sign = travel if inside else -travel
        return (t, v_add(o, v_mul(d, t)), axis_normal(axis, sign))
    return None


def cuboid_occluded(o, d, t_min, t_max):
    t_enter, t_exit = -INF, INF
    for axis in range(3):
        oc = o[axis]
        inv_d = recip(d[axis])
        a = (-1.0 - oc) * inv_d
        b = (1.0 - oc) * inv_d
        t0, t1 = (b, a) if inv_d < 0.0 else (a, b)
        if t0 > t_enter:
            t_enter = t0
        if t1 < t_exit:
            t_exit = t1
    return t_exit > t_enter and t_enter < t_max and t_exit > t_min


INTERSECT = {"sphere": sphere_intersect, "plane": plane_intersect,
             "rect": rect_intersect, "cuboid": cuboid_intersect}
OCCLUDED = {"sphere": sphere_occluded, "plane": plane_occluded,
            "rect": rect_occluded, "cuboid": cuboid_occluded}


# --- shapes, materials, lights, scene ---------------------------------------

class Shape:
    __slots__ = ("world_from_local", "local_from_world", "intersect", "occluded", "material")

    def __init__(self, world_from_local, geometry, material):
        self.world_from_local = world_from_local
        self.local_from_world = mat4_inverse(world_from_local)
        self.intersect = INTERSECT[geometry]
        self.occluded = OCCLUDED[geometry]
        self.material = material


def shape_sphere(center, radius, material):
    return Shape(mat4_mul(mat4_from_translation(center),
                          mat4_from_scale((radius, radius, radius))), "sphere", material)


def shape_plane(y, material):
    return Shape(mat4_from_translation((0.0, y, 0.0)), "plane", material)


def shape_hit_local(s, ray):
    lfw = s.local_from_world
    return s.intersect(transform_point(lfw, ray[0]), transform_vector(lfw, ray[1]))


def shape_finish_hit(s, lh):
    return (lh[0], transform_point(s.world_from_local, lh[1]),
            normalize(transform_normal(s.local_from_world, lh[2])), s.material)


def shape_occluded(s, ray, t_min, t_max):
    lfw = s.local_from_world
    return s.occluded(transform_point(lfw, ray[0]), transform_vector(lfw, ray[1]), t_min, t_max)


class Material:
    __slots__ = ("albedo", "specular", "shininess", "reflectivity", "transparency",
                 "ior", "abbe_number", "shader")

    def __init__(self, albedo, specular=(0.5, 0.5, 0.5), shininess=32.0, reflectivity=BLACK,
                 transparency=BLACK, ior=1.5, abbe_number=None, shader="lambertian"):
        self.albedo = albedo
        self.specular = specular
        self.shininess = shininess
        self.reflectivity = reflectivity
        self.transparency = transparency
        self.ior = ior
        self.abbe_number = abbe_number
        self.shader = shader


class PointLight:
    __slots__ = ("position", "power")

    def __init__(self, position, intensity):
        self.position = position
        self.power = intensity

    def intensity(self, point):
        return self.power


class SpotLight:
    """Full power within cone - penumbra degrees of the axis, easing to zero at cone."""
    __slots__ = ("position", "power", "direction", "cos_inner", "cos_outer")

    def __init__(self, position, target, intensity, cone_deg, penumbra_deg):
        self.position = position
        self.power = intensity
        self.direction = normalize(v_sub(target, position))
        self.cos_inner = math.cos(deg(max(0.0, cone_deg - penumbra_deg)))
        self.cos_outer = math.cos(deg(cone_deg))

    def intensity(self, point):
        cos_theta = dot(self.direction, normalize(v_sub(point, self.position)))
        band = self.cos_inner - self.cos_outer
        if band > 1.1920929e-7:          # raylisp's single-float-epsilon
            t = max(0.0, min(1.0, (cos_theta - self.cos_outer) / band))
        elif cos_theta >= self.cos_outer:
            t = 1.0
        else:
            t = 0.0
        return v_mul(self.power, t * t * (3.0 - 2.0 * t))


class Scene:
    __slots__ = ("camera", "shapes", "lights", "ambient", "sky_bottom", "sky_top", "max_depth")

    def __init__(self, camera, shapes, lights, ambient):
        self.camera = camera
        self.shapes = shapes
        self.lights = lights
        self.ambient = ambient
        self.sky_bottom = (0.4, 0.5, 0.7)
        self.sky_top = (0.08, 0.18, 0.5)
        self.max_depth = 8


def scene_intersect(scene, ray):
    best = best_lh = None
    for s in scene.shapes:
        lh = shape_hit_local(s, ray)
        if lh is not None and (best_lh is None or lh[0] < best_lh[0]):
            best, best_lh = s, lh
    return shape_finish_hit(best, best_lh) if best is not None else None


def scene_occluded(scene, ray, t_min, t_max):
    for s in scene.shapes:
        if shape_occluded(s, ray, t_min, t_max):
            return True
    return False


# --- camera -------------------------------------------------------------------

def camera(eye, target, up, vfov_deg, aspect):
    """(origin, lower-left, horizontal, vertical)"""
    h = math.tan(0.5 * deg(vfov_deg))
    viewport_h = 2.0 * h
    viewport_w = aspect * viewport_h
    w = normalize_or_zero(v_sub(eye, target))
    u = normalize_or_zero(cross(up, w))
    v = cross(w, u)
    horizontal = v_mul(u, viewport_w)
    vertical = v_mul(v, viewport_h)
    return (eye, v_sub(v_sub(v_sub(eye, v_mul(horizontal, 0.5)), v_mul(vertical, 0.5)), w),
            horizontal, vertical)


def camera_ray(cam, s, t):
    origin = cam[0]
    return (origin, v_sub(v_add(v_add(cam[1], v_mul(cam[2], s)), v_mul(cam[3], t)), origin))


# --- shading ------------------------------------------------------------------

SHADOW_T_MIN = 1e-3


def shade_lit(scene, ray, h, specular_model):
    mat = h[3]
    albedo = mat.albedo
    specular = mat.specular
    shininess = mat.shininess
    point = h[1]
    normal = h[2]
    view_dir = normalize(v_mul(ray[1], -1.0))
    ambient = scene.ambient
    ar, ag, ab = albedo
    r = ambient[0] * ar
    g = ambient[1] * ag
    b = ambient[2] * ab
    for light in scene.lights:
        to_light = v_sub(light.position, point)
        to_light_dir = normalize(to_light)
        cos_theta = dot(normal, to_light_dir)
        if cos_theta <= 0.0:
            continue
        intensity = light.intensity(point)
        if color_zero(intensity) or scene_occluded(scene, (point, to_light), SHADOW_T_MIN, 1.0):
            continue
        ir, ig, ib = intensity
        if specular_model == "phong":
            spec = max(0.0, dot(reflect(v_mul(to_light_dir, -1.0), normal), view_dir)) ** shininess
        elif specular_model == "blinn-phong":
            spec = max(0.0, dot(normal, normalize(v_add(view_dir, to_light_dir)))) ** shininess
        else:
            spec = 0.0
        r = r + ar * ir * cos_theta + specular[0] * ir * spec
        g = g + ag * ig * cos_theta + specular[1] * ig * spec
        b = b + ab * ib * cos_theta + specular[2] * ib * spec
    return (r, g, b)


def shade(scene, ray, h):
    shader = h[3].shader
    if shader == "normal-as-color":
        return v_mul(v_add(h[2], (1.0, 1.0, 1.0)), 0.5)
    if shader == "phong" or shader == "blinn-phong":
        return shade_lit(scene, ray, h, shader)
    return shade_lit(scene, ray, h, "none")


# --- the recursive integrator -----------------------------------------------

BOUNCE_EPS = 1e-3


def sky_color(scene, ray):
    a = 0.5 * (normalize(ray[1])[1] + 1.0)
    return v_add(v_mul(scene.sky_bottom, 1.0 - a), v_mul(scene.sky_top, a))


def blend(c, w, incoming):
    return (c[0] * (1.0 - w[0]) + w[0] * incoming[0],
            c[1] * (1.0 - w[1]) + w[1] * incoming[1],
            c[2] * (1.0 - w[2]) + w[2] * incoming[2])


def radiance(scene, ray, depth):
    if depth == 0:
        return BLACK
    h = scene_intersect(scene, ray)
    if h is None:
        return sky_color(scene, ray)
    mat = h[3]
    local = shade(scene, ray, h)
    k = mat.reflectivity
    tr = mat.transparency
    if color_zero(k) and color_zero(tr):
        return local
    incident = normalize(ray[1])
    normal = h[2]
    point = h[1]
    if color_zero(k):
        reflected = local
    else:
        reflected = blend(local, k, radiance(
            scene, (v_add(point, v_mul(normal, BOUNCE_EPS)), reflect(incident, normal)), depth - 1))
    if color_zero(tr):
        return reflected
    return blend(reflected, tr, refracted_radiance(scene, mat, incident, normal, point, depth))


def refract_dir(incident, normal, ior):
    if dot(normal, incident) < 0.0:
        refracted = refract(incident, normal, 1.0 / ior)
    else:
        refracted = refract(incident, v_mul(normal, -1.0), ior)
    return None if color_zero(refracted) else refracted


def dispersion_iors(ior, abbe):
    lc, ld, lf = 0.6563, 0.5876, 0.4861
    b = (ior - 1.0) / (abbe * (1.0 / (lf * lf) - 1.0 / (lc * lc)))
    a = ior - b / (ld * ld)
    return (a + b / (lc * lc), a + b / (ld * ld), a + b / (lf * lf))


def refracted_radiance(scene, mat, incident, normal, point, depth):
    tir = None

    def tir_reflect():
        nonlocal tir
        if tir is None:
            rdir = reflect(incident, normal)
            tir = radiance(scene, (v_add(point, v_mul(rdir, BOUNCE_EPS)), rdir), depth - 1)
        return tir

    def trace_channel(ior):
        d = refract_dir(incident, normal, ior)
        if d is None:
            return None
        return radiance(scene, (v_add(point, v_mul(d, BOUNCE_EPS)), d), depth - 1)

    if mat.abbe_number is not None:
        ior_r, ior_g, ior_b = dispersion_iors(mat.ior, mat.abbe_number)
        return ((trace_channel(ior_r) or tir_reflect())[0],
                (trace_channel(ior_g) or tir_reflect())[1],
                (trace_channel(ior_b) or tir_reflect())[2])
    return trace_channel(mat.ior) or tir_reflect()


# --- the demo scene: raylisp's src/scenes.lisp, value for value --------------

def demo_scene(aspect):
    eye = (0.0, 0.0, 1.6)
    cam = camera(eye, v_add(eye, (0.0, 0.0, -1.0)), (0.0, 1.0, 0.0), 60.0, aspect)
    white = (1.0, 1.0, 1.0)
    shapes = [
        shape_sphere((-2.45, -0.15, -2.6), 0.35,
                     Material((0.3, 0.5, 0.9), specular=white, shininess=16.0, shader="lambertian")),
        shape_sphere((-1.3, 0.0, -2.1), 0.5,
                     Material((0.9, 0.3, 0.3), specular=white, shininess=128.0, shader="phong",
                              transparency=(0.9, 0.9, 0.9), ior=1.5, abbe_number=15.0)),
        shape_sphere((-1.72, 0.0, -3.31), 0.5, Material((1.0, 0.65, 0.1))),
        shape_sphere((1.2, -0.1, -2.3), 0.4,
                     Material((0.4, 0.8, 0.4), specular=white, shininess=180.0, shader="blinn-phong")),
        shape_sphere((0.0, 0.3, -2.8), 0.8,
                     Material((0.05, 0.05, 0.05), reflectivity=(0.9, 0.9, 0.9))),
        shape_sphere((2.5, -0.1, -2.4), 0.4, Material(white, shader="normal-as-color")),
        shape_plane(-0.5, Material((0.6, 0.6, 0.6))),
        Shape(mat4_from_scale_rotation_translation(
                  (3.0, 1.0, 1.5), quat_from_axis_angle((1.0, 0.0, 0.0), deg(90.0)), (0.0, 1.3, -5.0)),
              "rect", Material((0.45, 0.25, 0.55))),
        Shape(mat4_from_scale_rotation_translation(
                  (0.35, 0.25, 0.25), quat_from_axis_angle((0.0, 1.0, 0.0), deg(30.0)), (1.5, -0.25, -1.3)),
              "cuboid", Material((0.2, 0.6, 0.6), specular=white, shininess=32.0)),
    ]
    lights = [
        PointLight((3.0, 3.0, 1.0), (1.0, 0.92, 0.80)),
        PointLight((-2.5, 1.5, 0.5), (0.30, 0.40, 0.55)),
        SpotLight((1.2, 3.5, 0.5), (1.2, -0.1, -2.3), (0.9, 0.95, 1.3), 18.0, 6.0),
    ]
    return Scene(cam, shapes, lights, (0.1, 0.1, 0.1))


# --- rendering and output -----------------------------------------------------

def render_row(scene, width, height, y):
    cam = scene.camera
    depth = scene.max_depth
    t = 1.0 - (y + 0.5) / height
    return [radiance(scene, camera_ray(cam, (x + 0.5) / width, t), depth) for x in range(width)]


def to_u8(c):
    return math.floor(min(1.0, max(0.0, c)) * 255.0 + 0.5)


def row_line(row):
    return " ".join(f"{to_u8(c[0])} {to_u8(c[1])} {to_u8(c[2])}" for c in row) + "\n"


def arg(name, default):
    args = sys.argv[1:]
    for i in range(len(args) - 1):
        if args[i] == name:
            return args[i + 1]
    return default


def main():
    width, height = (int(n) for n in arg("--size", "1280x720").split("x"))
    stripe = arg("--stripe", None)
    k, n = (int(s) for s in stripe.split("/")) if stripe else (0, 1)
    out = arg("--out", f"stripe-{k}.txt" if stripe else "demo.ppm")
    scene = demo_scene(width / height)
    start = time.perf_counter()
    rows = [render_row(scene, width, height, y) for y in range(k, height, n)]
    seconds = time.perf_counter() - start
    pixels = width * len(rows)
    with open(out, "w") as f:
        if not stripe:
            f.write(f"P3\n{width} {height}\n255\n")
        for row in rows:
            f.write(row_line(row))
    where = f" (stripe {stripe})" if stripe else ""
    print(f"rendered {pixels} px{where} in {round(seconds, 3)} s, "
          f"{round(pixels / seconds / 1e4) / 100} Mpix/s -> {out}")


if __name__ == "__main__":
    main()
