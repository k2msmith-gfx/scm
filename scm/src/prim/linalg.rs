//! `(scm linalg)`'s hot procedures, native (lux's plan, M7).
//!
//! The representation does not change: a mat4 is a Scheme vector of sixteen
//! flonums, column-major (element (row r, column c) at c*4+r, glam's
//! layout), a v3 three and a quaternion four, `x y z w`. A flonum is an
//! immediate `Value`, so a vector of them is one allocation and reading one
//! is a load -- which is why these can be native without a new type. What
//! was slow was the arithmetic: scm's VM spends ~37 ns on a floating-point
//! operation, and lux's P0 measured `mat4*` at 12.7 µs and
//! `mat4-translation` at 6.1 µs in Scheme.
//!
//! **Each is a port of the Scheme it replaces, operation for operation**, so
//! its results are bit-identical: `(+ a b c d)` folds left to right, and so
//! does every sum here. `tests/srfi/linalg-native.scm` holds the Scheme
//! originals and checks every procedure against them on random input, on
//! both tiers.
//!
//! **The vec3 layer (S39) keeps exactness**, which the matrices did not
//! need to: `[0 1 0]` is a v3, and `(v3+ [0 1 0] [1 0 0])` was the exact
//! `#(1 1 0)` in Scheme. So each v3 procedure computes on the components as
//! they are: two flonums are added, multiplied or compared directly, and
//! anything else goes to the generic `+`, `-`, `*`, `/`, `sqrt`, `min` and
//! `max` the Scheme called -- the same operations, in the same order and
//! grouping, so a result is bit-identical and exactly as exact as before.
//!
//! Two differences from the Scheme, both deliberate: results of arithmetic
//! of matrices are always inexact (a mat4 built from exact integers by hand multiplied
//! exactly in Scheme; every constructor `(scm linalg)` exports already made
//! flonums) -- `mat4-transpose`, which only moves elements, keeps them as
//! they are, as the Scheme did -- and an argument of the wrong shape is
//! refused naming the procedure, where the Scheme failed inside
//! `vector-ref`.

use super::{PResult, as_f64, err};
use crate::heap::{Engine, NumVec, Obj};
use crate::value::Value;
use crate::vm::{Vm, VmError};

pub(super) fn install(vm: &mut Vm) {
    vm.define_prim("%mat4*", mat4_mul);
    vm.define_prim("%mat4-transpose", mat4_transpose);
    vm.define_prim("%mat4-inverse", mat4_inverse);
    vm.define_prim("%mat4-translation", mat4_translation);
    vm.define_prim("%mat4-scaling", mat4_scaling);
    vm.define_prim("%mat4-rotation-x", mat4_rotation_x);
    vm.define_prim("%mat4-rotation-y", mat4_rotation_y);
    vm.define_prim("%mat4-rotation-z", mat4_rotation_z);
    vm.define_prim("%mat4-transform-point", mat4_transform_point);
    vm.define_prim("%mat4-transform-direction", mat4_transform_direction);
    vm.define_prim("%quat*", quat_mul);
    vm.define_prim("%quat->mat4", quat_to_mat4);
    vm.define_prim("%mat4-store!", mat4_store);
    vm.define_prim("%v3", v3);
    vm.define_prim("%v3+", v3_add);
    vm.define_prim("%v3-", v3_sub);
    vm.define_prim("%v3*", v3_mul);
    vm.define_prim("%v3/", v3_div);
    vm.define_prim("%v3-negate", v3_negate);
    vm.define_prim("%v3-scale", v3_scale);
    vm.define_prim("%v3-dot", v3_dot);
    vm.define_prim("%v3-cross", v3_cross);
    vm.define_prim("%v3-length", v3_length);
    vm.define_prim("%v3-length-squared", v3_length_squared);
    vm.define_prim("%v3-distance", v3_distance);
    vm.define_prim("%v3-distance-squared", v3_distance_squared);
    vm.define_prim("%v3-normalize", v3_normalize);
    vm.define_prim("%v3-lerp", v3_lerp);
    vm.define_prim("%v3-min", v3_min);
    vm.define_prim("%v3-max", v3_max);
}

