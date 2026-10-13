//! Enough primitives to exercise the machine. Not the standard library --
//! that is S5, behind `syntax-rules` and the library system.
//!
//! The arithmetic carries two settled decisions (plan §4.5), and both are
//! about refusing to lose information quietly:
//!
//!   * **Fixnum overflow is an error**, not a silent widening to a flonum.
//!   * **`(/ 8 3)` is inexact**, because `ratios` is not claimed -- which is
//!     the Steel trap avoided by construction rather than by discipline.

use crate::heap::{Engine, ErrorKind, Obj, PortKind};
use crate::value::Value;
use crate::vm::{Vm, VmError};
use crate::value::ObjRef;
use crate::writer::{Labels, display_str, write_str, write_with};

type PResult = Result<Value, VmError>;

fn err(msg: impl Into<String>) -> VmError {
    VmError::Error(msg.into())
}

/// A number as an `f64`, for when any argument is inexact.
fn as_f64(v: Value) -> Option<f64> {
    match v {
        Value::Fixnum(n) => Some(n as f64),
        Value::Flonum(x) => Some(x),
        _ => None,
    }
}

fn want_number(who: &str, v: Value, e: &Engine) -> Result<(), VmError> {
    if as_f64(v).is_none() {
        return Err(err(format!("{who}: not a number: {}", write_str(e, v))));
    }
    Ok(())
}

fn all_exact(args: &[Value]) -> bool {
    args.iter().all(|v| matches!(v, Value::Fixnum(_)))
}

/// `+`, `-` and `*` stay exact on exact inputs, and overflow is an error.
fn arith(
    who: &'static str,
    e: &Engine,
    args: &[Value],
    ident: i64,
    fx: fn(i64, i64) -> Option<i64>,
    fl: fn(f64, f64) -> f64,
) -> PResult {
    for &a in args {
        want_number(who, a, e)?;
    }
    if all_exact(args) {
        let mut acc = match args.len() {
            0 => return Ok(Value::Fixnum(ident)),
            1 => ident,
            _ => match args[0] {
                Value::Fixnum(n) => n,
                _ => unreachable!("all_exact"),
            },
        };
        let rest = if args.len() == 1 { args } else { &args[1..] };
        for &a in rest {
            let Value::Fixnum(n) = a else {
                unreachable!("all_exact")
            };
            acc = fx(acc, n).ok_or_else(|| {
                err(format!(
                    "{who}: exact integer overflow (no bignums: the `exact-closed` feature is absent)"
                ))
            })?;
        }
        return Ok(Value::Fixnum(acc));
    }
    let mut acc = match args.len() {
        0 => return Ok(Value::Fixnum(ident)),
        1 => ident as f64,
        _ => as_f64(args[0]).expect("checked"),
    };
    let rest = if args.len() == 1 { args } else { &args[1..] };
    for &a in rest {
        acc = fl(acc, as_f64(a).expect("checked"));
    }
    Ok(Value::Flonum(acc))
}

fn add(e: &mut Engine, a: &[Value]) -> PResult {
    arith("+", e, a, 0, i64::checked_add, |x, y| x + y)
}
fn sub(e: &mut Engine, a: &[Value]) -> PResult {
    if a.is_empty() {
        return Err(err("-: needs at least one argument"));
    }
    arith("-", e, a, 0, i64::checked_sub, |x, y| x - y)
}
fn mul(e: &mut Engine, a: &[Value]) -> PResult {
    arith("*", e, a, 1, i64::checked_mul, |x, y| x * y)
}

/// Division is where the absence of `ratios` shows. `(/ 8 4)` is the exact 2;
/// `(/ 8 3)` is the inexact 2.666… rather than the rational 8/3, which is what
/// silently turned Steel's `lorenz` into bignum arithmetic.
fn div(e: &mut Engine, a: &[Value]) -> PResult {
    if a.is_empty() {
        return Err(err("/: needs at least one argument"));
    }
    for &x in a {
        want_number("/", x, e)?;
    }
    let (first, rest) = if a.len() == 1 {
        (Value::Fixnum(1), a)
    } else {
        (a[0], &a[1..])
    };
    if all_exact(a) {
        let Value::Fixnum(mut acc) = first else {
            unreachable!()
        };
        let mut exact = true;
        let mut facc = acc as f64;
        for &d in rest {
            let Value::Fixnum(d) = d else { unreachable!() };
            if d == 0 {
                return Err(err("/: division by exact zero"));
            }
            if exact && acc % d == 0 {
                acc /= d;
                facc = acc as f64;
            } else {
                if exact {
                    facc = acc as f64;
                    exact = false;
                }
                facc /= d as f64;
            }
        }
        return Ok(if exact {
            Value::Fixnum(acc)
        } else {
            Value::Flonum(facc)
        });
    }
    let mut acc = as_f64(first).expect("checked");
    for &d in rest {
        acc /= as_f64(d).expect("checked");
    }
    Ok(Value::Flonum(acc))
}

fn compare(who: &'static str, e: &Engine, args: &[Value], ok: fn(f64, f64) -> bool) -> PResult {
    if args.len() < 2 {
        return Err(err(format!("{who}: needs at least two arguments")));
    }
    for &a in args {
        want_number(who, a, e)?;
    }
    for w in args.windows(2) {
        let (x, y) = (as_f64(w[0]).expect("checked"), as_f64(w[1]).expect("checked"));
        if !ok(x, y) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// `=` on a mixed pair must be mathematically correct, which comparing two
/// f64s is not: 9007199254740993 is not representable, so converting it
/// makes it equal to 9007199254740992.0. Compare in i128 when one side is
/// exact and the other is an integral flonum.
fn num_eq(e: &mut Engine, a: &[Value]) -> PResult {
    if a.len() < 2 {
        return Err(err("=: needs at least two arguments"));
    }
    for &x in a {
        want_number("=", x, e)?;
    }
    for w in a.windows(2) {
        let same = match (w[0], w[1]) {
            (Value::Fixnum(x), Value::Fixnum(y)) => x == y,
            (Value::Fixnum(n), Value::Flonum(f)) | (Value::Flonum(f), Value::Fixnum(n)) => {
                f.fract() == 0.0 && f.is_finite() && (f as i128) == (n as i128)
            }
            _ => as_f64(w[0]) == as_f64(w[1]),
        };
        if !same {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn lt(e: &mut Engine, a: &[Value]) -> PResult {
    compare("<", e, a, |x, y| x < y)
}
fn gt(e: &mut Engine, a: &[Value]) -> PResult {
    compare(">", e, a, |x, y| x > y)
}
fn le(e: &mut Engine, a: &[Value]) -> PResult {
    compare("<=", e, a, |x, y| x <= y)
}
fn ge(e: &mut Engine, a: &[Value]) -> PResult {
    compare(">=", e, a, |x, y| x >= y)
}

fn one(who: &str, a: &[Value]) -> Result<Value, VmError> {
    match a {
        [v] => Ok(*v),
        _ => Err(err(format!("{who}: expects one argument, got {}", a.len()))),
    }
}

fn cons(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [car, cdr] => Ok(e.cons(*car, *cdr)),
        _ => Err(err("cons: expects two arguments")),
    }
}

fn car(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("car", a)?;
    match v {
        Value::Obj(r) => match e.obj(r) {
            Obj::Pair(car, _) => Ok(*car),
            _ => Err(err(format!("car: not a pair: {}", write_str(e, v)))),
        },
        _ => Err(err(format!("car: not a pair: {}", write_str(e, v)))),
    }
}

fn cdr(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("cdr", a)?;
    match v {
        Value::Obj(r) => match e.obj(r) {
            Obj::Pair(_, cdr) => Ok(*cdr),
            _ => Err(err(format!("cdr: not a pair: {}", write_str(e, v)))),
        },
        _ => Err(err(format!("cdr: not a pair: {}", write_str(e, v)))),
    }
}

fn is_null(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(one("null?", a)?, Value::Null)))
}

fn is_pair(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("pair?", a)?;
    Ok(Value::Bool(matches!(
        v,
        Value::Obj(r) if matches!(e.obj(r), Obj::Pair(..))
    )))
}

fn not(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(!one("not", a)?.is_truthy()))
}

fn is_zero(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("zero?", a)?;
    want_number("zero?", v, e)?;
    Ok(Value::Bool(as_f64(v) == Some(0.0)))
}

/// `eq?` is identity. Immediates compare by value, and heap objects by
/// `ObjRef`, which `Value`'s own `PartialEq` already does -- interning is what
/// makes two `:aces` the same object, and so usable as an enum value.
fn eq(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [x, y] => Ok(Value::Bool(x == y)),
        _ => Err(err("eq?: expects two arguments")),
    }
}

/// `eqv?`. For the types that exist so far this is `eq?`, since numbers are
/// immediates and compare by value. It gets its own entry because the prelude
/// and R7RS both name it, and because it will diverge when records arrive.
fn eqv(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [x, y] => Ok(Value::Bool(x == y)),
        _ => Err(err("eqv?: expects two arguments")),
    }
}

fn equal(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [x, y] => Ok(Value::Bool(e.equal(*x, *y))),
        _ => Err(err("equal?: expects two arguments")),
    }
}

fn list(e: &mut Engine, a: &[Value]) -> PResult {
    Ok(e.list(a))
}

fn length(e: &mut Engine, a: &[Value]) -> PResult {
    let mut v = one("length", a)?;
    let mut n = 0i64;
    loop {
        match v {
            Value::Null => return Ok(Value::Fixnum(n)),
            Value::Obj(r) => match e.obj(r) {
                Obj::Pair(_, cdr) => {
                    n += 1;
                    v = *cdr;
                }
                _ => return Err(err("length: not a proper list")),
            },
            _ => return Err(err("length: not a proper list")),
        }
    }
}

fn vector(e: &mut Engine, a: &[Value]) -> PResult {
    // `(vector …)` and `[…]` build the same thing, and it prints with
    // brackets because that is how ray writes a position.
    Ok(e.vector(a.to_vec(), true))
}

fn vector_ref(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v @ Value::Obj(r), Value::Fixnum(i)] => match e.obj(*r) {
            Obj::Vector { items, .. } => items
                .get(*i as usize)
                .copied()
                .ok_or_else(|| err(format!("vector-ref: index {i} out of range"))),
            _ => Err(err(format!(
                "vector-ref: not a vector: {}",
                write_str(e, *v)
            ))),
        },
        _ => Err(err("vector-ref: expects a vector and an exact index")),
    }
}

fn vector_length(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("vector-length", a)?;
    match v {
        Value::Obj(r) => match e.obj(r) {
            Obj::Vector { items, .. } => Ok(Value::Fixnum(items.len() as i64)),
            _ => Err(err(format!(
                "vector-length: not a vector: {}",
                write_str(e, v)
            ))),
        },
        _ => Err(err(format!(
            "vector-length: not a vector: {}",
            write_str(e, v)
        ))),
    }
}

fn is_keyword(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(
        one("keyword?", a)?,
        Value::Keyword(_)
    )))
}

fn is_symbol(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(one("symbol?", a)?, Value::Symbol(_))))
}

/// `(values obj …)`. One value is not wrapped: `(values x)` IS `x`, so a
/// procedure that returns normally and one that returns a single value are
/// indistinguishable, which is what R7RS requires.
fn values_prim(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v] => Ok(*v),
        _ => Ok(e.alloc(Obj::Values(a.to_vec()))),
    }
}

#[allow(dead_code)]
/// A placeholder: `apply` is recognised by the VM before a primitive is ever
/// called, because a primitive cannot reach the control stack to call
/// anything. This body exists so the name has a slot in the table.
fn apply_placeholder(_e: &mut Engine, _a: &[Value]) -> PResult {
    Err(err("apply: internal error (should be handled by the VM)"))
}

/// The R7RS numeric tower, minus the parts §2.3 does not claim.
macro_rules! float_prim {
    ($name:ident, $f:expr) => {
        fn $name(e: &mut Engine, a: &[Value]) -> PResult {
            let v = one(stringify!($name), a)?;
            want_number(stringify!($name), v, e)?;
            let x: f64 = as_f64(v).expect("checked");
            Ok(Value::Flonum($f(x)))
        }
    };
}

