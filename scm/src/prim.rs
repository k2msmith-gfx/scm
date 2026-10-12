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
    vm.define_prim("equal?", equal);
    vm.define_prim("list", list);
    vm.define_prim("length", length);
    vm.define_prim("vector", vector);
    vm.define_prim("vector-ref", vector_ref);
    vm.define_prim("vector-length", vector_length);
    vm.define_prim("keyword?", is_keyword);
    vm.define_prim("symbol?", is_symbol);
    vm.define_prim("display", display);
    vm.define_prim("newline", newline);
}