/// The `n` reals of a vector argument, or a refusal naming `who`.
fn reals<const N: usize>(e: &Engine, who: &str, what: &str, v: Value) -> Result<[f64; N], VmError> {
    let refuse = || err(format!("{who}: expects a {what} (a vector of {N} reals)"));
    let Value::Obj(r) = v else { return Err(refuse()) };
    let Obj::Vector { items, .. } = e.obj(r) else { return Err(refuse()) };
    if items.len() != N {
        return Err(refuse());
    }
    let mut out = [0.0; N];
    for (o, &x) in out.iter_mut().zip(items.iter()) {
        *o = as_f64(e, x).ok_or_else(refuse)?;
    }
    Ok(out)
}

fn real(e: &Engine, who: &str, v: Value) -> Result<f64, VmError> {
    as_f64(e, v).ok_or_else(|| err(format!("{who}: expects a real")))
}

fn flonums(e: &mut Engine, xs: &[f64]) -> Value {
    e.vector(xs.iter().map(|&x| Value::Flonum(x)).collect(), false)
}

/// Sixteen values as they are written on paper, row by row, stored
/// column-major -- `mat4-from-rows`.
fn from_rows(rows: [f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            m[c * 4 + r] = rows[r * 4 + c];
        }
    }
    m
}

#[inline]
fn at(m: &[f64; 16], r: usize, c: usize) -> f64 {
    m[c * 4 + r]
}

fn mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            m[c * 4 + r] = at(a, r, 0) * at(b, 0, c)
                + at(a, r, 1) * at(b, 1, c)
                + at(a, r, 2) * at(b, 2, c)
                + at(a, r, 3) * at(b, 3, c);
        }
    }
    m
}

/// `(mat4* a b …)`, left to right. One argument is returned as it is, as
/// the Scheme's loop did.
fn mat4_mul(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "mat4*";
    let Some((&first, rest)) = a.split_first() else {
        return Err(err("mat4*: expects at least one matrix"));
    };
    if rest.is_empty() {
        reals::<16>(e, WHO, "mat4", first)?;
        return Ok(first);
    }
    let mut acc = reals::<16>(e, WHO, "mat4", first)?;
    for &m in rest {
        acc = mul(&acc, &reals::<16>(e, WHO, "mat4", m)?);
    }
    Ok(flonums(e, &acc))
}

/// Moves the elements themselves, as the Scheme did -- so an exact integer
/// stays exact, the one place here a result is not always inexact.
fn mat4_transpose(e: &mut Engine, a: &[Value]) -> PResult {
    let v = super::one("mat4-transpose", a)?;
    reals::<16>(e, "mat4-transpose", "mat4", v)?;
    let Value::Obj(r) = v else { unreachable!("checked by reals") };
    let Obj::Vector { items, .. } = e.obj(r) else { unreachable!("checked by reals") };
    let mut out = items.clone();
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = items[c * 4 + r];
        }
    }
    Ok(e.vector(out, false))
}

/// Gauss-Jordan with partial pivoting on `[ m | I ]`, as the Scheme does it:
/// the same pivot choice, the same scale by a reciprocal, the same row
/// updates in the same order.
fn mat4_inverse(e: &mut Engine, a: &[Value]) -> PResult {
    let m = reals::<16>(e, "mat4-inverse", "mat4", super::one("mat4-inverse", a)?)?;
    let mut s = [0.0f64; 32];
    for r in 0..4 {
        for c in 0..4 {
            s[r * 8 + c] = at(&m, r, c);
            s[r * 8 + 4 + c] = if r == c { 1.0 } else { 0.0 };
        }
    }
    for col in 0..4 {
        let mut best = col;
        let mut bv = s[col * 8 + col].abs();
        for r in col..4 {
            let v = s[r * 8 + col].abs();
            if v > bv {
                best = r;
                bv = v;
            }
        }
        if s[best * 8 + col].abs() < 1e-14 {
            return Err(err("mat4-inverse: the matrix is singular"));
        }
        if best != col {
            for c in 0..8 {
                s.swap(col * 8 + c, best * 8 + c);
            }
        }
        let k = 1.0 / s[col * 8 + col];
        for c in 0..8 {
            s[col * 8 + c] *= k;
        }
        for r in 0..4 {
            if r != col {
                let k = -s[r * 8 + col];
                for c in 0..8 {
                    s[r * 8 + c] += k * s[col * 8 + c];
                }
            }
        }
    }
    let mut out = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[c * 4 + r] = s[r * 8 + 4 + c];
        }
    }
    Ok(flonums(e, &out))
}