float_prim!(p_sin, f64::sin);
float_prim!(p_cos, f64::cos);
float_prim!(p_tan, f64::tan);
float_prim!(p_asin, f64::asin);
float_prim!(p_acos, f64::acos);
float_prim!(p_exp, f64::exp);

/// `sqrt` of an exact perfect square is exact, which R7RS asks for and which
/// keeps `(sqrt 4)` from becoming inexact in a scene.
fn p_sqrt(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("sqrt", a)?;
    want_number("sqrt", v, e)?;
    if let Value::Fixnum(n) = v
        && n >= 0
    {
        let r = (n as f64).sqrt().round() as i64;
        if r * r == n {
            return Ok(Value::Fixnum(r));
        }
    }
    Ok(Value::Flonum(as_f64(v).expect("checked").sqrt()))
}

/// One argument is the natural log; two is a log in the given base.
fn p_log(e: &mut Engine, a: &[Value]) -> PResult {
    for &x in a {
        want_number("log", x, e)?;
    }
    match a {
        [x] => Ok(Value::Flonum(as_f64(*x).expect("checked").ln())),
        [x, b] => Ok(Value::Flonum(
            as_f64(*x).expect("checked").ln() / as_f64(*b).expect("checked").ln(),
        )),
        _ => Err(err("log: expects one or two arguments")),
    }
}

/// R7RS's two-argument `atan` *is* Janet's `math/atan2`, which is one less
/// thing for a conversion to get wrong.
fn p_atan(e: &mut Engine, a: &[Value]) -> PResult {
    for &x in a {
        want_number("atan", x, e)?;
    }
    match a {
        [x] => Ok(Value::Flonum(as_f64(*x).expect("checked").atan())),
        [y, x] => Ok(Value::Flonum(
            as_f64(*y).expect("checked").atan2(as_f64(*x).expect("checked")),
        )),
        _ => Err(err("atan: expects one or two arguments")),
    }
}

/// `expt`, exact when it can be.
fn p_expt(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [base, exp] => {
            want_number("expt", *base, e)?;
            want_number("expt", *exp, e)?;
            if let (Value::Fixnum(b), Value::Fixnum(x)) = (base, exp)
                && *x >= 0
                && *x < 63
            {
                let mut acc: i64 = 1;
                for _ in 0..*x {
                    acc = acc.checked_mul(*b).ok_or_else(|| {
                        err("expt: exact integer overflow (the `exact-closed` feature is absent)")
                    })?;
                }
                return Ok(Value::Fixnum(acc));
            }
            Ok(Value::Flonum(
                as_f64(*base).expect("checked").powf(as_f64(*exp).expect("checked")),
            ))
        }
        _ => Err(err("expt: expects two arguments")),
    }
}

/// `floor`, `ceiling`, `round` and `truncate` keep exactness on an exact
/// argument, as R7RS requires.
macro_rules! round_prim {
    ($name:ident, $who:expr, $f:expr) => {
        fn $name(e: &mut Engine, a: &[Value]) -> PResult {
            let v = one($who, a)?;
            want_number($who, v, e)?;
            match v {
                Value::Fixnum(_) => Ok(v),
                _ => Ok(Value::Flonum($f(as_f64(v).expect("checked")))),
            }
        }
    };
}

round_prim!(p_floor, "floor", f64::floor);
round_prim!(p_ceiling, "ceiling", f64::ceil);
round_prim!(p_truncate, "truncate", f64::trunc);
// R7RS `round` rounds to even, which `f64::round` does not.
round_prim!(p_round, "round", |x: f64| {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
});

fn p_abs(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("abs", a)?;
    want_number("abs", v, e)?;
    match v {
        Value::Fixnum(n) => n
            .checked_abs()
            .map(Value::Fixnum)
            .ok_or_else(|| err("abs: exact integer overflow")),
        _ => Ok(Value::Flonum(as_f64(v).expect("checked").abs())),
    }
}

fn int_pair(who: &'static str, a: &[Value]) -> Result<(i64, i64), VmError> {
    match a {
        [Value::Fixnum(x), Value::Fixnum(y)] => {
            if *y == 0 {
                return Err(err(format!("{who}: division by zero")));
            }
            Ok((*x, *y))
        }
        _ => Err(err(format!("{who}: expects two exact integers"))),
    }
}

fn p_quotient(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("quotient", a)?;
    Ok(Value::Fixnum(x / y))
}

fn p_remainder(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("remainder", a)?;
    Ok(Value::Fixnum(x % y))
}

/// `modulo` takes the sign of the *divisor*, which the first version got
/// wrong for a negative one -- `(modulo 13 -4)` is -3, not -1. Found by the
/// R7RS suite; `rem_euclid` is not this operation.
fn p_modulo(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("modulo", a)?;
    Ok(Value::Fixnum(((x % y) + y) % y))
}

fn p_min(e: &mut Engine, a: &[Value]) -> PResult {
    fold_extreme("min", e, a, true)
}

fn p_max(e: &mut Engine, a: &[Value]) -> PResult {
    fold_extreme("max", e, a, false)
}

/// R7RS: if any argument is inexact, the result is inexact.
fn fold_extreme(who: &'static str, e: &Engine, a: &[Value], want_min: bool) -> PResult {
    if a.is_empty() {
        return Err(err(format!("{who}: needs at least one argument")));
    }
    for &x in a {
        want_number(who, x, e)?;
    }
    let inexact = a.iter().any(|v| matches!(v, Value::Flonum(_)));
    let mut best = a[0];
    for &x in &a[1..] {
        let (bx, xx) = (as_f64(best).expect("checked"), as_f64(x).expect("checked"));
        if (want_min && xx < bx) || (!want_min && xx > bx) {
            best = x;
        }
    }
    Ok(if inexact {
        Value::Flonum(as_f64(best).expect("checked"))
    } else {
        best
    })
}

fn p_exact_to_inexact(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("exact->inexact", a)?;
    want_number("exact->inexact", v, e)?;
    Ok(Value::Flonum(as_f64(v).expect("checked")))
}

fn p_is_exact(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("exact?", a)?;
    want_number("exact?", v, e)?;
    Ok(Value::Bool(matches!(v, Value::Fixnum(_))))
}

fn p_is_number(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(as_f64(one("number?", a)?).is_some()))
}

/// The port to write to: the optional argument, or `current-output-port`.
fn out_port(vm: &Vm, a: &[Value], at: usize) -> Result<Value, VmError> {
    match a.get(at) {
        Some(v) => Ok(*v),
        None => param_now(vm, "current-output-port"),
    }
}

fn display(vm: &mut Vm, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("display: expects a value")),
    };
    let port = out_port(vm, a, 1)?;
    let text = display_str(&vm.engine, v);
    emit(&mut vm.engine, port, &text)?;
    Ok(Value::Unspecified)
}

fn write_prim(vm: &mut Vm, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("write: expects a value")),
    };
    let port = out_port(vm, a, 1)?;
    let text = write_str(&vm.engine, v);
    emit(&mut vm.engine, port, &text)?;
    Ok(Value::Unspecified)
}

fn newline(vm: &mut Vm, a: &[Value]) -> PResult {
    let port = out_port(vm, a, 0)?;
    emit(&mut vm.engine, port, "\n")?;
    Ok(Value::Unspecified)
}

fn write_char(vm: &mut Vm, a: &[Value]) -> PResult {
    let c = match a.first() {
        Some(Value::Char(c)) => *c,
        _ => return Err(err("write-char: expects a character")),
    };
    let port = out_port(vm, a, 1)?;
    emit(&mut vm.engine, port, &c.to_string())?;
    Ok(Value::Unspecified)
}

fn write_string(vm: &mut Vm, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("write-string: expects a string")),
    };
    let cs = chars_of("write-string", &vm.engine, v)?;
    let port = out_port(vm, a, 1)?;
    let (start, end) = range_of("write-string", a, 2, cs.len())?;
    let text: String = cs[start..end].iter().collect();
    emit(&mut vm.engine, port, &text)?;
    Ok(Value::Unspecified)
}

/// `write-simple` prints without labels, which R7RS says may not terminate
/// on a cycle -- and now that `set-cdr!` exists, can be made to prove it.
fn write_simple(vm: &mut Vm, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("write-simple: expects a value")),
    };
    let port = out_port(vm, a, 1)?;
    let text = write_with(&vm.engine, v, true, Labels::None);
    emit(&mut vm.engine, port, &text)?;
    Ok(Value::Unspecified)
}

/// `write-shared` labels every shared node, not only the cyclic ones.
fn write_shared(vm: &mut Vm, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("write-shared: expects a value")),
    };
    let port = out_port(vm, a, 1)?;
    let text = write_with(&vm.engine, v, true, Labels::Shared);
    emit(&mut vm.engine, port, &text)?;
    Ok(Value::Unspecified)
}

fn read_char_prim(e: &mut Engine, a: &[Value]) -> PResult {
    read_char(e, a, false)
}

fn peek_char_prim(e: &mut Engine, a: &[Value]) -> PResult {
    read_char(e, a, true)
}


// ── vectors ──────────────────────────────────────────────────────────────
//
// One vector type, and it grows. R7RS vectors are fixed-length and the
// standard has no growable sequence at all, but ray's scene corpus does
// `array/push` 248 times across 48 files (scene-language-survey §2), so the
// idiom has to exist. The survey floated a separate growable type with a
// wrapper; this is simpler and was the better answer once written down --
// `Obj::Vector` already holds a `Vec<Value>`, so `vector-push!` is one
// operation rather than a second type that every `vector?` has to know about.
//
// It is a documented non-standard addition, not the recommended idiom: a model
// writes accumulate-and-reverse (plan §4.9), and that path is the fast one.

fn vec_ref(who: &str, e: &Engine, v: Value) -> Result<crate::value::ObjRef, VmError> {
    match v {
        Value::Obj(r) if matches!(e.obj(r), Obj::Vector { .. }) => Ok(r),
        _ => Err(err(format!("{who}: not a vector: {}", write_str(e, v)))),
    }
}

fn make_vector(e: &mut Engine, a: &[Value]) -> PResult {
    let (n, fill) = match a {
        [Value::Fixnum(n)] => (*n, Value::Bool(false)),
        [Value::Fixnum(n), fill] => (*n, *fill),
        _ => return Err(err("make-vector: expects a length and an optional fill")),
    };
    if n < 0 {
        return Err(err("make-vector: negative length"));
    }
    Ok(e.vector(vec![fill; n as usize], true))
}

fn vector_set(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, Value::Fixnum(i), x] => {
            let r = vec_ref("vector-set!", e, *v)?;
            let (i, x) = (*i, *x);
            let Obj::Vector { items, .. } = e.obj_mut(r) else {
                unreachable!("checked")
            };
            match items.get_mut(i as usize) {
                Some(slot) => {
                    *slot = x;
                    Ok(Value::Unspecified)
                }
                None => Err(err(format!("vector-set!: index {i} out of range"))),
            }
        }
        _ => Err(err("vector-set!: expects a vector, an index and a value")),
    }
}

/// The growable addition.
fn vector_push(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, rest @ ..] => {
            let r = vec_ref("vector-push!", e, *v)?;
            let rest: Vec<Value> = rest.to_vec();
            let Obj::Vector { items, .. } = e.obj_mut(r) else {
                unreachable!("checked")
            };
            items.extend(rest);
            Ok(*v)
        }
        _ => Err(err("vector-push!: expects a vector and a value")),
    }
}

fn vector_pop(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("vector-pop!", a)?;
    let r = vec_ref("vector-pop!", e, v)?;
    let Obj::Vector { items, .. } = e.obj_mut(r) else {
        unreachable!("checked")
    };
    items.pop().ok_or_else(|| err("vector-pop!: empty vector"))
}

fn vector_to_list(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("vector->list: expects a vector")),
    };
    let r = vec_ref("vector->list", e, v)?;
    let Obj::Vector { items, .. } = e.obj(r) else {
        unreachable!("checked")
    };
    let items = items.clone();
    let (start, end) = range_of("vector->list", a, 1, items.len())?;
    Ok(e.list(&items[start..end]))
}

