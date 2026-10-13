//! Enough primitives to exercise the machine. Not the standard library --
//! that is S5, behind `syntax-rules` and the library system.
//!
//! The arithmetic carries two settled decisions (plan §4.5), and both are
//! about refusing to lose information quietly:
//!
//!   * **Fixnum overflow is an error**, not a silent widening to a flonum.
//!   * **`(/ 8 3)` is inexact**, because `ratios` is not claimed -- which is
//!     the Steel trap avoided by construction rather than by discipline.

use crate::heap::{Engine, Obj};
use crate::value::Value;
use crate::vm::{Vm, VmError};
use crate::writer::{display_str, write_str};

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

fn num_eq(e: &mut Engine, a: &[Value]) -> PResult {
    compare("=", e, a, |x, y| x == y)
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

fn p_modulo(_e: &mut Engine, a: &[Value]) -> PResult {
    let (x, y) = int_pair("modulo", a)?;
    Ok(Value::Fixnum(x.rem_euclid(y.abs()) * if y < 0 { -1 } else { 1 }))
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

fn display(e: &mut Engine, a: &[Value]) -> PResult {
    for &v in a {
        print!("{}", display_str(e, v));
    }
    Ok(Value::Unspecified)
}

fn newline(_e: &mut Engine, _a: &[Value]) -> PResult {
    println!();
    Ok(Value::Unspecified)
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
    vm.define_prim("display", display);
    vm.define_prim("newline", newline);
}