fn mat4_translation(e: &mut Engine, a: &[Value]) -> PResult {
    let [x, y, z] = reals::<3>(e, "mat4-translation", "v3", super::one("mat4-translation", a)?)?;
    let m = from_rows([1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0]);
    Ok(flonums(e, &m))
}

fn mat4_scaling(e: &mut Engine, a: &[Value]) -> PResult {
    let [x, y, z] = reals::<3>(e, "mat4-scaling", "v3", super::one("mat4-scaling", a)?)?;
    let m = from_rows([x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0]);
    Ok(flonums(e, &m))
}

fn mat4_rotation_x(e: &mut Engine, a: &[Value]) -> PResult {
    let t = real(e, "mat4-rotation-x", super::one("mat4-rotation-x", a)?)?;
    let (c, s) = (t.cos(), t.sin());
    let m = from_rows([1.0, 0.0, 0.0, 0.0, 0.0, c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0]);
    Ok(flonums(e, &m))
}

fn mat4_rotation_y(e: &mut Engine, a: &[Value]) -> PResult {
    let t = real(e, "mat4-rotation-y", super::one("mat4-rotation-y", a)?)?;
    let (c, s) = (t.cos(), t.sin());
    let m = from_rows([c, 0.0, s, 0.0, 0.0, 1.0, 0.0, 0.0, -s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0]);
    Ok(flonums(e, &m))
}

fn mat4_rotation_z(e: &mut Engine, a: &[Value]) -> PResult {
    let t = real(e, "mat4-rotation-z", super::one("mat4-rotation-z", a)?)?;
    let (c, s) = (t.cos(), t.sin());
    let m = from_rows([c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    Ok(flonums(e, &m))
}

fn two(who: &str, a: &[Value]) -> Result<(Value, Value), VmError> {
    match a {
        [x, y] => Ok((*x, *y)),
        _ => Err(err(format!("{who}: expects two arguments, got {}", a.len()))),
    }
}

/// w=1, then divided through by w unless it is exactly 1 or 0.
fn mat4_transform_point(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "mat4-transform-point";
    let (mv, pv) = two(WHO, a)?;
    let m = reals::<16>(e, WHO, "mat4", mv)?;
    let [x, y, z] = reals::<3>(e, WHO, "v3", pv)?;
    let row = |r| at(&m, r, 0) * x + at(&m, r, 1) * y + at(&m, r, 2) * z + at(&m, r, 3);
    let (rx, ry, rz, rw) = (row(0), row(1), row(2), row(3));
    let p = if rw == 1.0 || rw == 0.0 { [rx, ry, rz] } else { [rx / rw, ry / rw, rz / rw] };
    Ok(flonums(e, &p))
}

/// w=0: the translation does not apply.
fn mat4_transform_direction(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "mat4-transform-direction";
    let (mv, dv) = two(WHO, a)?;
    let m = reals::<16>(e, WHO, "mat4", mv)?;
    let [x, y, z] = reals::<3>(e, WHO, "v3", dv)?;
    let row = |r| at(&m, r, 0) * x + at(&m, r, 1) * y + at(&m, r, 2) * z;
    Ok(flonums(e, &[row(0), row(1), row(2)]))
}

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz + -(az * by),
        aw * by + -(ax * bz) + ay * bw + az * bx,
        aw * bz + ax * by + -(ay * bx) + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `(quat* a b …)`, left to right; one argument returned as it is.
fn quat_mul(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "quat*";
    let Some((&first, rest)) = a.split_first() else {
        return Err(err("quat*: expects at least one quaternion"));
    };
    let mut acc = reals::<4>(e, WHO, "quaternion", first)?;
    if rest.is_empty() {
        return Ok(first);
    }
    for &q in rest {
        acc = qmul(acc, reals::<4>(e, WHO, "quaternion", q)?);
    }
    Ok(flonums(e, &acc))
}

/// Normalised first, as `quat-normalize` does: the length is the square
/// root of the dot product, and each part is divided by it.
fn quat_to_mat4(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "quat->mat4";
    let q = reals::<4>(e, WHO, "quaternion", super::one(WHO, a)?)?;
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n == 0.0 {
        return Err(err("quat-normalize: the zero quaternion is not a rotation"));
    }
    let (x, y, z, w) = (q[0] / n, q[1] / n, q[2] / n, q[3] / n);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    let m = from_rows([
        1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy), 0.0,
        2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx), 0.0,
        2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy), 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]);
    Ok(flonums(e, &m))
}