fn list_to_vector(e: &mut Engine, a: &[Value]) -> PResult {
    let mut v = one("list->vector", a)?;
    let mut items = Vec::new();
    loop {
        match v {
            Value::Null => break,
            Value::Obj(r) => match e.obj(r) {
                Obj::Pair(car, cdr) => {
                    items.push(*car);
                    v = *cdr;
                }
                _ => return Err(err("list->vector: not a proper list")),
            },
            _ => return Err(err("list->vector: not a proper list")),
        }
    }
    Ok(e.vector(items, true))
}

fn vector_fill(e: &mut Engine, a: &[Value]) -> PResult {
    let (v, x) = match a {
        [v, x, ..] => (*v, *x),
        _ => return Err(err("vector-fill!: expects a vector and a value")),
    };
    let r = vec_ref("vector-fill!", e, v)?;
    let len = match e.obj(r) {
        Obj::Vector { items, .. } => items.len(),
        _ => unreachable!("checked"),
    };
    let (start, end) = range_of("vector-fill!", a, 2, len)?;
    let Obj::Vector { items, .. } = e.obj_mut(r) else {
        unreachable!("checked")
    };
    items[start..end].fill(x);
    Ok(Value::Unspecified)
}

fn string_fill(e: &mut Engine, a: &[Value]) -> PResult {
    let (v, c) = match a {
        [v, Value::Char(c), ..] => (*v, *c),
        _ => return Err(err("string-fill!: expects a string and a character")),
    };
    let r = str_ref("string-fill!", e, v)?;
    let len = match e.obj(r) {
        Obj::Str(cs) => cs.len(),
        _ => unreachable!("checked"),
    };
    let (start, end) = range_of("string-fill!", a, 2, len)?;
    let Obj::Str(cs) = e.obj_mut(r) else {
        unreachable!("checked")
    };
    cs[start..end].fill(c);
    Ok(Value::Unspecified)
}

fn vector_copy(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("vector-copy: expects a vector")),
    };
    let r = vec_ref("vector-copy", e, v)?;
    let Obj::Vector { items, bracket } = e.obj(r) else {
        unreachable!("checked")
    };
    let (items, bracket) = (items.clone(), *bracket);
    let (start, end) = range_of("vector-copy", a, 1, items.len())?;
    Ok(e.vector(items[start..end].to_vec(), bracket))
}

fn is_vector(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("vector?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::Vector { .. })),
    ))
}

// ── strings ──────────────────────────────────────────────────────────────

fn str_ref(who: &str, e: &Engine, v: Value) -> Result<crate::value::ObjRef, VmError> {
    match v {
        Value::Obj(r) if matches!(e.obj(r), Obj::Str(_)) => Ok(r),
        _ => Err(err(format!("{who}: not a string: {}", write_str(e, v)))),
    }
}

fn chars_of(who: &str, e: &Engine, v: Value) -> Result<Vec<char>, VmError> {
    let r = str_ref(who, e, v)?;
    let Obj::Str(cs) = e.obj(r) else {
        unreachable!("checked")
    };
    Ok(cs.clone())
}

fn string_length(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string-length", a)?;
    Ok(Value::Fixnum(chars_of("string-length", e, v)?.len() as i64))
}

fn string_ref(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, Value::Fixnum(i)] => {
            let cs = chars_of("string-ref", e, *v)?;
            cs.get(*i as usize)
                .copied()
                .map(Value::Char)
                .ok_or_else(|| err(format!("string-ref: index {i} out of range")))
        }
        _ => Err(err("string-ref: expects a string and an index")),
    }
}

fn string_set(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, Value::Fixnum(i), Value::Char(c)] => {
            let r = str_ref("string-set!", e, *v)?;
            let (i, c) = (*i, *c);
            let Obj::Str(cs) = e.obj_mut(r) else {
                unreachable!("checked")
            };
            match cs.get_mut(i as usize) {
                Some(slot) => {
                    *slot = c;
                    Ok(Value::Unspecified)
                }
                None => Err(err(format!("string-set!: index {i} out of range"))),
            }
        }
        _ => Err(err("string-set!: expects a string, an index and a character")),
    }
}

fn make_string(e: &mut Engine, a: &[Value]) -> PResult {
    let (n, fill) = match a {
        [Value::Fixnum(n)] => (*n, ' '),
        [Value::Fixnum(n), Value::Char(c)] => (*n, *c),
        _ => return Err(err("make-string: expects a length and an optional character")),
    };
    if n < 0 {
        return Err(err("make-string: negative length"));
    }
    Ok(e.alloc(Obj::Str(vec![fill; n as usize])))
}

/// R7RS `string` builds a string from CHARACTERS.
///
/// Janet's `string` stringifies and concatenates, which is a conversion
/// hazard rather than something to imitate: 75 corpus uses of `(string a b)`
/// become `(string-append …)` with `number->string` where needed. Plan §4.9
/// says the standard spelling wins, because that is what a model writes.
fn string_from_chars(e: &mut Engine, a: &[Value]) -> PResult {
    let mut out = Vec::with_capacity(a.len());
    for &v in a {
        match v {
            Value::Char(c) => out.push(c),
            _ => {
                return Err(err(format!(
                    "string: expects characters, got {} (for concatenation use string-append)",
                    write_str(e, v)
                )));
            }
        }
    }
    Ok(e.alloc(Obj::Str(out)))
}

fn string_append(e: &mut Engine, a: &[Value]) -> PResult {
    let mut out = Vec::new();
    for &v in a {
        out.extend(chars_of("string-append", e, v)?);
    }
    Ok(e.alloc(Obj::Str(out)))
}

fn substring(e: &mut Engine, a: &[Value]) -> PResult {
    let (v, start, end) = match a {
        [v, Value::Fixnum(s)] => (*v, *s, None),
        [v, Value::Fixnum(s), Value::Fixnum(x)] => (*v, *s, Some(*x)),
        _ => return Err(err("substring: expects a string, a start and an optional end")),
    };
    let cs = chars_of("substring", e, v)?;
    let end = end.unwrap_or(cs.len() as i64);
    if start < 0 || end > cs.len() as i64 || start > end {
        return Err(err(format!(
            "substring: {start}..{end} is out of range for a string of {}",
            cs.len()
        )));
    }
    Ok(e.alloc(Obj::Str(cs[start as usize..end as usize].to_vec())))
}

fn string_to_list(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("string->list: expects a string")),
    };
    let cs = chars_of("string->list", e, v)?;
    let (start, end) = range_of("string->list", a, 1, cs.len())?;
    let items: Vec<Value> = cs[start..end].iter().copied().map(Value::Char).collect();
    Ok(e.list(&items))
}

fn list_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    let mut v = one("list->string", a)?;
    let mut out = Vec::new();
    loop {
        match v {
            Value::Null => break,
            Value::Obj(r) => match *e.obj(r) {
                Obj::Pair(Value::Char(c), cdr) => {
                    out.push(c);
                    v = cdr;
                }
                Obj::Pair(..) => return Err(err("list->string: not a list of characters")),
                _ => return Err(err("list->string: not a proper list")),
            },
            _ => return Err(err("list->string: not a proper list")),
        }
    }
    Ok(e.alloc(Obj::Str(out)))
}

fn string_compare(
    who: &'static str,
    e: &Engine,
    a: &[Value],
    ok: fn(std::cmp::Ordering) -> bool,
    fold: bool,
) -> PResult {
    if a.len() < 2 {
        return Err(err(format!("{who}: needs at least two strings")));
    }
    let mut prev = chars_of(who, e, a[0])?;
    if fold {
        prev = prev.iter().flat_map(|c| c.to_lowercase()).collect();
    }
    for &v in &a[1..] {
        let mut next = chars_of(who, e, v)?;
        if fold {
            next = next.iter().flat_map(|c| c.to_lowercase()).collect();
        }
        if !ok(prev.cmp(&next)) {
            return Ok(Value::Bool(false));
        }
        prev = next;
    }
    Ok(Value::Bool(true))
}

fn string_eq(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string=?", e, a, |o| o == std::cmp::Ordering::Equal, false)
}
fn string_lt(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string<?", e, a, |o| o == std::cmp::Ordering::Less, false)
}
fn string_gt(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string>?", e, a, |o| o == std::cmp::Ordering::Greater, false)
}
fn string_ci_eq(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string-ci=?", e, a, |o| o == std::cmp::Ordering::Equal, true)
}

fn string_upcase(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string-upcase", a)?;
    let cs: Vec<char> = chars_of("string-upcase", e, v)?
        .iter()
        .flat_map(|c| c.to_uppercase())
        .collect();
    Ok(e.alloc(Obj::Str(cs)))
}

fn string_downcase(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string-downcase", a)?;
    let cs: Vec<char> = chars_of("string-downcase", e, v)?
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect();
    Ok(e.alloc(Obj::Str(cs)))
}

fn string_copy(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string-copy", a)?;
    let cs = chars_of("string-copy", e, v)?;
    Ok(e.alloc(Obj::Str(cs)))
}

fn is_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::Str(_))),
    ))
}

// ── conversions between the name-ish types ──────────────────────────────

fn string_to_symbol(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string->symbol", a)?;
    let s: String = chars_of("string->symbol", e, v)?.into_iter().collect();
    Ok(e.sym(&s))
}

fn symbol_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    match one("symbol->string", a)? {
        Value::Symbol(id) => {
            let name = e.interner.sym_name(e.resolve(id)).to_string();
            Ok(e.string(&name))
        }
        v => Err(err(format!(
            "symbol->string: not a symbol: {}",
            write_str(e, v)
        ))),
    }
}

/// `string->keyword` and `keyword->string`, which are how a converted scene
/// gets at Janet's `(keyword …)` -- 101 uses in the corpus.
fn string_to_keyword(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string->keyword", a)?;
    let s: String = chars_of("string->keyword", e, v)?.into_iter().collect();
    Ok(e.kw(&s))
}

fn keyword_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    match one("keyword->string", a)? {
        Value::Keyword(id) => {
            let name = e.interner.kw_name(id).to_string();
            Ok(e.string(&name))
        }
        v => Err(err(format!(
            "keyword->string: not a keyword: {}",
            write_str(e, v)
        ))),
    }
}

/// `string->number`, which is Janet's `scan-number` -- 45 corpus uses. Returns
/// `#f` rather than raising, as R7RS requires.
fn string_to_number(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("string->number: expects a string")),
    };
    let text: String = chars_of("string->number", e, v)?.into_iter().collect();
    let s = match a.get(1) {
        Some(Value::Fixnum(2)) => format!("#b{text}"),
        Some(Value::Fixnum(8)) => format!("#o{text}"),
        Some(Value::Fixnum(16)) => format!("#x{text}"),
        Some(Value::Fixnum(10)) | None => text,
        _ => return Err(err("string->number: radix must be 2, 8, 10 or 16")),
    };
    let trimmed = s.trim();
    let mut scratch = Engine::new();
    let mut reader = crate::reader::Reader::new(trimmed);
    match reader.read(&mut scratch) {
        // The whole string has to be a number: `"1 2"` is #f, not 1.
        Ok(Some(n @ (Value::Fixnum(_) | Value::Flonum(_))))
            if reader.consumed() == trimmed.chars().count() =>
        {
            Ok(n)
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn number_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("number->string: expects a number")),
    };
    want_number("number->string", v, e)?;
    let radix = match a.get(1) {
        Some(Value::Fixnum(r)) if [2, 8, 10, 16].contains(r) => *r as u32,
        None => 10,
        _ => return Err(err("number->string: radix must be 2, 8, 10 or 16")),
    };
    if radix == 10 {
        let text = write_str(e, v);
        return Ok(e.string(&text));
    }
    let Value::Fixnum(n) = v else {
        return Err(err("number->string: a non-decimal radix needs an exact integer"));
    };
    let (sign, mag) = if n < 0 {
        ("-", (n as i128).unsigned_abs())
    } else {
        ("", n as u128)
    };
    let digits = match radix {
        2 => format!("{mag:b}"),
        8 => format!("{mag:o}"),
        _ => format!("{mag:x}"),
    };
    Ok(e.string(&format!("{sign}{digits}")))
}

// ── characters ──────────────────────────────────────────────────────────

fn char_to_integer(e: &mut Engine, a: &[Value]) -> PResult {
    match one("char->integer", a)? {
        Value::Char(c) => Ok(Value::Fixnum(c as i64)),
        v => Err(err(format!(
            "char->integer: not a character: {}",
            write_str(e, v)
        ))),
    }
}

fn integer_to_char(_e: &mut Engine, a: &[Value]) -> PResult {
    match one("integer->char", a)? {
        Value::Fixnum(n) => u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .map(Value::Char)
            .ok_or_else(|| err(format!("integer->char: {n} is not a Unicode scalar value"))),
        _ => Err(err("integer->char: expects an exact integer")),
    }
}

fn char_pred(who: &'static str, a: &[Value], f: fn(char) -> bool) -> PResult {
    match a {
        [Value::Char(c)] => Ok(Value::Bool(f(*c))),
        _ => Err(err(format!("{who}: expects one character"))),
    }
}

fn is_char(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(one("char?", a)?, Value::Char(_))))
}
fn char_alpha(_e: &mut Engine, a: &[Value]) -> PResult {
    char_pred("char-alphabetic?", a, |c| c.is_alphabetic())
}
fn char_numeric(_e: &mut Engine, a: &[Value]) -> PResult {
    char_pred("char-numeric?", a, |c| c.is_numeric())
}
fn char_white(_e: &mut Engine, a: &[Value]) -> PResult {
    char_pred("char-whitespace?", a, |c| c.is_whitespace())
}
fn char_upcase(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Char(c)] => Ok(Value::Char(c.to_uppercase().next().unwrap_or(*c))),
        _ => Err(err("char-upcase: expects one character")),
    }
}
fn char_downcase(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Char(c)] => Ok(Value::Char(c.to_lowercase().next().unwrap_or(*c))),
        _ => Err(err("char-downcase: expects one character")),
    }
}
fn char_cmp(who: &'static str, a: &[Value], ok: fn(char, char) -> bool) -> PResult {
    if a.len() < 2 {
        return Err(err(format!("{who}: needs at least two characters")));
    }
    for w in a.windows(2) {
        match (w[0], w[1]) {
            (Value::Char(x), Value::Char(y)) => {
                if !ok(x, y) {
                    return Ok(Value::Bool(false));
                }
            }
            _ => return Err(err(format!("{who}: expects characters"))),
        }
    }
    Ok(Value::Bool(true))
}
fn char_eq(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char=?", a, |x, y| x == y)
}
fn char_lt(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char<?", a, |x, y| x < y)
}
fn char_gt(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char>?", a, |x, y| x > y)
}

// ── format ───────────────────────────────────────────────────────────────

/// SRFI-28's `~a`, `~s`, `~%` and `~~`, plus `~d` and a `~,Nf` extension.
///
/// The spelling is SRFI-28's rather than Janet's `%s`/`%.3f`, because a model
/// writing Scheme writes `~a` (plan §4.9). `~,3f` is Common Lisp's and is
/// here because a readout needs fixed precision and SRFI-28 has no directive
/// for it -- the corpus formats one on every drag event.
fn format(e: &mut Engine, a: &[Value]) -> PResult {
    let (fmt, args) = match a {
        [] => return Err(err("format: expects a format string")),
        [f, rest @ ..] => (*f, rest),
    };
    let cs = chars_of("format", e, fmt)?;
    let mut out = String::new();
    let mut next = 0usize;
    let mut i = 0usize;
    let take = |next: &mut usize| -> Result<Value, VmError> {
        let v = args
            .get(*next)
            .copied()
            .ok_or_else(|| err("format: not enough arguments for the directives"))?;
        *next += 1;
        Ok(v)
    };
    while i < cs.len() {
        if cs[i] != '~' {
            out.push(cs[i]);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&d) = cs.get(i) else {
            return Err(err("format: a `~` at the end of the string"));
        };
        match d {
            'a' | 'A' => {
                let v = take(&mut next)?;
                out.push_str(&display_str(e, v));
            }
            's' | 'S' => {
                let v = take(&mut next)?;
                out.push_str(&write_str(e, v));
            }
            'd' | 'D' => {
                let v = take(&mut next)?;
                want_number("format", v, e)?;
                out.push_str(&display_str(e, v));
            }
            '%' | 'n' => out.push('\n'),
            '~' => out.push('~'),
            // `~,Nf` -- fixed precision.
            ',' => {
                let mut digits = String::new();
                i += 1;
                while let Some(c) = cs.get(i) {
                    if c.is_ascii_digit() {
                        digits.push(*c);
                        i += 1;
                    } else {
                        break;
                    }
                }
                match cs.get(i) {
                    Some('f') | Some('F') => {
                        let prec: usize = digits.parse().unwrap_or(6);
                        let v = take(&mut next)?;
                        want_number("format", v, e)?;
                        let x = as_f64(v).expect("checked");
                        out.push_str(&format!("{x:.prec$}"));
                    }
                    _ => return Err(err("format: expected ~,Nf")),
                }
            }
            other => return Err(err(format!("format: unknown directive ~{other}"))),
        }
        i += 1;
    }
    if next < args.len() {
        return Err(err(format!(
            "format: {} argument(s) left over",
            args.len() - next
        )));
    }
    Ok(e.string(&out))
}

// ── errors and the remaining predicates ─────────────────────────────────

/// `(error message irritant …)` builds an **error object** and raises it, so
/// a `guard` can ask it for its message and irritants rather than being
/// handed a flattened string. It is still an ordinary refusal: the VM stays
/// usable, which is requirement 8.
fn error_prim(e: &mut Engine, a: &[Value]) -> PResult {
    let message = match a.first() {
        Some(v) => *v,
        None => e.string("error"),
    };
    let irritants = e.list(a.get(1..).unwrap_or(&[]));
    let obj = e.error_obj(message, irritants, ErrorKind::Plain);
    let text = write_str(e, obj);
    Err(VmError::Raised(obj, text))
}

/// `(raise obj)` -- the primitive underneath the prelude's `raise`, which
/// adds the handler-stack behaviour on top.
fn raise_prim(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("%raise", a)?;
    let text = format!("raised: {}", write_str(e, v));
    Err(VmError::Raised(v, text))
}

fn err_obj<'a>(who: &str, e: &'a Engine, v: Value) -> Result<&'a Obj, VmError> {
    match v {
        Value::Obj(r) if matches!(e.obj(r), Obj::ErrorObj { .. }) => Ok(e.obj(r)),
        _ => Err(err(format!(
            "{who}: not an error object: {}",
            write_str(e, v)
        ))),
    }
}

fn is_error_object(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("error-object?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::ErrorObj { .. })),
    ))
}

fn error_object_message(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("error-object-message", a)?;
    match err_obj("error-object-message", e, v)? {
        Obj::ErrorObj { message, .. } => Ok(*message),
        _ => unreachable!("checked"),
    }
}

fn error_object_irritants(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("error-object-irritants", a)?;
    match err_obj("error-object-irritants", e, v)? {
        Obj::ErrorObj { irritants, .. } => Ok(*irritants),
        _ => unreachable!("checked"),
    }
}

/// R7RS distinguishes read errors and file errors from other conditions. The
/// reader does not raise through this path yet and there is no file I/O, so
/// both answer honestly rather than guessing.
fn error_kind_is(e: &Engine, v: Value, want: ErrorKind) -> bool {
    matches!(v, Value::Obj(r)
        if matches!(e.obj(r), Obj::ErrorObj { kind, .. } if *kind == want))
}

fn is_read_error(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("read-error?", a)?;
    Ok(Value::Bool(error_kind_is(e, v, ErrorKind::Read)))
}

fn is_file_error(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("file-error?", a)?;
    Ok(Value::Bool(error_kind_is(e, v, ErrorKind::File)))
}

fn is_boolean(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(one("boolean?", a)?, Value::Bool(_))))
}

fn is_procedure(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("procedure?", a)?;
    Ok(Value::Bool(match v {
        Value::Primitive(_) => true,
        Value::Obj(r) => matches!(e.obj(r), Obj::Closure { .. }),
        _ => false,
    }))
}

fn is_exact_integer(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(
        one("exact-integer?", a)?,
        Value::Fixnum(_)
    )))
}

fn num_pred(who: &'static str, e: &Engine, a: &[Value], f: fn(f64) -> bool) -> PResult {
    let v = one(who, a)?;
    want_number(who, v, e)?;
    Ok(Value::Bool(f(as_f64(v).expect("checked"))))
}

fn is_positive(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("positive?", e, a, |x| x > 0.0)
}
fn is_negative(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("negative?", e, a, |x| x < 0.0)
}
fn is_even(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("even?", e, a, |x| x % 2.0 == 0.0)
}
fn is_odd(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("odd?", e, a, |x| x % 2.0 != 0.0)
}
fn is_nan(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("nan?", e, a, |x| x.is_nan())
}

// ── parameters ───────────────────────────────────────────────────────────

/// `(%make-parameter init [converter])`, the raw constructor.
///
/// R7RS says the parameter's initial value is `(converter init)`, and applying
/// a procedure needs the control stack -- so the prelude's `make-parameter`
/// wraps this and does the call in Scheme. Installing the raw one under a `%`
/// name keeps that ordering honest instead of silently storing an
/// unconverted init.
fn make_parameter(vm: &mut Vm, a: &[Value]) -> PResult {
    match a {
        [init] => Ok(vm.make_parameter(*init, None)),
        [init, conv] => Ok(vm.make_parameter(*init, Some(*conv))),
        _ => Err(err("make-parameter: expects an initial value and an optional converter")),
    }
}

fn param_converter(e: &mut Engine, a: &[Value]) -> PResult {
    match one("%param-converter", a)? {
        Value::Obj(r) => match e.obj(r) {
            Obj::Parameter { converter, .. } => Ok(converter.unwrap_or(Value::Bool(false))),
            _ => Err(err("%param-converter: not a parameter")),
        },
        _ => Err(err("%param-converter: not a parameter")),
    }
}

fn is_parameter(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("parameter?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::Parameter { .. })),
    ))
}

// ── randomness ───────────────────────────────────────────────────────────
//
// R7RS has no RNG at all, and the corpus needs one that is REPRODUCIBLE:
// `math/random` appears in 6 scene files but `:seed` in 17, and ray's
// `:ray-rng` is why `grow-branches` can be replayed (survey §4). So the
// generator is a *value* reachable through a parameter, never a global --
// which is exactly what lets `parameterize` scope a seed to one build.
//
// SRFI-27's names, because a model writing Scheme knows them.

/// SplitMix64. Small, fast, and good enough for scattering points; it is not
/// a cryptographic generator and nothing here pretends otherwise.
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn make_random_source(e: &mut Engine, a: &[Value]) -> PResult {
    let seed = match a {
        [] => 0x2545_F491_4F6C_DD1D,
        [Value::Fixnum(n)] => *n as u64,
        _ => return Err(err("make-random-source: expects an optional exact seed")),
    };
    Ok(e.alloc(Obj::Rng(seed)))
}

fn rng_ref(who: &str, vm: &Vm, v: Value) -> Result<ObjRef, VmError> {
    match v {
        Value::Obj(r) if matches!(vm.engine.obj(r), Obj::Rng(_)) => Ok(r),
        _ => Err(err(format!(
            "{who}: not a random source: {}",
            write_str(&vm.engine, v)
        ))),
    }
}

/// The current value of a global parameter, by name.
///
/// This is how a primitive honours `current-output-port` and
/// `current-random-source`: the default comes from the parameter, so
/// `parameterize` redirects it -- which is the same thing ray does with
/// `(with-dyns [:out buf] (snapshot))`.
fn param_now(vm: &Vm, name: &str) -> Result<Value, VmError> {
    let Some(sym) = vm.engine.interner.lookup(name) else {
        return Err(err(format!("{name} is not bound")));
    };
    let Some(param) = vm.global(sym) else {
        return Err(err(format!("{name} is not bound")));
    };
    match param {
        Value::Obj(r) => match vm.engine.obj(r) {
            Obj::Parameter { slot, .. } => Ok(vm.param_value(*slot)),
            _ => Err(err(format!("{name} is not a parameter"))),
        },
        _ => Err(err(format!("{name} is not a parameter"))),
    }
}