/// `(mat4-store! dst offset m)`: M's sixteen values, in storage order
/// (column-major), into the `f32vector` or `f64vector` DST from OFFSET --
/// the one copy a matrix makes on its way to a GPU buffer. New with M7;
/// it has no Scheme original.
fn mat4_store(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "mat4-store!";
    let [dst, off, mv] = a else {
        return Err(err(format!("{WHO}: expects a numeric vector, an offset and a mat4, got {} arguments", a.len())));
    };
    let m = reals::<16>(e, WHO, "mat4", *mv)?;
    let Value::Fixnum(off) = *off else {
        return Err(err(format!("{WHO}: the offset must be an exact non-negative integer")));
    };
    let Value::Obj(r) = *dst else {
        return Err(err(format!("{WHO}: expects an f32vector or f64vector")));
    };
    let start = usize::try_from(off).map_err(|_| err(format!("{WHO}: the offset must be an exact non-negative integer")))?;
    let fits = |len: usize| start.checked_add(16).is_some_and(|end| end <= len);
    match e.obj_mut(r) {
        Obj::NumVector(NumVec::F32(v)) if fits(v.len()) => {
            for (d, x) in v[start..start + 16].iter_mut().zip(m) {
                *d = x as f32;
            }
        }
        Obj::NumVector(NumVec::F64(v)) if fits(v.len()) => v[start..start + 16].copy_from_slice(&m),
        Obj::NumVector(NumVec::F32(_) | NumVec::F64(_)) => {
            return Err(err(format!("{WHO}: sixteen values from offset {start} do not fit")));
        }
        _ => return Err(err(format!("{WHO}: expects an f32vector or f64vector"))),
    }
    Ok(Value::Unspecified)
}

// ── the vec3 layer (S39) ─────────────────────────────────────────────
//
// A v3's components are kept as they come, exact or not, and every
// operation below is one the Scheme made: a flonum fast path, else the
// generic primitive it called.

/// A v3's three components, as they are, or a refusal naming `who`.
fn parts(e: &Engine, who: &str, v: Value) -> Result<[Value; 3], VmError> {
    let refuse = || err(format!("{who}: expects a v3 (a vector of 3 reals)"));
    let Value::Obj(r) = v else { return Err(refuse()) };
    let Obj::Vector { items, .. } = e.obj(r) else { return Err(refuse()) };
    let [x, y, z] = items[..] else { return Err(refuse()) };
    for c in [x, y, z] {
        as_f64(e, c).ok_or_else(refuse)?;
    }
    Ok([x, y, z])
}

/// Exactly N arguments, or a refusal naming `who`.
fn args<const N: usize>(who: &str, a: &[Value]) -> Result<[Value; N], VmError> {
    a.try_into()
        .map_err(|_| err(format!("{who}: expects {N} arguments, got {}", a.len())))
}

fn vec3(e: &mut Engine, [x, y, z]: [Value; 3]) -> Value {
    e.vector(vec![x, y, z], false)
}

/// `%f`: `exact->inexact`.
fn inexact(e: &Engine, who: &str, v: Value) -> Result<f64, VmError> {
    real(e, who, v)
}

fn add2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(a + b)),
        _ => super::add(e, &[x, y]),
    }
}

/// `(+ a b c)`, one fold: not two `add2`s, which would add two exact
/// integers exactly before meeting a flonum, where `+` converts all three.
fn add3(e: &mut Engine, x: Value, y: Value, z: Value) -> PResult {
    match (x, y, z) {
        (Value::Flonum(a), Value::Flonum(b), Value::Flonum(c)) => Ok(Value::Flonum(a + b + c)),
        _ => super::add(e, &[x, y, z]),
    }
}

fn sub2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(a - b)),
        _ => super::sub(e, &[x, y]),
    }
}

fn neg(e: &mut Engine, x: Value) -> PResult {
    match x {
        Value::Flonum(a) => Ok(Value::Flonum(-a)),
        _ => super::sub(e, &[x]),
    }
}