/// The source to use when none is given: the value of `current-random-source`.
fn current_source(vm: &mut Vm, who: &str, a: &[Value], at: usize) -> Result<ObjRef, VmError> {
    if let Some(v) = a.get(at) {
        return rng_ref(who, vm, *v);
    }
    let v = param_now(vm, "current-random-source")?;
    rng_ref(who, vm, v)
}

fn random_real(vm: &mut Vm, a: &[Value]) -> PResult {
    let r = current_source(vm, "random-real", a, 0)?;
    let Obj::Rng(state) = vm.engine.obj_mut(r) else {
        unreachable!("checked")
    };
    let n = next_u64(state);
    // 53 bits of mantissa, so the result is uniform in [0, 1).
    Ok(Value::Flonum((n >> 11) as f64 / (1u64 << 53) as f64))
}

fn random_integer(vm: &mut Vm, a: &[Value]) -> PResult {
    let (bound, at) = match a.first() {
        Some(Value::Fixnum(n)) if *n > 0 => (*n as u64, 1),
        _ => return Err(err("random-integer: expects a positive exact bound")),
    };
    let r = current_source(vm, "random-integer", a, at)?;
    let Obj::Rng(state) = vm.engine.obj_mut(r) else {
        unreachable!("checked")
    };
    // Rejection sampling, so the distribution is actually uniform rather
    // than biased toward the low values by a modulo.
    let limit = u64::MAX - (u64::MAX % bound);
    let mut n = next_u64(state);
    while n >= limit {
        n = next_u64(state);
    }
    Ok(Value::Fixnum((n % bound) as i64))
}

fn is_random_source(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("random-source?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::Rng(_))),
    ))
}

// ── quasiquote's runtime helpers ────────────────────────────────────────
//
// `quasiquote` expands into calls to these rather than to `cons`, `append`
// and `list->vector`, because the expansion is built in Rust and would
// otherwise reference whatever those names mean *at the use site*. A user
// writing `(let ([cons list]) `(1 ,x))` would then get something surprising.
// The `%` names cannot be shadowed by accident.

fn qq_cons(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [car, cdr] => Ok(e.cons(*car, *cdr)),
        _ => Err(err("%qq-cons: expects two arguments")),
    }
}

/// Append one list to a tail, which may be improper -- `` `(1 ,@x . 2) ``.
fn qq_append(e: &mut Engine, a: &[Value]) -> PResult {
    let (list, tail) = match a {
        [l, t] => (*l, *t),
        _ => return Err(err("%qq-append: expects two arguments")),
    };
    let mut items = Vec::new();
    let mut cur = list;
    loop {
        match cur {
            Value::Null => break,
            Value::Obj(r) => match e.obj(r) {
                Obj::Pair(car, cdr) => {
                    items.push(*car);
                    cur = *cdr;
                }
                // Splicing a vector is an extension, and a convenient one
                // when a scene has accumulated its points into one.
                Obj::Vector { items: vs, .. } => {
                    items.extend(vs.iter().copied());
                    break;
                }
                _ => return Err(err("unquote-splicing: not a list")),
            },
            _ => return Err(err("unquote-splicing: not a list")),
        }
    }
    let mut acc = tail;
    for v in items.into_iter().rev() {
        acc = e.cons(v, acc);
    }
    Ok(acc)
}

/// Build a vector from a list, remembering which brackets it was written
/// with -- `` `[1 ,x] `` must stay a bracket vector and `` `#(1 ,x) `` must
/// not (plan §4.5).
fn qq_list_to_vector(e: &mut Engine, a: &[Value]) -> PResult {
    let (list, bracket) = match a {
        [l, b] => (*l, b.is_truthy()),
        _ => return Err(err("%qq-list->vector: expects a list and a flag")),
    };
    let mut items = Vec::new();
    let mut cur = list;
    loop {
        match cur {
            Value::Null => break,
            Value::Obj(r) => match *e.obj(r) {
                Obj::Pair(car, cdr) => {
                    items.push(car);
                    cur = cdr;
                }
                _ => return Err(err("%qq-list->vector: not a proper list")),
            },
            _ => return Err(err("%qq-list->vector: not a proper list")),
        }
    }
    Ok(e.vector(items, bracket))
}


// ── ports ────────────────────────────────────────────────────────────────
//
// Textual ports only. Most of what the R7RS suite exercises through them is
// the reader and the printer that already existed since S1 -- `read` is this
// reader pointed at a port, and `(write x out)` plus `get-output-string` is
// how the suite reaches the writer. So this block mostly *exposes* work
// already done and tested in Rust.
//
// `current-output-port` is a parameter, which is what makes
// `(parameterize ([current-output-port buf]) …)` work -- the same thing ray
// does with `(with-dyns [:out buf] (snapshot))`.

fn port_ref(who: &str, e: &Engine, v: Value) -> Result<crate::value::ObjRef, VmError> {
    match v {
        Value::Obj(r) if matches!(e.obj(r), Obj::Port { .. }) => Ok(r),
        _ => Err(err(format!("{who}: not a port: {}", write_str(e, v)))),
    }
}

fn open_input_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("open-input-string", a)?;
    let chars = chars_of("open-input-string", e, v)?;
    Ok(e.port(PortKind::StringIn { chars, pos: 0 }))
}

fn open_output_string(e: &mut Engine, a: &[Value]) -> PResult {
    if !a.is_empty() {
        return Err(err("open-output-string: takes no arguments"));
    }
    Ok(e.port(PortKind::StringOut(String::new())))
}

fn get_output_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("get-output-string", a)?;
    let r = port_ref("get-output-string", e, v)?;
    let text = match e.obj(r) {
        Obj::Port {
            kind: PortKind::StringOut(s),
            ..
        } => s.clone(),
        _ => return Err(err("get-output-string: not a string output port")),
    };
    Ok(e.string(&text))
}

/// Write text to a port. The one place output actually leaves the system.
fn emit(e: &mut Engine, port: Value, text: &str) -> Result<(), VmError> {
    let r = port_ref("write", e, port)?;
    match e.obj_mut(r) {
        Obj::Port { open: false, .. } => Err(err("write: the port is closed")),
        Obj::Port {
            kind: PortKind::StringOut(s),
            ..
        } => {
            s.push_str(text);
            Ok(())
        }
        Obj::Port {
            kind: PortKind::Stdout,
            ..
        } => {
            print!("{text}");
            Ok(())
        }
        Obj::Port {
            kind: PortKind::Stderr,
            ..
        } => {
            eprint!("{text}");
            Ok(())
        }
        _ => Err(err("write: not an output port")),
    }
}

fn read_char(e: &mut Engine, a: &[Value], peek: bool) -> PResult {
    let v = one(if peek { "peek-char" } else { "read-char" }, a)?;
    let r = port_ref("read-char", e, v)?;
    match e.obj_mut(r) {
        Obj::Port { open: false, .. } => Err(err("read-char: the port is closed")),
        Obj::Port {
            kind: PortKind::StringIn { chars, pos },
            ..
        } => match chars.get(*pos).copied() {
            Some(c) => {
                if !peek {
                    *pos += 1;
                }
                Ok(Value::Char(c))
            }
            None => Ok(Value::Eof),
        },
        _ => Err(err("read-char: not a textual input port")),
    }
}

fn read_line(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("read-line", a)?;
    let r = port_ref("read-line", e, v)?;
    let line = match e.obj_mut(r) {
        Obj::Port {
            kind: PortKind::StringIn { chars, pos },
            ..
        } => {
            if *pos >= chars.len() {
                None
            } else {
                let mut out = String::new();
                while *pos < chars.len() {
                    let c = chars[*pos];
                    *pos += 1;
                    if c == '\n' {
                        break;
                    }
                    out.push(c);
                }
                Some(out)
            }
        }
        _ => return Err(err("read-line: not a textual input port")),
    };
    match line {
        Some(text) => Ok(e.string(&text)),
        None => Ok(Value::Eof),
    }
}

/// `read`: this reader, pointed at a port, advancing its cursor by exactly
/// one datum. A truncated datum is a **read error**, which `read-error?`
/// can then identify — the suite tests exactly that.
fn read_datum(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("read", a)?;
    let r = port_ref("read", e, v)?;
    let (text, base) = match e.obj(r) {
        Obj::Port {
            kind: PortKind::StringIn { chars, pos },
            ..
        } => (chars[*pos..].iter().collect::<String>(), *pos),
        _ => return Err(err("read: not a textual input port")),
    };
    let mut reader = crate::reader::Reader::new(&text);
    match reader.read(e) {
        Ok(None) => Ok(Value::Eof),
        Ok(Some(datum)) => {
            let consumed = reader.consumed();
            if let Obj::Port {
                kind: PortKind::StringIn { pos, .. },
                ..
            } = e.obj_mut(r)
            {
                *pos = base + consumed;
            }
            Ok(datum)
        }
        Err(re) => {
            let msg = e.string(&format!("read: {re}"));
            let obj = e.error_obj(msg, Value::Null, ErrorKind::Read);
            let shown = write_str(e, obj);
            Err(VmError::Raised(obj, shown))
        }
    }
}

fn stdout_port(e: &mut Engine, _a: &[Value]) -> PResult {
    Ok(e.port(PortKind::Stdout))
}

fn stderr_port(e: &mut Engine, _a: &[Value]) -> PResult {
    Ok(e.port(PortKind::Stderr))
}

fn eof_object(_e: &mut Engine, a: &[Value]) -> PResult {
    if !a.is_empty() {
        return Err(err("eof-object: takes no arguments"));
    }
    Ok(Value::Eof)
}

fn is_eof_object(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(matches!(one("eof-object?", a)?, Value::Eof)))
}

fn port_pred(who: &'static str, e: &Engine, a: &[Value], f: fn(&PortKind) -> bool) -> PResult {
    let v = one(who, a)?;
    Ok(Value::Bool(match v {
        Value::Obj(r) => match e.obj(r) {
            Obj::Port { kind, .. } => f(kind),
            _ => false,
        },
        _ => false,
    }))
}

fn is_port(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("port?", e, a, |_| true)
}
fn is_input_port(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("input-port?", e, a, |k| {
        matches!(k, PortKind::StringIn { .. })
    })
}
fn is_output_port(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("output-port?", e, a, |k| {
        !matches!(k, PortKind::StringIn { .. })
    })
}
fn is_textual_port(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("textual-port?", e, a, |_| true)
}
/// There are no binary ports, and saying so is better than implying there are.
fn is_binary_port(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("binary-port?", e, a, |_| false)
}

fn port_open(who: &'static str, e: &Engine, a: &[Value], input: bool) -> PResult {
    let v = one(who, a)?;
    Ok(Value::Bool(match v {
        Value::Obj(r) => match e.obj(r) {
            Obj::Port { kind, open } => {
                *open && (matches!(kind, PortKind::StringIn { .. }) == input)
            }
            _ => false,
        },
        _ => false,
    }))
}

fn is_input_port_open(e: &mut Engine, a: &[Value]) -> PResult {
    port_open("input-port-open?", e, a, true)
}
fn is_output_port_open(e: &mut Engine, a: &[Value]) -> PResult {
    port_open("output-port-open?", e, a, false)
}

fn close_port(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("close-port", a)?;
    let r = port_ref("close-port", e, v)?;
    if let Obj::Port { open, .. } = e.obj_mut(r) {
        *open = false;
    }
    Ok(Value::Unspecified)
}

/// `char-ready?` is true for a string port: the characters are already here.
fn is_char_ready(e: &mut Engine, a: &[Value]) -> PResult {
    port_pred("char-ready?", e, a, |k| {
        matches!(k, PortKind::StringIn { .. })
    })
}


// ── the cheap tail ───────────────────────────────────────────────────────
//
// Thirty-odd procedures the R7RS suite named, with no design in any of them.
// Picked from the measured failure list rather than from the spec's table of
// contents, which is why `sort` and `get-in` are still absent: nothing asked
// for them.
//
// Two things deliberately NOT here. `make-rectangular` and friends are
// `(scheme complex)`, a whole tower §2.3 does not claim -- 23 assertions, and
// they belong in the explained remainder. `let-syntax` needs a macro alias to
// carry its own environment, which syntax.rs explicitly deferred.


/// Optional `start`/`end`, which R7RS allows on most sequence procedures --
/// several suite failures were only this.
fn range_of(who: &str, a: &[Value], at: usize, len: usize) -> Result<(usize, usize), VmError> {
    let start = match a.get(at) {
        Some(Value::Fixnum(n)) if *n >= 0 => *n as usize,
        None => 0,
        _ => return Err(err(format!("{who}: start must be a non-negative exact integer"))),
    };
    let end = match a.get(at + 1) {
        Some(Value::Fixnum(n)) if *n >= 0 => *n as usize,
        None => len,
        _ => return Err(err(format!("{who}: end must be a non-negative exact integer"))),
    };
    if end > len || start > end {
        return Err(err(format!("{who}: range {start}..{end} is out of bounds for {len}")));
    }
    Ok((start, end))
}

fn p_is_real(_e: &mut Engine, a: &[Value]) -> PResult {
    Ok(Value::Bool(as_f64(one("real?", a)?).is_some()))
}

/// Without `ratios` every number is either an exact integer or a flonum, so
/// `rational?` is true of anything finite -- and saying that plainly is
/// better than pretending there is a rational type.
fn p_is_rational(_e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("rational?", a)?;
    Ok(Value::Bool(match as_f64(v) {
        Some(x) => x.is_finite(),
        None => false,
    }))
}

fn p_is_integer(_e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("integer?", a)?;
    Ok(Value::Bool(match v {
        Value::Fixnum(_) => true,
        Value::Flonum(x) => x.fract() == 0.0 && x.is_finite(),
        _ => false,
    }))
}

fn p_is_inexact(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("inexact?", a)?;
    want_number("inexact?", v, e)?;
    Ok(Value::Bool(matches!(v, Value::Flonum(_))))
}

fn p_is_finite(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("finite?", e, a, |x| x.is_finite())
}

fn p_is_infinite(e: &mut Engine, a: &[Value]) -> PResult {
    num_pred("infinite?", e, a, |x| x.is_infinite())
}

/// `exact` of an integral flonum is the integer; of anything else it would
/// need a rational, which §2.3 does not claim, so it says so.
fn p_exact(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("exact", a)?;
    want_number("exact", v, e)?;
    match v {
        Value::Fixnum(_) => Ok(v),
        Value::Flonum(x) if x.fract() == 0.0 && x.abs() < (i64::MAX as f64) => {
            Ok(Value::Fixnum(x as i64))
        }
        _ => Err(err(format!(
            "exact: {} would need an exact rational (the `ratios` feature is absent)",
            write_str(e, v)
        ))),
    }
}

fn p_square(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("square", a)?;
    want_number("square", v, e)?;
    match v {
        Value::Fixnum(n) => n
            .checked_mul(n)
            .map(Value::Fixnum)
            .ok_or_else(|| err("square: exact integer overflow")),
        _ => {
            let x = as_f64(v).expect("checked");
            Ok(Value::Flonum(x * x))
        }
    }
}

fn p_gcd(e: &mut Engine, a: &[Value]) -> PResult {
    let mut g: i64 = 0;
    for &v in a {
        let Value::Fixnum(n) = v else {
            return Err(err(format!("gcd: expects exact integers, got {}", write_str(e, v))));
        };
        let (mut x, mut y) = (g.abs(), n.abs());
        while y != 0 {
            let t = y;
            y = x % y;
            x = t;
        }
        g = x;
    }
    Ok(Value::Fixnum(g))
}

fn p_lcm(e: &mut Engine, a: &[Value]) -> PResult {
    let mut l: i64 = 1;
    for &v in a {
        let Value::Fixnum(n) = v else {
            return Err(err(format!("lcm: expects exact integers, got {}", write_str(e, v))));
        };
        if n == 0 {
            return Ok(Value::Fixnum(0));
        }
        let (mut x, mut y) = (l.abs(), n.abs());
        while y != 0 {
            let t = y;
            y = x % y;
            x = t;
        }
        l = (l / x).saturating_mul(n).abs();
    }
    Ok(Value::Fixnum(l))
}

/// `(exact-integer-sqrt n)` returns two values: the root and the remainder.
fn p_exact_integer_sqrt(e: &mut Engine, a: &[Value]) -> PResult {
    match one("exact-integer-sqrt", a)? {
        Value::Fixnum(n) if n >= 0 => {
            // Checked, because `(r + 1) * (r + 1)` overflows near i64::MAX and
            // wraps negative -- which made the loop below run forever. Found
            // by the R7RS suite, which tests the boundary.
            let mut r = (n as f64).sqrt() as i64;
            while r > 0 && r.checked_mul(r).is_none_or(|sq| sq > n) {
                r -= 1;
            }
            while (r + 1)
                .checked_mul(r + 1)
                .is_some_and(|sq| sq <= n)
            {
                r += 1;
            }
            Ok(e.alloc(Obj::Values(vec![
                Value::Fixnum(r),
                Value::Fixnum(n - r * r),
            ])))
        }
        _ => Err(err("exact-integer-sqrt: expects a non-negative exact integer")),
    }
}

/// `floor/` and `truncate/` each return the quotient and remainder as two
/// values, which is why they needed `values` to exist first.
fn div_pair(who: &'static str, e: &mut Engine, a: &[Value], floor: bool) -> PResult {
    let (x, y) = int_pair(who, a)?;
    // The floor remainder takes the sign of the divisor, which is the same
    // definition `modulo` needed -- and the same place `rem_euclid` was wrong.
    let (q, r) = if floor {
        let r = ((x % y) + y) % y;
        ((x - r) / y, r)
    } else {
        (x / y, x % y)
    };
    Ok(e.alloc(Obj::Values(vec![Value::Fixnum(q), Value::Fixnum(r)])))
}

fn p_floor_div(e: &mut Engine, a: &[Value]) -> PResult {
    div_pair("floor/", e, a, true)
}
fn p_truncate_div(e: &mut Engine, a: &[Value]) -> PResult {
    div_pair("truncate/", e, a, false)
}
fn p_floor_quotient(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("floor-quotient", a)?;
    Ok(Value::Fixnum((x - (((x % y) + y) % y)) / y))
}
fn p_floor_remainder(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("floor-remainder", a)?;
    Ok(Value::Fixnum(((x % y) + y) % y))
}

/// Without `ratios`, every exact number is an integer: its numerator is
/// itself and its denominator is 1.
fn p_numerator(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("numerator", a)?;
    match v {
        Value::Fixnum(_) => Ok(v),
        Value::Flonum(x) if x.fract() == 0.0 => Ok(Value::Flonum(x)),
        _ => Err(err(format!(
            "numerator: {} would need an exact rational (the `ratios` feature is absent)",
            write_str(e, v)
        ))),
    }
}

fn p_denominator(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("denominator", a)?;
    match v {
        Value::Fixnum(_) => Ok(Value::Fixnum(1)),
        Value::Flonum(x) if x.fract() == 0.0 => Ok(Value::Flonum(1.0)),
        _ => Err(err(format!(
            "denominator: {} would need an exact rational (the `ratios` feature is absent)",
            write_str(e, v)
        ))),
    }
}

// ---- characters -------------------------------------------------------

fn char_le(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char<=?", a, |x, y| x <= y)
}
fn char_ge(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char>=?", a, |x, y| x >= y)
}
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}
fn char_ci_eq(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char-ci=?", a, |x, y| fold(x) == fold(y))
}
fn char_ci_lt(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char-ci<?", a, |x, y| fold(x) < fold(y))
}
fn char_ci_gt(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char-ci>?", a, |x, y| fold(x) > fold(y))
}
fn char_ci_le(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char-ci<=?", a, |x, y| fold(x) <= fold(y))
}
fn char_ci_ge(_e: &mut Engine, a: &[Value]) -> PResult {
    char_cmp("char-ci>=?", a, |x, y| fold(x) >= fold(y))
}
fn char_upper(_e: &mut Engine, a: &[Value]) -> PResult {
    char_pred("char-upper-case?", a, |c| c.is_uppercase())
}
fn char_lower(_e: &mut Engine, a: &[Value]) -> PResult {
    char_pred("char-lower-case?", a, |c| c.is_lowercase())
}
fn char_foldcase(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Char(c)] => Ok(Value::Char(fold(*c))),
        _ => Err(err("char-foldcase: expects one character")),
    }
}

/// `digit-value` is the numeric value of a digit character, or `#f`.
fn digit_value(_e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Char(c)] => Ok(match c.to_digit(10) {
            Some(d) => Value::Fixnum(d as i64),
            None => Value::Bool(false),
        }),
        _ => Err(err("digit-value: expects one character")),
    }
}

// ---- strings ----------------------------------------------------------

fn string_le(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string<=?", e, a, |o| o != std::cmp::Ordering::Greater, false)
}
fn string_ge(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string>=?", e, a, |o| o != std::cmp::Ordering::Less, false)
}
fn string_ci_lt(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string-ci<?", e, a, |o| o == std::cmp::Ordering::Less, true)
}
fn string_ci_gt(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string-ci>?", e, a, |o| o == std::cmp::Ordering::Greater, true)
}
fn string_ci_le(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string-ci<=?", e, a, |o| o != std::cmp::Ordering::Greater, true)
}
fn string_ci_ge(e: &mut Engine, a: &[Value]) -> PResult {
    string_compare("string-ci>=?", e, a, |o| o != std::cmp::Ordering::Less, true)
}

fn string_foldcase(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string-foldcase", a)?;
    let cs: Vec<char> = chars_of("string-foldcase", e, v)?
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect();
    Ok(e.alloc(Obj::Str(cs)))
}

/// `(string-copy! to at from [start [end]])`.
fn string_copy_into(e: &mut Engine, a: &[Value]) -> PResult {
    let (to, at, from) = match a {
        [to, Value::Fixnum(at), from, ..] => (*to, *at as usize, *from),
        _ => return Err(err("string-copy!: expects (to at from [start [end]])")),
    };
    let src = chars_of("string-copy!", e, from)?;
    let start = match a.get(3) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => 0,
        _ => return Err(err("string-copy!: start must be an exact integer")),
    };
    let end = match a.get(4) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => src.len(),
        _ => return Err(err("string-copy!: end must be an exact integer")),
    };
    let r = str_ref("string-copy!", e, to)?;
    let Obj::Str(dst) = e.obj_mut(r) else {
        unreachable!("checked")
    };
    if end > src.len() || start > end || at + (end - start) > dst.len() {
        return Err(err("string-copy!: range out of bounds"));
    }
    dst[at..at + (end - start)].copy_from_slice(&src[start..end]);
    Ok(Value::Unspecified)
}

fn string_to_vector(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("string->vector", a)?;
    let items: Vec<Value> = chars_of("string->vector", e, v)?
        .into_iter()
        .map(Value::Char)
        .collect();
    Ok(e.vector(items, true))
}

fn vector_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("vector->string", a)?;
    let r = vec_ref("vector->string", e, v)?;
    let Obj::Vector { items, .. } = e.obj(r) else {
        unreachable!("checked")
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items.clone() {
        match item {
            Value::Char(c) => out.push(c),
            _ => return Err(err("vector->string: not a vector of characters")),
        }
    }
    Ok(e.alloc(Obj::Str(out)))
}

// ---- vectors and lists ------------------------------------------------