fn mul2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(a * b)),
        _ => super::mul(e, &[x, y]),
    }
}

fn div2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(a / b)),
        _ => super::div(e, &[x, y]),
    }
}

fn sqrt(e: &mut Engine, x: Value) -> PResult {
    match x {
        Value::Flonum(a) => Ok(Value::Flonum(a.sqrt())),
        _ => super::p_sqrt(e, &[x]),
    }
}

/// `min` of two flonums is the first unless the second is less, as
/// `fold_extreme` decides.
fn min2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(if b < a { b } else { a })),
        _ => super::p_min(e, &[x, y]),
    }
}

fn max2(e: &mut Engine, x: Value, y: Value) -> PResult {
    match (x, y) {
        (Value::Flonum(a), Value::Flonum(b)) => Ok(Value::Flonum(if b > a { b } else { a })),
        _ => super::p_max(e, &[x, y]),
    }
}

/// `(+ (* x x) (* y y) (* z z))`.
fn length_squared(e: &mut Engine, [x, y, z]: [Value; 3]) -> PResult {
    let (xx, yy, zz) = (mul2(e, x, x)?, mul2(e, y, y)?, mul2(e, z, z)?);
    add3(e, xx, yy, zz)
}

fn difference(e: &mut Engine, [ax, ay, az]: [Value; 3], [bx, by, bz]: [Value; 3]) -> Result<[Value; 3], VmError> {
    Ok([sub2(e, ax, bx)?, sub2(e, ay, by)?, sub2(e, az, bz)?])
}

/// `v3-scale` once its factor is a flonum: `(* x k)` for each.
fn scaled(e: &mut Engine, [x, y, z]: [Value; 3], k: f64) -> Result<[Value; 3], VmError> {
    let k = Value::Flonum(k);
    Ok([mul2(e, x, k)?, mul2(e, y, k)?, mul2(e, z, k)?])
}

/// `(v3 x y z)`: three reals, made inexact.
fn v3(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3";
    let [x, y, z] = args::<3>(WHO, a)?;
    let c = [inexact(e, WHO, x)?, inexact(e, WHO, y)?, inexact(e, WHO, z)?];
    Ok(flonums(e, &c))
}

/// A variadic componentwise fold, left to right: `v3+`, `v3-`, `v3*`. One
/// argument is returned as it is, as the Scheme's loop did.
fn fold3(e: &mut Engine, who: &str, a: &[Value], op: fn(&mut Engine, Value, Value) -> PResult) -> PResult {
    let Some((&first, rest)) = a.split_first() else {
        return Err(err(format!("{who}: expects at least one v3")));
    };
    let mut acc = parts(e, who, first)?;
    if rest.is_empty() {
        return Ok(first);
    }
    for &v in rest {
        let [bx, by, bz] = parts(e, who, v)?;
        acc = [op(e, acc[0], bx)?, op(e, acc[1], by)?, op(e, acc[2], bz)?];
    }
    Ok(vec3(e, acc))
}

fn v3_add(e: &mut Engine, a: &[Value]) -> PResult {
    fold3(e, "v3+", a, add2)
}

/// One argument negates, as `-` does on numbers.
fn v3_sub(e: &mut Engine, a: &[Value]) -> PResult {
    if let [v] = a {
        return negate(e, "v3-", *v);
    }
    fold3(e, "v3-", a, sub2)
}

fn v3_mul(e: &mut Engine, a: &[Value]) -> PResult {
    fold3(e, "v3*", a, mul2)
}

fn v3_div(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3/";
    let [p, q] = args::<2>(WHO, a)?;
    let ([ax, ay, az], [bx, by, bz]) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let c = [div2(e, ax, bx)?, div2(e, ay, by)?, div2(e, az, bz)?];
    Ok(vec3(e, c))
}

fn negate(e: &mut Engine, who: &str, v: Value) -> PResult {
    let [x, y, z] = parts(e, who, v)?;
    let c = [neg(e, x)?, neg(e, y)?, neg(e, z)?];
    Ok(vec3(e, c))
}

fn v3_negate(e: &mut Engine, a: &[Value]) -> PResult {
    let [v] = args::<1>("v3-negate", a)?;
    negate(e, "v3-negate", v)
}