/// `(vector-copy! to at from [start [end]])`.
fn vector_copy_into(e: &mut Engine, a: &[Value]) -> PResult {
    let (to, at, from) = match a {
        [to, Value::Fixnum(at), from, ..] => (*to, *at as usize, *from),
        _ => return Err(err("vector-copy!: expects (to at from [start [end]])")),
    };
    let rf = vec_ref("vector-copy!", e, from)?;
    let Obj::Vector { items: src, .. } = e.obj(rf) else {
        unreachable!("checked")
    };
    let src = src.clone();
    let start = match a.get(3) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => 0,
        _ => return Err(err("vector-copy!: start must be an exact integer")),
    };
    let end = match a.get(4) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => src.len(),
        _ => return Err(err("vector-copy!: end must be an exact integer")),
    };
    let rt = vec_ref("vector-copy!", e, to)?;
    let Obj::Vector { items: dst, .. } = e.obj_mut(rt) else {
        unreachable!("checked")
    };
    if end > src.len() || start > end || at + (end - start) > dst.len() {
        return Err(err("vector-copy!: range out of bounds"));
    }
    dst[at..at + (end - start)].copy_from_slice(&src[start..end]);
    Ok(Value::Unspecified)
}

fn vector_append(e: &mut Engine, a: &[Value]) -> PResult {
    let mut out = Vec::new();
    for &v in a {
        let r = vec_ref("vector-append", e, v)?;
        let Obj::Vector { items, .. } = e.obj(r) else {
            unreachable!("checked")
        };
        out.extend(items.iter().copied());
    }
    Ok(e.vector(out, true))
}

fn set_car(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Obj(r), v] => {
            let (r, v) = (*r, *v);
            match e.obj_mut(r) {
                Obj::Pair(car, _) => {
                    *car = v;
                    Ok(Value::Unspecified)
                }
                _ => Err(err("set-car!: not a pair")),
            }
        }
        _ => Err(err("set-car!: expects a pair and a value")),
    }
}

fn set_cdr(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [Value::Obj(r), v] => {
            let (r, v) = (*r, *v);
            match e.obj_mut(r) {
                Obj::Pair(_, cdr) => {
                    *cdr = v;
                    Ok(Value::Unspecified)
                }
                _ => Err(err("set-cdr!: not a pair")),
            }
        }
        _ => Err(err("set-cdr!: expects a pair and a value")),
    }
}

// ---- equality ---------------------------------------------------------

fn boolean_eq(_e: &mut Engine, a: &[Value]) -> PResult {
    if a.len() < 2 {
        return Err(err("boolean=?: needs at least two booleans"));
    }
    let mut prev = None;
    for &v in a {
        let Value::Bool(b) = v else {
            return Err(err("boolean=?: expects booleans"));
        };
        if let Some(p) = prev
            && p != b
        {
            return Ok(Value::Bool(false));
        }
        prev = Some(b);
    }
    Ok(Value::Bool(true))
}

fn symbol_eq(e: &mut Engine, a: &[Value]) -> PResult {
    if a.len() < 2 {
        return Err(err("symbol=?: needs at least two symbols"));
    }
    let mut prev = None;
    for &v in a {
        let Value::Symbol(sid) = v else {
            return Err(err("symbol=?: expects symbols"));
        };
        let r = e.resolve(sid);
        if let Some(p) = prev
            && p != r
        {
            return Ok(Value::Bool(false));
        }
        prev = Some(r);
    }
    Ok(Value::Bool(true))
}

// ---- bytevectors ------------------------------------------------------
//
// The heap object has existed since S1 for the reader's `#u8(…)`; these are
// the procedures over it.

fn bv_ref(who: &str, e: &Engine, v: Value) -> Result<crate::value::ObjRef, VmError> {
    match v {
        Value::Obj(r) if matches!(e.obj(r), Obj::Bytevector(_)) => Ok(r),
        _ => Err(err(format!("{who}: not a bytevector: {}", write_str(e, v)))),
    }
}

fn bytes_of(who: &str, e: &Engine, v: Value) -> Result<Vec<u8>, VmError> {
    let r = bv_ref(who, e, v)?;
    let Obj::Bytevector(b) = e.obj(r) else {
        unreachable!("checked")
    };
    Ok(b.clone())
}

fn bv_new(e: &mut Engine, a: &[Value]) -> PResult {
    let mut out = Vec::with_capacity(a.len());
    for &v in a {
        match v {
            Value::Fixnum(n) if (0..=255).contains(&n) => out.push(n as u8),
            _ => return Err(err("bytevector: elements must be exact 0..255")),
        }
    }
    Ok(e.bytevector(out))
}

fn bv_make(e: &mut Engine, a: &[Value]) -> PResult {
    let (n, fill) = match a {
        [Value::Fixnum(n)] => (*n, 0u8),
        [Value::Fixnum(n), Value::Fixnum(f)] if (0..=255).contains(f) => (*n, *f as u8),
        _ => return Err(err("make-bytevector: expects a length and an optional byte")),
    };
    if n < 0 {
        return Err(err("make-bytevector: negative length"));
    }
    Ok(e.bytevector(vec![fill; n as usize]))
}

fn bv_is(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("bytevector?", a)?;
    Ok(Value::Bool(
        matches!(v, Value::Obj(r) if matches!(e.obj(r), Obj::Bytevector(_))),
    ))
}

fn bv_length(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("bytevector-length", a)?;
    Ok(Value::Fixnum(bytes_of("bytevector-length", e, v)?.len() as i64))
}

fn bv_u8_ref(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, Value::Fixnum(i)] => {
            let b = bytes_of("bytevector-u8-ref", e, *v)?;
            b.get(*i as usize)
                .map(|x| Value::Fixnum(*x as i64))
                .ok_or_else(|| err(format!("bytevector-u8-ref: index {i} out of range")))
        }
        _ => Err(err("bytevector-u8-ref: expects a bytevector and an index")),
    }
}

fn bv_u8_set(e: &mut Engine, a: &[Value]) -> PResult {
    match a {
        [v, Value::Fixnum(i), Value::Fixnum(x)] if (0..=255).contains(x) => {
            let r = bv_ref("bytevector-u8-set!", e, *v)?;
            let (i, x) = (*i as usize, *x as u8);
            let Obj::Bytevector(b) = e.obj_mut(r) else {
                unreachable!("checked")
            };
            match b.get_mut(i) {
                Some(slot) => {
                    *slot = x;
                    Ok(Value::Unspecified)
                }
                None => Err(err(format!("bytevector-u8-set!: index {i} out of range"))),
            }
        }
        _ => Err(err("bytevector-u8-set!: expects a bytevector, an index and a byte")),
    }
}

fn bv_copy(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("bytevector-copy: expects a bytevector")),
    };
    let b = bytes_of("bytevector-copy", e, v)?;
    let start = match a.get(1) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => 0,
        _ => return Err(err("bytevector-copy: start must be an exact integer")),
    };
    let end = match a.get(2) {
        Some(Value::Fixnum(n)) => *n as usize,
        None => b.len(),
        _ => return Err(err("bytevector-copy: end must be an exact integer")),
    };
    if end > b.len() || start > end {
        return Err(err("bytevector-copy: range out of bounds"));
    }
    Ok(e.bytevector(b[start..end].to_vec()))
}

fn bv_append(e: &mut Engine, a: &[Value]) -> PResult {
    let mut out = Vec::new();
    for &v in a {
        out.extend(bytes_of("bytevector-append", e, v)?);
    }
    Ok(e.bytevector(out))
}

fn utf8_to_string(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("utf8->string: expects a bytevector")),
    };
    let b = bytes_of("utf8->string", e, v)?;
    let (start, end) = range_of("utf8->string", a, 1, b.len())?;
    match String::from_utf8(b[start..end].to_vec()) {
        Ok(text) => Ok(e.string(&text)),
        Err(_) => Err(err("utf8->string: not valid UTF-8")),
    }
}

fn string_to_utf8(e: &mut Engine, a: &[Value]) -> PResult {
    let v = match a.first() {
        Some(v) => *v,
        None => return Err(err("string->utf8: expects a string")),
    };
    let cs = chars_of("string->utf8", e, v)?;
    // The range is in CHARACTERS, and the result in bytes.
    let (start, end) = range_of("string->utf8", a, 1, cs.len())?;
    let text: String = cs[start..end].iter().collect();
    Ok(e.bytevector(text.into_bytes()))
}

// ── the system interface, and `features` ────────────────────────────────
//
// §2.3 committed to `(features)` being HONEST -- that is the whole obligation
// taken on by not implementing the numeric tower -- and until now it did not
// exist at all, which is worse than answering wrongly.

fn p_features(e: &mut Engine, _a: &[Value]) -> PResult {
    let mut names: Vec<&str> = vec!["r7rs", "ieee-float", "scm"];
    // NOT claimed, and each absence is a decision recorded in the plan:
    //   exact-closed  -- exact arithmetic raises on overflow (no bignums)
    //   ratios        -- no exact rationals (§2.3)
    //   full-unicode  -- no case folding or Unicode digit values (§2.3)
    //   exact-complex -- no complex numbers at all
    if cfg!(unix) {
        names.push("posix");
        names.push("unix");
    }
    if cfg!(target_os = "macos") {
        names.push("darwin");
    }
    if cfg!(target_os = "linux") {
        names.push("gnu-linux");
    }
    if cfg!(target_arch = "aarch64") {
        names.push("aarch64");
    }
    if cfg!(target_arch = "x86_64") {
        names.push("x86-64");
    }
    if cfg!(target_pointer_width = "64") {
        names.push("lp64");
    }
    let syms: Vec<Value> = names.iter().map(|n| e.sym(n)).collect();
    Ok(e.list(&syms))
}

fn p_get_env(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("get-environment-variable", a)?;
    let name: String = chars_of("get-environment-variable", e, v)?.into_iter().collect();
    match std::env::var(&name) {
        Ok(val) => Ok(e.string(&val)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn p_get_envs(e: &mut Engine, _a: &[Value]) -> PResult {
    let pairs: Vec<Value> = std::env::vars()
        .map(|(k, v)| {
            let k = e.string(&k);
            let v = e.string(&v);
            e.cons(k, v)
        })
        .collect();
    Ok(e.list(&pairs))
}

fn p_command_line(e: &mut Engine, _a: &[Value]) -> PResult {
    let args: Vec<Value> = std::env::args().map(|a| e.string(&a)).collect();
    Ok(e.list(&args))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn p_current_second(_e: &mut Engine, _a: &[Value]) -> PResult {
    Ok(Value::Flonum(now_secs()))
}

/// Jiffies are microseconds here, which `jiffies-per-second` reports.
fn p_current_jiffy(_e: &mut Engine, _a: &[Value]) -> PResult {
    Ok(Value::Fixnum((now_secs() * 1e6) as i64))
}

fn p_jiffies_per_second(_e: &mut Engine, _a: &[Value]) -> PResult {
    Ok(Value::Fixnum(1_000_000))
}

fn p_exit(_e: &mut Engine, a: &[Value]) -> PResult {
    let code = match a.first() {
        Some(Value::Fixnum(n)) => *n as i32,
        Some(Value::Bool(false)) => 1,
        _ => 0,
    };
    std::process::exit(code);
}

// ── files ───────────────────────────────────────────────────────────────
//
// Enough of `(scheme file)` to be truthful: a missing file raises a FILE
// error, which `file-error?` can then identify -- the suite tests exactly
// that, and stubbing it would have been a lie dressed as a pass.

fn file_err(e: &mut Engine, msg: &str) -> VmError {
    let m = e.string(msg);
    let obj = e.error_obj(m, Value::Null, ErrorKind::File);
    let text = write_str(e, obj);
    VmError::Raised(obj, text)
}

fn p_file_exists(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("file-exists?", a)?;
    let path: String = chars_of("file-exists?", e, v)?.into_iter().collect();
    Ok(Value::Bool(std::path::Path::new(&path).exists()))
}

fn p_delete_file(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("delete-file", a)?;
    let path: String = chars_of("delete-file", e, v)?.into_iter().collect();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Value::Unspecified),
        Err(err_io) => Err(file_err(e, &format!("delete-file: {path}: {err_io}"))),
    }
}

fn p_open_input_file(e: &mut Engine, a: &[Value]) -> PResult {
    let v = one("open-input-file", a)?;
    let path: String = chars_of("open-input-file", e, v)?.into_iter().collect();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(e.port(PortKind::StringIn {
            chars: text.chars().collect(),
            pos: 0,
        })),
        Err(err_io) => Err(file_err(e, &format!("open-input-file: {path}: {err_io}"))),
    }
}

pub fn install(vm: &mut Vm) {
    vm.define_prim("+", add);
    vm.define_prim("-", sub);
    vm.define_prim("*", mul);
    vm.define_prim("/", div);
    vm.define_prim("=", num_eq);
    vm.define_prim("<", lt);
    vm.define_prim(">", gt);
    vm.define_prim("<=", le);
    vm.define_prim(">=", ge);
    vm.define_prim("cons", cons);
    vm.define_prim("car", car);
    vm.define_prim("cdr", cdr);
    vm.define_prim("null?", is_null);
    vm.define_prim("pair?", is_pair);
    vm.define_prim("not", not);
    vm.define_prim("zero?", is_zero);
    vm.define_prim("eq?", eq);
    vm.define_prim("eqv?", eqv);
    vm.define_prim("equal?", equal);
    vm.define_prim("list", list);
    vm.define_prim("length", length);
    vm.define_prim("vector", vector);
    vm.define_prim("vector-ref", vector_ref);
    vm.define_prim("vector-length", vector_length);
    vm.define_prim("keyword?", is_keyword);
    vm.define_prim("symbol?", is_symbol);
    vm.define_prim("apply", apply_placeholder);
    vm.define_prim("values", values_prim);
    vm.define_prim("call-with-values", apply_placeholder);
    vm.define_prim("eval", apply_placeholder);
    vm.define_prim("sin", p_sin);
    vm.define_prim("cos", p_cos);
    vm.define_prim("tan", p_tan);
    vm.define_prim("asin", p_asin);
    vm.define_prim("acos", p_acos);
    vm.define_prim("atan", p_atan);
    vm.define_prim("exp", p_exp);
    vm.define_prim("log", p_log);
    vm.define_prim("sqrt", p_sqrt);
    vm.define_prim("expt", p_expt);
    vm.define_prim("floor", p_floor);
    vm.define_prim("ceiling", p_ceiling);
    vm.define_prim("truncate", p_truncate);
    vm.define_prim("round", p_round);
    vm.define_prim("abs", p_abs);
    vm.define_prim("quotient", p_quotient);
    vm.define_prim("remainder", p_remainder);
    vm.define_prim("modulo", p_modulo);
    vm.define_prim("min", p_min);
    vm.define_prim("max", p_max);
    vm.define_prim("exact->inexact", p_exact_to_inexact);
    vm.define_prim("inexact", p_exact_to_inexact);
    vm.define_prim("exact?", p_is_exact);
    vm.define_prim("number?", p_is_number);
    vm.define_vm_prim("display", display);
    vm.define_vm_prim("write", write_prim);
    vm.define_vm_prim("newline", newline);
    vm.define_vm_prim("write-simple", write_simple);
    vm.define_vm_prim("write-shared", write_shared);
    vm.define_vm_prim("write-char", write_char);
    vm.define_vm_prim("write-string", write_string);
    // ports
    vm.define_prim("open-input-string", open_input_string);
    vm.define_prim("open-output-string", open_output_string);
    vm.define_prim("get-output-string", get_output_string);
    vm.define_prim("read", read_datum);
    vm.define_prim("read-char", read_char_prim);
    vm.define_prim("peek-char", peek_char_prim);
    vm.define_prim("read-line", read_line);
    vm.define_prim("eof-object", eof_object);
    vm.define_prim("eof-object?", is_eof_object);
    vm.define_prim("port?", is_port);
    vm.define_prim("input-port?", is_input_port);
    vm.define_prim("output-port?", is_output_port);
    vm.define_prim("textual-port?", is_textual_port);
    vm.define_prim("binary-port?", is_binary_port);
    vm.define_prim("input-port-open?", is_input_port_open);
    vm.define_prim("output-port-open?", is_output_port_open);
    vm.define_prim("close-port", close_port);
    vm.define_prim("close-input-port", close_port);
    vm.define_prim("close-output-port", close_port);
    vm.define_prim("char-ready?", is_char_ready);
    vm.define_prim("%stdout-port", stdout_port);
    vm.define_prim("%stderr-port", stderr_port);
    // vectors, including the growable addition
    vm.define_prim("make-vector", make_vector);
    vm.define_prim("vector-set!", vector_set);
    vm.define_prim("vector-push!", vector_push);
    vm.define_prim("vector-pop!", vector_pop);
    vm.define_prim("vector->list", vector_to_list);
    vm.define_prim("list->vector", list_to_vector);
    vm.define_prim("vector-fill!", vector_fill);
    vm.define_prim("string-fill!", string_fill);
    vm.define_prim("vector-copy", vector_copy);
    vm.define_prim("vector?", is_vector);
    // strings
    vm.define_prim("string-length", string_length);
    vm.define_prim("string-ref", string_ref);
    vm.define_prim("string-set!", string_set);
    vm.define_prim("make-string", make_string);
    vm.define_prim("string", string_from_chars);
    vm.define_prim("string-append", string_append);
    vm.define_prim("substring", substring);
    vm.define_prim("string->list", string_to_list);
    vm.define_prim("list->string", list_to_string);
    vm.define_prim("string=?", string_eq);
    vm.define_prim("string<?", string_lt);
    vm.define_prim("string>?", string_gt);
    vm.define_prim("string-ci=?", string_ci_eq);
    vm.define_prim("string-upcase", string_upcase);
    vm.define_prim("string-downcase", string_downcase);
    vm.define_prim("string-copy", string_copy);
    vm.define_prim("string?", is_string);
    // the name-ish conversions
    vm.define_prim("string->symbol", string_to_symbol);
    vm.define_prim("symbol->string", symbol_to_string);
    vm.define_prim("string->keyword", string_to_keyword);
    vm.define_prim("keyword->string", keyword_to_string);
    vm.define_prim("string->number", string_to_number);
    vm.define_prim("number->string", number_to_string);
    // characters
    vm.define_prim("char->integer", char_to_integer);
    vm.define_prim("integer->char", integer_to_char);
    vm.define_prim("char?", is_char);
    vm.define_prim("char=?", char_eq);
    vm.define_prim("char<?", char_lt);
    vm.define_prim("char>?", char_gt);
    vm.define_prim("char-upcase", char_upcase);
    vm.define_prim("char-downcase", char_downcase);
    vm.define_prim("char-alphabetic?", char_alpha);
    vm.define_prim("char-numeric?", char_numeric);
    vm.define_prim("char-whitespace?", char_white);
    // format, and errors
    vm.define_prim("format", format);
    vm.define_prim("error", error_prim);
    vm.define_prim("%raise", raise_prim);
    vm.define_prim("error-object?", is_error_object);
    vm.define_prim("error-object-message", error_object_message);
    vm.define_prim("error-object-irritants", error_object_irritants);
    vm.define_prim("read-error?", is_read_error);
    vm.define_prim("file-error?", is_file_error);
    // the remaining predicates
    vm.define_prim("boolean?", is_boolean);
    vm.define_prim("procedure?", is_procedure);
    vm.define_prim("exact-integer?", is_exact_integer);
    vm.define_prim("positive?", is_positive);
    vm.define_prim("negative?", is_negative);
    vm.define_prim("even?", is_even);
    vm.define_prim("odd?", is_odd);
    vm.define_prim("nan?", is_nan);
    // parameters
    vm.define_vm_prim("%make-parameter", make_parameter);
    vm.define_prim("%param-converter", param_converter);
    vm.define_prim("parameter?", is_parameter);
    // randomness, SRFI-27 flavoured
    vm.define_prim("make-random-source", make_random_source);
    vm.define_prim("random-source?", is_random_source);
    vm.define_vm_prim("random-real", random_real);
    vm.define_vm_prim("random-integer", random_integer);
    // quasiquote's runtime, under names nothing will shadow
    vm.define_prim("%qq-cons", qq_cons);
    vm.define_prim("%qq-append", qq_append);
    vm.define_prim("%qq-list->vector", qq_list_to_vector);
    // the cheap tail: numbers
    vm.define_prim("real?", p_is_real);
    vm.define_prim("rational?", p_is_rational);
    vm.define_prim("integer?", p_is_integer);
    vm.define_prim("inexact?", p_is_inexact);
    vm.define_prim("finite?", p_is_finite);
    vm.define_prim("infinite?", p_is_infinite);
    vm.define_prim("exact", p_exact);
    vm.define_prim("inexact->exact", p_exact);
    vm.define_prim("square", p_square);
    vm.define_prim("gcd", p_gcd);
    vm.define_prim("lcm", p_lcm);
    vm.define_prim("exact-integer-sqrt", p_exact_integer_sqrt);
    vm.define_prim("floor/", p_floor_div);
    vm.define_prim("truncate/", p_truncate_div);
    vm.define_prim("floor-quotient", p_floor_quotient);
    vm.define_prim("floor-remainder", p_floor_remainder);
    vm.define_prim("truncate-quotient", p_quotient);
    vm.define_prim("truncate-remainder", p_remainder);
    vm.define_prim("numerator", p_numerator);
    vm.define_prim("denominator", p_denominator);
    // characters
    vm.define_prim("char<=?", char_le);
    vm.define_prim("char>=?", char_ge);
    vm.define_prim("char-ci=?", char_ci_eq);
    vm.define_prim("char-ci<?", char_ci_lt);
    vm.define_prim("char-ci>?", char_ci_gt);
    vm.define_prim("char-ci<=?", char_ci_le);
    vm.define_prim("char-ci>=?", char_ci_ge);
    vm.define_prim("char-upper-case?", char_upper);
    vm.define_prim("char-lower-case?", char_lower);
    vm.define_prim("char-foldcase", char_foldcase);
    vm.define_prim("digit-value", digit_value);
    // strings
    vm.define_prim("string<=?", string_le);
    vm.define_prim("string>=?", string_ge);
    vm.define_prim("string-ci<?", string_ci_lt);
    vm.define_prim("string-ci>?", string_ci_gt);
    vm.define_prim("string-ci<=?", string_ci_le);
    vm.define_prim("string-ci>=?", string_ci_ge);
    vm.define_prim("string-foldcase", string_foldcase);
    vm.define_prim("string-copy!", string_copy_into);
    vm.define_prim("string->vector", string_to_vector);
    vm.define_prim("vector->string", vector_to_string);
    // vectors and lists
    vm.define_prim("vector-copy!", vector_copy_into);
    vm.define_prim("vector-append", vector_append);
    vm.define_prim("set-car!", set_car);
    vm.define_prim("set-cdr!", set_cdr);
    // equality
    vm.define_prim("boolean=?", boolean_eq);
    vm.define_prim("symbol=?", symbol_eq);
    // bytevectors
    vm.define_prim("bytevector", bv_new);
    vm.define_prim("make-bytevector", bv_make);
    vm.define_prim("bytevector?", bv_is);
    vm.define_prim("bytevector-length", bv_length);
    vm.define_prim("bytevector-u8-ref", bv_u8_ref);
    vm.define_prim("bytevector-u8-set!", bv_u8_set);
    vm.define_prim("bytevector-copy", bv_copy);
    vm.define_prim("bytevector-append", bv_append);
    vm.define_prim("utf8->string", utf8_to_string);
    vm.define_prim("string->utf8", string_to_utf8);
    // the system interface
    vm.define_prim("features", p_features);
    vm.define_prim("get-environment-variable", p_get_env);
    vm.define_prim("get-environment-variables", p_get_envs);
    vm.define_prim("command-line", p_command_line);
    vm.define_prim("current-second", p_current_second);
    vm.define_prim("current-jiffy", p_current_jiffy);
    vm.define_prim("jiffies-per-second", p_jiffies_per_second);
    vm.define_prim("exit", p_exit);
    vm.define_prim("emergency-exit", p_exit);
    // files
    vm.define_prim("file-exists?", p_file_exists);
    vm.define_prim("delete-file", p_delete_file);
    vm.define_prim("open-input-file", p_open_input_file);
}