fn v3_scale(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-scale";
    let [v, k] = args::<2>(WHO, a)?;
    let p = parts(e, WHO, v)?;
    let k = inexact(e, WHO, k)?;
    let c = scaled(e, p, k)?;
    Ok(vec3(e, c))
}

/// `(+ (* ax bx) (* ay by) (* az bz))`.
fn v3_dot(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-dot";
    let [p, q] = args::<2>(WHO, a)?;
    let ([ax, ay, az], [bx, by, bz]) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let (x, y, z) = (mul2(e, ax, bx)?, mul2(e, ay, by)?, mul2(e, az, bz)?);
    add3(e, x, y, z)
}

fn v3_cross(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-cross";
    let [p, q] = args::<2>(WHO, a)?;
    let ([ax, ay, az], [bx, by, bz]) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let mut c = [Value::Unspecified; 3];
    for (out, (s, t, u, v)) in c.iter_mut().zip([(ay, bz, az, by), (az, bx, ax, bz), (ax, by, ay, bx)]) {
        let (l, r) = (mul2(e, s, t)?, mul2(e, u, v)?);
        *out = sub2(e, l, r)?;
    }
    Ok(vec3(e, c))
}

fn v3_length_squared(e: &mut Engine, a: &[Value]) -> PResult {
    let [v] = args::<1>("v3-length-squared", a)?;
    let p = parts(e, "v3-length-squared", v)?;
    length_squared(e, p)
}

fn v3_length(e: &mut Engine, a: &[Value]) -> PResult {
    let [v] = args::<1>("v3-length", a)?;
    let p = parts(e, "v3-length", v)?;
    let s = length_squared(e, p)?;
    sqrt(e, s)
}

fn v3_distance_squared(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-distance-squared";
    let [p, q] = args::<2>(WHO, a)?;
    let (p, q) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let d = difference(e, p, q)?;
    length_squared(e, d)
}

fn v3_distance(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-distance";
    let [p, q] = args::<2>(WHO, a)?;
    let (p, q) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let d = difference(e, p, q)?;
    let s = length_squared(e, d)?;
    sqrt(e, s)
}

/// Raises on a zero vector, with the Scheme's message and through the same
/// `error`: silently returning zero would make a mis-set direction look
/// like a legal one. Otherwise `(v3-scale a (/ 1.0 n))`.
fn v3_normalize(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-normalize";
    let [v] = args::<1>(WHO, a)?;
    let p = parts(e, WHO, v)?;
    let s = length_squared(e, p)?;
    let n = sqrt(e, s)?;
    if as_f64(e, n) == Some(0.0) {
        let msg = e.string("v3-normalize: the zero vector has no direction");
        return super::error_prim(e, &[msg]);
    }
    let k = div2(e, Value::Flonum(1.0), n)?;
    let k = inexact(e, WHO, k)?;
    let c = scaled(e, p, k)?;
    Ok(vec3(e, c))
}

/// `(+ a (* t (- b a)))` for each, `t` made inexact.
fn v3_lerp(e: &mut Engine, a: &[Value]) -> PResult {
    const WHO: &str = "v3-lerp";
    let [p, q, t] = args::<3>(WHO, a)?;
    let (p, q) = (parts(e, WHO, p)?, parts(e, WHO, q)?);
    let t = Value::Flonum(inexact(e, WHO, t)?);
    let mut c = [Value::Unspecified; 3];
    for i in 0..3 {
        let d = sub2(e, q[i], p[i])?;
        let m = mul2(e, t, d)?;
        c[i] = add2(e, p[i], m)?;
    }
    Ok(vec3(e, c))
}

fn extreme(e: &mut Engine, who: &str, a: &[Value], op: fn(&mut Engine, Value, Value) -> PResult) -> PResult {
    let [p, q] = args::<2>(who, a)?;
    let ([ax, ay, az], [bx, by, bz]) = (parts(e, who, p)?, parts(e, who, q)?);
    let c = [op(e, ax, bx)?, op(e, ay, by)?, op(e, az, bz)?];
    Ok(vec3(e, c))
}

fn v3_min(e: &mut Engine, a: &[Value]) -> PResult {
    extreme(e, "v3-min", a, min2)
}

fn v3_max(e: &mut Engine, a: &[Value]) -> PResult {
    extreme(e, "v3-max", a, max2)
}
