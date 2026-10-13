//! IR emission for the leaf subset.
//!
//! Every value is an `f64`. Booleans exist only in test position -- the
//! condition of an `if`, under `and`/`or`/`not` -- which is where the
//! corpus has them, and a boolean used as a value is a refusal rather than
//! a convention nobody wrote down. A named `let` is a loop with phi nodes;
//! an unnamed one binds SSA values; a captured table is a constant array;
//! a 3-float return is built per the target hook in `crate::vec3_abi`.
//!
//! Carried over from the S5b probe and extended: predicates, integer
//! operations on `f64`, captured tables with bounds checks, `begin`, `let`
//! in loop tails, and the `Vec3` ABI hook.

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::ContextRef;
use inkwell::module::{Linkage, Module};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, FloatValue, FunctionValue, IntValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use scm::heap::Obj;
use scm::{SymId, Value, Vm};

use crate::subset::{binding_items, expand, list, literal_sequence};
use crate::{Shape, Vec3Abi};

type R<T> = Result<T, String>;
type Env<'c> = HashMap<SymId, FloatValue<'c>>;

/// Primitives that are one libm call.
fn libm1(n: &str) -> Option<&'static str> {
    Some(match n {
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "asin" => "asin",
        "acos" => "acos",
        "sqrt" => "sqrt",
        "exp" => "exp",
        "log" => "log",
        "floor" => "floor",
        "ceiling" => "ceil",
        "truncate" => "trunc",
        // T0 rounds half to even, which is `rint` in the default mode.
        "round" => "rint",
        _ => return None,
    })
}

/// A compiled table: a private constant array of `f64`.
struct Table<'c> {
    global: inkwell::values::GlobalValue<'c>,
    len: u64,
}

/// Per-function state: the function, and the trap block an out-of-range
/// table read branches to. The trap returns NaN, because a leaf has no way
/// to raise and must never read outside its tables.
struct Fun<'c> {
    f: FunctionValue<'c>,
    shape: Shape,
    trap: Option<BasicBlock<'c>>,
    /// Loop names in scope: a call to one anywhere but tail position is a
    /// refusal, since a loop is a branch and not a procedure.
    loops: Vec<SymId>,
    /// Booleans bound by a `let` in test position -- which is how `or`
    /// expands, through a hygienic temporary -- visible only to tests.
    bools: HashMap<SymId, IntValue<'c>>,
}

pub struct Cg<'c> {
    ctx: ContextRef<'c>,
    pub module: Module<'c>,
    pub b: Builder<'c>,
    done: HashMap<String, FunctionValue<'c>>,
    tables: HashMap<String, Table<'c>>,
}

/// The symbol the entry point is emitted under, distinct from the Scheme
/// name so a name like `h-at` is a valid symbol and cannot collide with libm.
pub fn entry_symbol(name: &str) -> String {
    format!("scm_leaf_{}", name.replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
}

impl<'c> Cg<'c> {
    pub fn new(ctx: ContextRef<'c>) -> Self {
        let module = ctx.create_module("leaf");
        let b = ctx.create_builder();
        Self { ctx, module, b, done: HashMap::new(), tables: HashMap::new() }
    }

    /// The entry point: `name` with the requested shape, under its own symbol.
    pub fn entry(&mut self, vm: &mut Vm, name: &str, shape: Shape) -> R<()> {
        match shape {
            Shape::Scalar => {
                let f = self.function(vm, name)?;
                // An alias under the entry symbol: a scalar leaf IS its helper.
                let fty = f.get_type();
                let alias = self.module.add_function(&entry_symbol(name), fty, None);
                let bb = self.ctx.append_basic_block(alias, "entry");
                self.b.position_at_end(bb);
                let args: Vec<BasicMetadataValueEnum> =
                    alias.get_param_iter().map(|p| p.into()).collect();
                let cs = self.b.build_call(f, &args, "r").map_err(e)?;
                let v = cs.try_as_basic_value().basic().ok_or("a call with no value")?;
                self.b.build_return(Some(&v)).map_err(e)?;
                Ok(())
            }
            Shape::Vec3 => self.function3(vm, name),
        }
    }

    fn closure(&self, vm: &Vm, name: &str) -> R<(Vec<SymId>, Vec<Value>)> {
        let sym = vm.engine.interner.lookup(name).ok_or_else(|| format!("`{name}` is unbound"))?;
        let Some(Value::Obj(r)) = vm.global(sym) else {
            return Err(format!("`{name}` is not defined"));
        };
        let Obj::Closure { params, body, .. } = vm.engine.obj(r).clone() else {
            return Err(format!("`{name}` is not an authored procedure"));
        };
        Ok((params.to_vec(), body.to_vec()))
    }

    /// A scalar function: `f64`s in, `f64` out. Compiled once per module;
    /// self-recursion finds the entry inserted before the body is built.
    fn function(&mut self, vm: &mut Vm, name: &str) -> R<FunctionValue<'c>> {
        if let Some(f) = self.done.get(name) {
            return Ok(*f);
        }
        let (params, body) = self.closure(vm, name)?;
        let f64t = self.ctx.f64_type();
        let args: Vec<BasicMetadataTypeEnum> = vec![f64t.into(); params.len()];
        let f = self.module.add_function(&format!("leaf_{name}"), f64t.fn_type(&args, false), Some(Linkage::Private));
        self.done.insert(name.to_string(), f);
        let saved = self.b.get_insert_block();
        let entry = self.ctx.append_basic_block(f, "entry");
        self.b.position_at_end(entry);
        let mut env = Env::new();
        for (i, p) in params.iter().enumerate() {
            env.insert(*p, f.get_nth_param(i as u32).unwrap().into_float_value());
        }
        let mut fun = Fun { f, shape: Shape::Scalar, trap: None, loops: Vec::new(), bools: HashMap::new() };
        let out = self
            .body(vm, &body, &mut env, &mut fun)
            .and_then(|v| self.b.build_return(Some(&v)).map(|_| ()).map_err(e));
        if let Some(bb) = saved {
            self.b.position_at_end(bb);
        }
        match out {
            Ok(()) => Ok(f),
            Err(why) => {
                // A failed body must not leave a terminator-less function
                // behind, or one refusal stops the whole module verifying.
                self.done.remove(name);
                unsafe { f.delete() };
                // The entry's own name is the refusal's `via`; a helper's
                // is part of the message, as "reached through" is for the
                // analysis.
                if self.done.is_empty() { Err(why) } else { Err(format!("{why} (in `{name}`)")) }
            }
        }
    }

    /// A texture-shaped function: three `f64`s out, by the ABI hook.
    fn function3(&mut self, vm: &mut Vm, name: &str) -> R<()> {
        let (params, body) = self.closure(vm, name)?;
        let f64t = self.ctx.f64_type();
        let v3 = self.ctx.struct_type(&[f64t.into(), f64t.into(), f64t.into()], false);
        let abi = crate::vec3_abi();
        let mut args: Vec<BasicMetadataTypeEnum> = Vec::new();
        if abi == Vec3Abi::OutPointer {
            args.push(self.ctx.ptr_type(AddressSpace::default()).into());
        }
        args.extend(std::iter::repeat_n(BasicMetadataTypeEnum::from(f64t), params.len()));
        let fty = match abi {
            Vec3Abi::Registers => v3.fn_type(&args, false),
            Vec3Abi::OutPointer => self.ctx.void_type().fn_type(&args, false),
        };
        let f = self.module.add_function(&entry_symbol(name), fty, None);
        let entry = self.ctx.append_basic_block(f, "entry");
        self.b.position_at_end(entry);
        let skip = if abi == Vec3Abi::OutPointer { 1 } else { 0 };
        let mut env = Env::new();
        for (i, p) in params.iter().enumerate() {
            env.insert(*p, f.get_nth_param((i + skip) as u32).unwrap().into_float_value());
        }
        let mut fun = Fun { f, shape: Shape::Vec3, trap: None, loops: Vec::new(), bools: HashMap::new() };
        let three = (|| {
            if body.is_empty() {
                return Err("an empty body".into());
            }
            for form in &body[..body.len() - 1] {
                self.expr(vm, *form, &mut env, &mut fun)?;
            }
            self.three(vm, body[body.len() - 1], &mut env, &mut fun)
        })();
        let three = match three {
            Ok(t) => t,
            Err(why) => {
                unsafe { f.delete() };
                return Err(why);
            }
        };
        self.return3(three, &fun)
    }

    fn return3(&mut self, three: [FloatValue<'c>; 3], fun: &Fun<'c>) -> R<()> {
        match crate::vec3_abi() {
            Vec3Abi::Registers => {
                let f64t = self.ctx.f64_type();
                let v3 = self.ctx.struct_type(&[f64t.into(), f64t.into(), f64t.into()], false);
                let mut agg = v3.get_undef();
                for (i, v) in three.iter().enumerate() {
                    agg = self.b.build_insert_value(agg, *v, i as u32, "c").map_err(e)?.into_struct_value();
                }
                self.b.build_return(Some(&agg)).map_err(e)?;
            }
            Vec3Abi::OutPointer => {
                let out = fun.f.get_nth_param(0).unwrap().into_pointer_value();
                let f64t = self.ctx.f64_type();
                for (i, v) in three.iter().enumerate() {
                    let idx = self.ctx.i64_type().const_int(i as u64, false);
                    let p = unsafe { self.b.build_in_bounds_gep(f64t, out, &[idx], "o").map_err(e)? };
                    self.b.build_store(p, *v).map_err(e)?;
                }
                self.b.build_return(None).map_err(e)?;
            }
        }
        Ok(())
    }

    /// A body: every form but the last is compiled for its refusals and
    /// discarded, because a pure form's value is its only effect.
    fn body(&mut self, vm: &mut Vm, body: &[Value], env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let Some((last, rest)) = body.split_last() else {
            return Err("an empty body".into());
        };
        for form in rest {
            self.expr(vm, *form, env, fun)?;
        }
        self.expr(vm, *last, env, fun)
    }

    /// The three components of a colour: through `let`s, `begin`s and `if`s
    /// to a literal `(list a b c)`, `(vector a b c)`, `[a b c]`, or the
    /// corpus's `(map (lambda (c) …) (list a b c))`.
    fn three(&mut self, vm: &mut Vm, expr: Value, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<[FloatValue<'c>; 3]> {
        if let Value::Obj(r) = expr
            && let Obj::Vector { items, bracket: true } = vm.engine.obj(r).clone()
        {
            return self.three_of(vm, &items, env, fun);
        }
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr)?;
        if let Some(l) = as_let(vm, &parts)? {
            if l.name.is_some() {
                return Err("a loop cannot produce a colour directly; bind its result with `let`".into());
            }
            let saved = self.bind_let(vm, &l, env, fun)?;
            let out = self.three(vm, l.body, env, fun);
            restore(env, saved);
            return out;
        }
        let Some(&Value::Symbol(h)) = parts.first() else {
            return Err("not a 3-element colour".into());
        };
        let hn = name_of(vm, vm.engine.resolve(h));
        match hn.as_str() {
            "begin" if parts.len() >= 2 => {
                for p in &parts[1..parts.len() - 1] {
                    self.expr(vm, *p, env, fun)?;
                }
                self.three(vm, parts[parts.len() - 1], env, fun)
            }
            "if" if parts.len() == 4 => {
                let cond = self.test(vm, parts[1], env, fun)?;
                let tb = self.ctx.append_basic_block(fun.f, "t3");
                let eb = self.ctx.append_basic_block(fun.f, "e3");
                let jb = self.ctx.append_basic_block(fun.f, "j3");
                self.b.build_conditional_branch(cond, tb, eb).map_err(e)?;
                self.b.position_at_end(tb);
                let a = self.three(vm, parts[2], env, fun)?;
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let t_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(eb);
                let b = self.three(vm, parts[3], env, fun)?;
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let e_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(jb);
                let mut out = [a[0]; 3];
                for i in 0..3 {
                    let phi = self.b.build_phi(self.ctx.f64_type(), "c").map_err(e)?;
                    phi.add_incoming(&[(&a[i], t_end), (&b[i], e_end)]);
                    out[i] = phi.as_basic_value().into_float_value();
                }
                Ok(out)
            }
            "list" | "vector" if parts.len() == 4 => self.three_of(vm, &parts[1..], env, fun),
            "map" if parts.len() == 3 => {
                let Some(items) = literal_sequence(vm, parts[2])? else {
                    return Err("`map` over a sequence of unknown length cannot be specialised".into());
                };
                if items.len() != 3 {
                    return Err(format!("a colour has 3 components, this `map` produces {}", items.len()));
                }
                let mut out = Vec::new();
                for item in &items {
                    out.push(self.apply_inline(vm, parts[1], &[*item], env, fun)?);
                }
                Ok([out[0], out[1], out[2]])
            }
            _ => Err(format!("`{hn}` does not produce a 3-element colour")),
        }
    }

    fn three_of(&mut self, vm: &mut Vm, items: &[Value], env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<[FloatValue<'c>; 3]> {
        if items.len() != 3 {
            return Err(format!("a colour has 3 components, not {}", items.len()));
        }
        let a = self.expr(vm, items[0], env, fun)?;
        let b = self.expr(vm, items[1], env, fun)?;
        let c = self.expr(vm, items[2], env, fun)?;
        Ok([a, b, c])
    }

    /// Apply `f` -- an inline `lambda` or the name of a compiled helper --
    /// to already-known argument *forms*, inlining a lambda's body.
    fn apply_inline(&mut self, vm: &mut Vm, f: Value, args: &[Value], env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let f = expand(vm, f)?;
        if let Value::Symbol(s) = f {
            let mut call = vec![f];
            call.extend_from_slice(args);
            let form = vm.engine.list(&call);
            let _ = s;
            return self.expr(vm, form, env, fun);
        }
        let parts = list(vm, f)?;
        let (formals, body) = as_lambda(vm, &parts)?;
        if formals.len() != args.len() {
            return Err(format!("a lambda of {} parameters applied to {} arguments", formals.len(), args.len()));
        }
        let vals: Vec<_> = args.iter().map(|a| self.expr(vm, *a, env, fun)).collect::<R<_>>()?;
        let saved: Vec<_> = formals.iter().map(|p| (*p, env.get(p).copied())).collect();
        for (p, v) in formals.iter().zip(vals) {
            env.insert(*p, v);
        }
        let out = self.body(vm, &body, env, fun);
        restore(env, saved);
        out
    }

    fn expr(&mut self, vm: &mut Vm, expr: Value, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let f64t = self.ctx.f64_type();
        match expr {
            Value::Flonum(x) => Ok(f64t.const_float(x)),
            Value::Fixnum(n) => Ok(f64t.const_float(n as f64)),
            Value::Bool(_) => Err("a boolean is not a value in the leaf subset; it may only be tested".into()),
            Value::Symbol(s) => {
                if let Some(v) = env.get(&s) {
                    return Ok(*v);
                }
                let r = vm.engine.resolve(s);
                if let Some(v) = env.get(&r) {
                    return Ok(*v);
                }
                match vm.global(r) {
                    Some(Value::Flonum(x)) => Ok(f64t.const_float(x)),
                    Some(Value::Fixnum(n)) => Ok(f64t.const_float(n as f64)),
                    Some(Value::Obj(o)) if matches!(vm.engine.obj(o), Obj::Vector { .. }) => {
                        Err(format!("`{}` is a table; read it with `vector-ref`", name_of(vm, r)))
                    }
                    _ => Err(format!("`{}` is not a local or a numeric constant", name_of(vm, r))),
                }
            }
            Value::Obj(r) => match vm.engine.obj(r) {
                Obj::Pair(..) => self.combination(vm, expr, env, fun),
                Obj::Vector { .. } => Err("a vector is not a scalar value".into()),
                _ => Err(format!("`{}` has no numeric form", scm::write_str(&vm.engine, expr))),
            },
            other => Err(format!("`{}` has no numeric form", scm::write_str(&vm.engine, other))),
        }
    }

    fn combination(&mut self, vm: &mut Vm, expr: Value, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr)?;

        if let Some(l) = as_let(vm, &parts)? {
            return match l.name {
                None => {
                    let saved = self.bind_let(vm, &l, env, fun)?;
                    let out = self.expr(vm, l.body, env, fun);
                    restore(env, saved);
                    out
                }
                Some(_) => self.loop_(vm, &l, env, fun),
            };
        }

        let Some(&Value::Symbol(h)) = parts.first() else {
            return Err("a computed operator cannot be compiled".into());
        };
        if fun.loops.contains(&h) || fun.loops.contains(&vm.engine.resolve(h)) {
            return Err(format!("`{}` is a loop, and a loop may only be called in tail position", name_of(vm, h)));
        }
        if env.contains_key(&h) {
            return Err(format!("`{}` is a local, and a leaf cannot call a local", name_of(vm, h)));
        }
        let hn = name_of(vm, vm.engine.resolve(h));
        let args = &parts[1..];
        let f64t = self.ctx.f64_type();

        match hn.as_str() {
            "+" | "-" | "*" | "/" => {
                let vals = self.all(vm, args, env, fun)?;
                if vals.is_empty() {
                    return match hn.as_str() {
                        "+" => Ok(f64t.const_float(0.0)),
                        "*" => Ok(f64t.const_float(1.0)),
                        _ => Err(format!("`{hn}` with no arguments")),
                    };
                }
                if vals.len() == 1 {
                    return match hn.as_str() {
                        "-" => self.b.build_float_neg(vals[0], "neg").map_err(e),
                        "/" => self.b.build_float_div(f64t.const_float(1.0), vals[0], "rec").map_err(e),
                        _ => Ok(vals[0]),
                    };
                }
                let mut acc = vals[0];
                for v in &vals[1..] {
                    acc = match hn.as_str() {
                        "+" => self.b.build_float_add(acc, *v, "add"),
                        "-" => self.b.build_float_sub(acc, *v, "sub"),
                        "*" => self.b.build_float_mul(acc, *v, "mul"),
                        _ => self.b.build_float_div(acc, *v, "div"),
                    }
                    .map_err(e)?;
                }
                Ok(acc)
            }
            "min" | "max" => {
                let vals = self.all(vm, args, env, fun)?;
                let Some((first, rest)) = vals.split_first() else {
                    return Err(format!("`{hn}` with no arguments"));
                };
                let intrinsic = if hn == "min" { "llvm.minnum.f64" } else { "llvm.maxnum.f64" };
                let mut acc = *first;
                for v in rest {
                    acc = self.call_extern(intrinsic, &[acc, *v])?;
                }
                Ok(acc)
            }
            "abs" => {
                let [v] = self.all(vm, args, env, fun)?[..] else {
                    return Err("`abs` wants one argument".into());
                };
                self.call_extern("llvm.fabs.f64", &[v])
            }
            "square" => {
                let [v] = self.all(vm, args, env, fun)?[..] else {
                    return Err("`square` wants one argument".into());
                };
                self.b.build_float_mul(v, v, "sq").map_err(e)
            }
            "exact" | "inexact" | "exact->inexact" | "inexact->exact" => {
                let [v] = self.all(vm, args, env, fun)?[..] else {
                    return Err(format!("`{hn}` wants one argument"));
                };
                Ok(v)
            }
            "expt" => {
                let [a, b] = self.all(vm, args, env, fun)?[..] else {
                    return Err("`expt` wants two arguments".into());
                };
                self.call_extern("pow", &[a, b])
            }
            "atan" => match self.all(vm, args, env, fun)?[..] {
                [y] => self.call_extern("atan", &[y]),
                [y, x] => self.call_extern("atan2", &[y, x]),
                _ => Err("`atan` wants one or two arguments".into()),
            },
            "quotient" | "truncate-quotient" => {
                let [a, b] = self.all(vm, args, env, fun)?[..] else {
                    return Err(format!("`{hn}` wants two arguments"));
                };
                let q = self.b.build_float_div(a, b, "q").map_err(e)?;
                self.call_extern("trunc", &[q])
            }
            "remainder" | "truncate-remainder" => {
                let [a, b] = self.all(vm, args, env, fun)?[..] else {
                    return Err(format!("`{hn}` wants two arguments"));
                };
                self.b.build_float_rem(a, b, "rem").map_err(e)
            }
            "modulo" | "floor-remainder" => {
                let [a, b] = self.all(vm, args, env, fun)?[..] else {
                    return Err(format!("`{hn}` wants two arguments"));
                };
                self.floor_mod(a, b)
            }
            "floor-quotient" => {
                let [a, b] = self.all(vm, args, env, fun)?[..] else {
                    return Err("`floor-quotient` wants two arguments".into());
                };
                let q = self.b.build_float_div(a, b, "q").map_err(e)?;
                self.call_extern("floor", &[q])
            }
            "vector-ref" => {
                let [table, index] = args else {
                    return Err("`vector-ref` wants a table and an index".into());
                };
                let t = self.table(vm, *table)?;
                let i = self.expr(vm, *index, env, fun)?;
                self.load(t, i, fun)
            }
            "vector-length" => {
                let [table] = args else {
                    return Err("`vector-length` wants a table".into());
                };
                let t = self.table(vm, *table)?;
                Ok(f64t.const_float(t as f64))
            }
            "if" => {
                if args.len() != 2 && args.len() != 3 {
                    return Err("`if` wants a test and one or two branches".into());
                }
                // `(if #f #f)` is the idiom for the unspecified value, and
                // `when` expands to it: here it is 0.0, like a missing arm.
                if let [Value::Bool(false), Value::Bool(false)] = args {
                    return Ok(f64t.const_float(0.0));
                }
                let cond = self.test(vm, args[0], env, fun)?;
                let tb = self.ctx.append_basic_block(fun.f, "then");
                let eb = self.ctx.append_basic_block(fun.f, "else");
                let jb = self.ctx.append_basic_block(fun.f, "join");
                self.b.build_conditional_branch(cond, tb, eb).map_err(e)?;
                self.b.position_at_end(tb);
                let a = self.expr(vm, args[1], env, fun)?;
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let t_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(eb);
                // A one-armed `if` -- `when` -- in value position yields an
                // unspecified value in T0; here it is 0.0, and a leaf whose
                // result depends on it was already wrong.
                let b = match args.get(2) {
                    Some(x) => self.expr(vm, *x, env, fun)?,
                    None => f64t.const_float(0.0),
                };
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let e_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(jb);
                let phi = self.b.build_phi(f64t, "v").map_err(e)?;
                phi.add_incoming(&[(&a, t_end), (&b, e_end)]);
                Ok(phi.as_basic_value().into_float_value())
            }
            "begin" => self.body(vm, args, env, fun),
            "map" => Err("`map` produces a sequence, not a number".into()),
            "list" | "vector" => Err(format!("`{hn}` allocates a sequence, which is not a value in the leaf subset; a table must be a global")),
            "and" | "or" | "not" | "=" | "<" | "<=" | ">" | ">=" | "zero?" | "positive?" | "negative?"
            | "even?" | "odd?" | "nan?" => {
                Err(format!("`{hn}` produces a boolean, which is not a value in the leaf subset; it may only be tested"))
            }
            "quote" => match args {
                [Value::Fixnum(n)] => Ok(f64t.const_float(*n as f64)),
                [Value::Flonum(x)] => Ok(f64t.const_float(*x)),
                _ => Err("quoted structure has no numeric form".into()),
            },
            "lambda" => Err("a lambda is only compiled as the procedure of a `map` over a literal list".into()),
            "set!" => Err("`set!` on a local is not in the leaf subset; rebind with a named `let`".into()),
            _ => {
                if let Some(m) = libm1(&hn) {
                    let [v] = self.all(vm, args, env, fun)?[..] else {
                        return Err(format!("`{hn}` wants one argument"));
                    };
                    return self.call_extern(m, &[v]);
                }
                // An authored procedure in the subset.
                let resolved = vm.engine.resolve(h);
                if let Some(Value::Obj(o)) = vm.global(resolved)
                    && matches!(vm.engine.obj(o), Obj::Closure { .. })
                {
                    let callee = self.function(vm, &hn)?;
                    let n = callee.count_params() as usize;
                    if n != args.len() {
                        return Err(format!("`{hn}` takes {n} argument{}, called with {}", if n == 1 { "" } else { "s" }, args.len()));
                    }
                    let vals: Vec<BasicMetadataValueEnum> =
                        self.all(vm, args, env, fun)?.into_iter().map(Into::into).collect();
                    let cs = self.b.build_call(callee, &vals, "c").map_err(e)?;
                    return Ok(cs.try_as_basic_value().basic().ok_or("a call with no value")?.into_float_value());
                }
                Err(format!("`{hn}` has no compiled form"))
            }
        }
    }

    fn all(&mut self, vm: &mut Vm, args: &[Value], env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<Vec<FloatValue<'c>>> {
        args.iter().map(|a| self.expr(vm, *a, env, fun)).collect()
    }

    /// R7RS `modulo`: the sign of the divisor. `frem` has the dividend's.
    fn floor_mod(&mut self, a: FloatValue<'c>, b: FloatValue<'c>) -> R<FloatValue<'c>> {
        let r = self.b.build_float_rem(a, b, "rem").map_err(e)?;
        let zero = self.ctx.f64_type().const_float(0.0);
        let r_neg = self.b.build_float_compare(FloatPredicate::OLT, r, zero, "rn").map_err(e)?;
        let b_neg = self.b.build_float_compare(FloatPredicate::OLT, b, zero, "bn").map_err(e)?;
        let r_nz = self.b.build_float_compare(FloatPredicate::ONE, r, zero, "rnz").map_err(e)?;
        let differ = self.b.build_xor(r_neg, b_neg, "d").map_err(e)?;
        let fix = self.b.build_and(differ, r_nz, "fix").map_err(e)?;
        let adjusted = self.b.build_float_add(r, b, "adj").map_err(e)?;
        Ok(self.b.build_select(fix, adjusted, r, "mod").map_err(e)?.into_float_value())
    }

    /// Item (2): a global vector of numbers becomes a private constant array,
    /// snapshotted now. Returns its length.
    fn table(&mut self, vm: &mut Vm, form: Value) -> R<u64> {
        let Value::Symbol(s) = form else {
            return Err("a table must be named by a global variable".into());
        };
        let r = vm.engine.resolve(s);
        let name = name_of(vm, r);
        if let Some(t) = self.tables.get(&name) {
            return Ok(t.len);
        }
        let Some(Value::Obj(o)) = vm.global(r) else {
            return Err(format!("`{name}` is not a global table"));
        };
        let Obj::Vector { items, .. } = vm.engine.obj(o).clone() else {
            return Err(format!("`{name}` is not a vector"));
        };
        let f64t = self.ctx.f64_type();
        let mut consts = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            consts.push(match it {
                Value::Flonum(x) => f64t.const_float(*x),
                Value::Fixnum(n) => f64t.const_float(*n as f64),
                other => {
                    return Err(format!(
                        "`{name}` holds `{}` at index {i}, and a table can only hold numbers",
                        scm::write_str(&vm.engine, *other)
                    ));
                }
            });
        }
        let arr = f64t.const_array(&consts);
        let global = self.module.add_global(arr.get_type(), Some(AddressSpace::default()), &format!("table_{}", entry_symbol(&name)));
        global.set_initializer(&arr);
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        let len = items.len() as u64;
        self.tables.insert(name, Table { global, len });
        Ok(len)
    }

    /// A bounds-checked load: an index outside the table branches to the
    /// function's trap block, which returns NaN.
    fn load(&mut self, len: u64, index: FloatValue<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let f64t = self.ctx.f64_type();
        let i64t = self.ctx.i64_type();
        let table = self.tables.values().find(|t| t.len == len).map(|t| t.global);
        // `table` found the right global only if lengths are unique; look it
        // up properly by the caller instead. (Kept simple: the caller just
        // resolved it, so take the most recent insertion with this length.)
        let global = table.ok_or("internal: table not found")?;
        let i = self.b.build_float_to_signed_int(index, i64t, "i").map_err(e)?;
        let in_range = self.b.build_int_compare(IntPredicate::ULT, i, i64t.const_int(len, false), "inb").map_err(e)?;
        let ok = self.ctx.append_basic_block(fun.f, "ld");
        let trap = self.trap(fun)?;
        self.b.build_conditional_branch(in_range, ok, trap).map_err(e)?;
        self.b.position_at_end(ok);
        let zero = i64t.const_int(0, false);
        let p = unsafe {
            self.b.build_in_bounds_gep(f64t.array_type(len as u32), global.as_pointer_value(), &[zero, i], "e").map_err(e)?
        };
        Ok(self.b.build_load(f64t, p, "v").map_err(e)?.into_float_value())
    }

    fn trap(&mut self, fun: &mut Fun<'c>) -> R<BasicBlock<'c>> {
        if let Some(t) = fun.trap {
            return Ok(t);
        }
        let saved = self.b.get_insert_block().unwrap();
        let t = self.ctx.append_basic_block(fun.f, "trap");
        self.b.position_at_end(t);
        let nan = self.ctx.f64_type().const_float(f64::NAN);
        match fun.shape {
            Shape::Scalar => {
                self.b.build_return(Some(&nan)).map_err(e)?;
            }
            Shape::Vec3 => self.return3([nan, nan, nan], fun)?,
        }
        self.b.position_at_end(saved);
        fun.trap = Some(t);
        Ok(t)
    }

    /// A test: comparisons, `and`/`or`/`not`, the numeric predicates, and
    /// the literals. This is the only place a boolean exists.
    fn test(&mut self, vm: &mut Vm, expr: Value, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<IntValue<'c>> {
        let bool_t = self.ctx.bool_type();
        match expr {
            Value::Bool(b) => return Ok(bool_t.const_int(b as u64, false)),
            Value::Symbol(s) => {
                if let Some(v) = fun.bools.get(&s).or_else(|| fun.bools.get(&vm.engine.resolve(s))) {
                    return Ok(*v);
                }
                return Err(format!(
                    "`{}` is tested as a boolean, and booleans are not values in the leaf subset",
                    scm::write_str(&vm.engine, expr)
                ));
            }
            _ => {}
        }
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr).map_err(|_| "a test that is not a comparison".to_string())?;
        let Some(&Value::Symbol(h)) = parts.first() else {
            return Err("a test that is not a comparison".into());
        };
        let hn = name_of(vm, vm.engine.resolve(h));
        let args = &parts[1..];
        let zero = self.ctx.f64_type().const_float(0.0);
        match hn.as_str() {
            "not" => {
                let [a] = args else { return Err("`not` wants one argument".into()) };
                let v = self.test(vm, *a, env, fun)?;
                self.b.build_not(v, "not").map_err(e)
            }
            "and" | "or" => {
                // Short-circuit, so the right operand's table reads are not
                // evaluated when the left already decided.
                let is_and = hn == "and";
                let Some((first, rest)) = args.split_first() else {
                    return Ok(bool_t.const_int(is_and as u64, false));
                };
                let mut acc = self.test(vm, *first, env, fun)?;
                for a in rest {
                    let cur = self.b.get_insert_block().unwrap();
                    let more = self.ctx.append_basic_block(fun.f, "more");
                    let join = self.ctx.append_basic_block(fun.f, "jb");
                    if is_and {
                        self.b.build_conditional_branch(acc, more, join).map_err(e)?;
                    } else {
                        self.b.build_conditional_branch(acc, join, more).map_err(e)?;
                    }
                    self.b.position_at_end(more);
                    let v = self.test(vm, *a, env, fun)?;
                    self.b.build_unconditional_branch(join).map_err(e)?;
                    let more_end = self.b.get_insert_block().unwrap();
                    self.b.position_at_end(join);
                    let phi = self.b.build_phi(bool_t, "sc").map_err(e)?;
                    phi.add_incoming(&[(&acc, cur), (&v, more_end)]);
                    acc = phi.as_basic_value().into_int_value();
                }
                Ok(acc)
            }
            "=" | "<" | "<=" | ">" | ">=" => {
                let p = match hn.as_str() {
                    "<" => FloatPredicate::OLT,
                    "<=" => FloatPredicate::OLE,
                    ">" => FloatPredicate::OGT,
                    ">=" => FloatPredicate::OGE,
                    _ => FloatPredicate::OEQ,
                };
                let vals = self.all(vm, args, env, fun)?;
                if vals.len() < 2 {
                    return Err(format!("`{hn}` wants at least two arguments"));
                }
                let mut acc: Option<IntValue<'c>> = None;
                for w in vals.windows(2) {
                    let c = self.b.build_float_compare(p, w[0], w[1], "cmp").map_err(e)?;
                    acc = Some(match acc {
                        None => c,
                        Some(prev) => self.b.build_and(prev, c, "chain").map_err(e)?,
                    });
                }
                Ok(acc.unwrap())
            }
            "zero?" | "positive?" | "negative?" | "nan?" => {
                let [a] = args else { return Err(format!("`{hn}` wants one argument")) };
                let v = self.expr(vm, *a, env, fun)?;
                let p = match hn.as_str() {
                    "zero?" => FloatPredicate::OEQ,
                    "positive?" => FloatPredicate::OGT,
                    "negative?" => FloatPredicate::OLT,
                    _ => FloatPredicate::UNO,
                };
                let rhs = if hn == "nan?" { v } else { zero };
                self.b.build_float_compare(p, v, rhs, "p").map_err(e)
            }
            "even?" | "odd?" => {
                let [a] = args else { return Err(format!("`{hn}` wants one argument")) };
                let v = self.expr(vm, *a, env, fun)?;
                let two = self.ctx.f64_type().const_float(2.0);
                let r = self.b.build_float_rem(v, two, "r2").map_err(e)?;
                let p = if hn == "even?" { FloatPredicate::OEQ } else { FloatPredicate::ONE };
                self.b.build_float_compare(p, r, zero, "par").map_err(e)
            }
            // `or` and `cond` expand into these, so test position follows
            // them: an `if` of tests is a phi of booleans, a `let` binds
            // and tests its body.
            "if" if args.len() == 3 => {
                let cond = self.test(vm, args[0], env, fun)?;
                let tb = self.ctx.append_basic_block(fun.f, "tt");
                let eb = self.ctx.append_basic_block(fun.f, "te");
                let jb = self.ctx.append_basic_block(fun.f, "tj");
                self.b.build_conditional_branch(cond, tb, eb).map_err(e)?;
                self.b.position_at_end(tb);
                let a = self.test(vm, args[1], env, fun)?;
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let t_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(eb);
                let b = self.test(vm, args[2], env, fun)?;
                self.b.build_unconditional_branch(jb).map_err(e)?;
                let e_end = self.b.get_insert_block().unwrap();
                self.b.position_at_end(jb);
                let phi = self.b.build_phi(bool_t, "tb").map_err(e)?;
                phi.add_incoming(&[(&a, t_end), (&b, e_end)]);
                Ok(phi.as_basic_value().into_int_value())
            }
            "let" => {
                let Some(l) = as_let(vm, &parts)? else { unreachable!("head is let") };
                if l.name.is_some() {
                    return Err("a loop in test position is not in the leaf subset".into());
                }
                // Each init is a boolean if it looks like one, else a number.
                let mut saved_f = Vec::new();
                let mut saved_b = Vec::new();
                let mut bound_f = Vec::new();
                let mut bound_b = Vec::new();
                for (p, init) in l.formals.iter().zip(&l.inits) {
                    if is_boolean_form(vm, *init) {
                        bound_b.push((*p, self.test(vm, *init, env, fun)?));
                    } else {
                        bound_f.push((*p, self.expr(vm, *init, env, fun)?));
                    }
                }
                for (p, v) in bound_f {
                    saved_f.push((p, env.insert(p, v)));
                }
                for (p, v) in bound_b {
                    saved_b.push((p, fun.bools.insert(p, v)));
                }
                let out = self.test(vm, l.body, env, fun);
                restore(env, saved_f);
                for (p, old) in saved_b {
                    match old {
                        Some(v) => fun.bools.insert(p, v),
                        None => fun.bools.remove(&p),
                    };
                }
                out
            }
            "begin" if !args.is_empty() => {
                for a in &args[..args.len() - 1] {
                    self.expr(vm, *a, env, fun)?;
                }
                self.test(vm, args[args.len() - 1], env, fun)
            }
            _ => Err(format!("`{hn}` is not a compiled comparison or predicate")),
        }
    }

    /// Bind a `let`'s variables, returning what to restore afterwards.
    fn bind_let(&mut self, vm: &mut Vm, l: &Let, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<Vec<(SymId, Option<FloatValue<'c>>)>> {
        let vals: Vec<_> = l.inits.iter().map(|i| self.expr(vm, *i, env, fun)).collect::<R<_>>()?;
        let saved: Vec<_> = l.formals.iter().map(|p| (*p, env.get(p).copied())).collect();
        for (p, v) in l.formals.iter().zip(vals) {
            env.insert(*p, v);
        }
        Ok(saved)
    }

    /// A named `let`: phi nodes in a header block, a branch back for each
    /// tail self-call, and a result phi at the exit (item 3).
    fn loop_(&mut self, vm: &mut Vm, l: &Let, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<FloatValue<'c>> {
        let f64t = self.ctx.f64_type();
        let entry_vals = self.all(vm, &l.inits, env, fun)?;
        let pre = self.b.get_insert_block().unwrap();
        let header = self.ctx.append_basic_block(fun.f, "loop");
        let exit = self.ctx.append_basic_block(fun.f, "done");
        self.b.build_unconditional_branch(header).map_err(e)?;
        self.b.position_at_end(header);
        let saved: Vec<_> = l.formals.iter().map(|p| (*p, env.get(p).copied())).collect();
        let mut phis = Vec::new();
        for (i, v) in l.formals.iter().enumerate() {
            let phi = self.b.build_phi(f64t, "i").map_err(e)?;
            phi.add_incoming(&[(&entry_vals[i], pre)]);
            env.insert(*v, phi.as_basic_value().into_float_value());
            phis.push(phi);
        }
        let mut lp = Loop { name: l.name.unwrap(), arity: l.formals.len(), header, exit, backedges: Vec::new(), results: Vec::new() };
        fun.loops.push(lp.name);
        let out = self.tail(vm, l.body, &mut lp, env, fun);
        fun.loops.pop();
        restore(env, saved);
        out?;
        for (args, blk) in &lp.backedges {
            for (i, phi) in phis.iter().enumerate() {
                phi.add_incoming(&[(&args[i], *blk)]);
            }
        }
        self.b.position_at_end(exit);
        if lp.results.is_empty() {
            return Err("a loop with no exit".into());
        }
        let out = self.b.build_phi(f64t, "r").map_err(e)?;
        for (v, blk) in &lp.results {
            out.add_incoming(&[(v, *blk)]);
        }
        Ok(out.as_basic_value().into_float_value())
    }

    /// `expr` in tail position of a loop: a self-call branches back, a value
    /// branches to the exit, and `if`/`let`/`begin` are followed through.
    fn tail(&mut self, vm: &mut Vm, expr: Value, lp: &mut Loop<'c>, env: &mut Env<'c>, fun: &mut Fun<'c>) -> R<()> {
        let expr = expand(vm, expr)?;
        if let Ok(parts) = list(vm, expr)
            && let Some(&Value::Symbol(h)) = parts.first()
        {
            let resolved = vm.engine.resolve(h);
            if resolved == lp.name || h == lp.name {
                let args = self.all(vm, &parts[1..], env, fun)?;
                if args.len() != lp.arity {
                    return Err(format!("a loop self-call with {} arguments, the loop has {}", args.len(), lp.arity));
                }
                let blk = self.b.get_insert_block().unwrap();
                lp.backedges.push((args, blk));
                self.b.build_unconditional_branch(lp.header).map_err(e)?;
                return Ok(());
            }
            let hn = name_of(vm, resolved);
            if hn == "if" && (parts.len() == 3 || parts.len() == 4) {
                let cond = self.test(vm, parts[1], env, fun)?;
                let tb = self.ctx.append_basic_block(fun.f, "t");
                let eb = self.ctx.append_basic_block(fun.f, "e");
                self.b.build_conditional_branch(cond, tb, eb).map_err(e)?;
                self.b.position_at_end(tb);
                self.tail(vm, parts[2], lp, env, fun)?;
                self.b.position_at_end(eb);
                match parts.get(3) {
                    Some(&x) => self.tail(vm, x, lp, env, fun)?,
                    None => {
                        let z = self.ctx.f64_type().const_float(0.0);
                        let blk = self.b.get_insert_block().unwrap();
                        lp.results.push((z, blk));
                        self.b.build_unconditional_branch(lp.exit).map_err(e)?;
                    }
                }
                return Ok(());
            }
            if hn == "begin" && parts.len() >= 2 {
                for p in &parts[1..parts.len() - 1] {
                    self.expr(vm, *p, env, fun)?;
                }
                return self.tail(vm, parts[parts.len() - 1], lp, env, fun);
            }
            if let Some(l) = as_let(vm, &parts)?
                && l.name.is_none()
            {
                let saved = self.bind_let(vm, &l, env, fun)?;
                let out = self.tail(vm, l.body, lp, env, fun);
                restore(env, saved);
                return out;
            }
        }
        let v = self.expr(vm, expr, env, fun)?;
        let blk = self.b.get_insert_block().unwrap();
        lp.results.push((v, blk));
        self.b.build_unconditional_branch(lp.exit).map_err(e)?;
        Ok(())
    }

    fn call_extern(&mut self, n: &str, args: &[FloatValue<'c>]) -> R<FloatValue<'c>> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(n).unwrap_or_else(|| {
            let tys: Vec<BasicMetadataTypeEnum> = vec![f64t.into(); args.len()];
            self.module.add_function(n, f64t.fn_type(&tys, false), None)
        });
        let vals: Vec<BasicMetadataValueEnum> = args.iter().map(|v| (*v).into()).collect();
        let cs = self.b.build_call(f, &vals, "x").map_err(e)?;
        Ok(cs.try_as_basic_value().basic().ok_or("an extern call with no value")?.into_float_value())
    }
}

struct Loop<'c> {
    name: SymId,
    arity: usize,
    header: BasicBlock<'c>,
    exit: BasicBlock<'c>,
    backedges: Vec<(Vec<FloatValue<'c>>, BasicBlock<'c>)>,
    results: Vec<(FloatValue<'c>, BasicBlock<'c>)>,
}

/// `(let [name] ((v init) …) body …)`, a core form.
struct Let {
    name: Option<SymId>,
    formals: Vec<SymId>,
    inits: Vec<Value>,
    /// One body form; a multi-form body is wrapped in `begin`.
    body: Value,
}

fn as_let(vm: &mut Vm, parts: &[Value]) -> R<Option<Let>> {
    let Some(&Value::Symbol(h)) = parts.first() else {
        return Ok(None);
    };
    if name_of(vm, vm.engine.resolve(h)) != "let" {
        return Ok(None);
    }
    let args = &parts[1..];
    let (name, rest) = match args.first() {
        Some(&Value::Symbol(n)) => (Some(n), &args[1..]),
        _ => (None, args),
    };
    let Some((spec, body)) = rest.split_first() else {
        return Err("`let` with no binding list".into());
    };
    let mut formals = Vec::new();
    let mut inits = Vec::new();
    for pair in binding_items(vm, *spec)? {
        let parts = binding_items(vm, pair)?;
        match parts.as_slice() {
            [Value::Symbol(v), init] => {
                formals.push(*v);
                inits.push(*init);
            }
            _ => return Err("a `let` binding that is not (variable init)".into()),
        }
    }
    let body = match body {
        [one] => *one,
        [] => return Err("a `let` with no body".into()),
        many => {
            let mut forms = vec![vm.engine.sym("begin")];
            forms.extend_from_slice(many);
            vm.engine.list(&forms)
        }
    };
    Ok(Some(Let { name, formals, inits, body }))
}

fn as_lambda(vm: &mut Vm, parts: &[Value]) -> R<(Vec<SymId>, Vec<Value>)> {
    let Some(&Value::Symbol(h)) = parts.first() else {
        return Err("`map` wants a lambda or the name of a procedure".into());
    };
    if name_of(vm, vm.engine.resolve(h)) != "lambda" || parts.len() < 3 {
        return Err("`map` wants a lambda or the name of a procedure".into());
    }
    let formals = list(vm, parts[1])?
        .into_iter()
        .map(|x| match x {
            Value::Symbol(s) => Ok(s),
            _ => Err("a lambda formal that is not a symbol".to_string()),
        })
        .collect::<R<_>>()?;
    Ok((formals, parts[2..].to_vec()))
}

/// Does this form produce a boolean? Decided by its head, before expansion,
/// so `and`/`or`/`not` and the comparisons count and a number does not.
fn is_boolean_form(vm: &Vm, expr: Value) -> bool {
    match expr {
        Value::Bool(_) => true,
        Value::Obj(r) => match vm.engine.obj(r) {
            Obj::Pair(Value::Symbol(h), _) => matches!(
                name_of(vm, vm.engine.resolve(*h)).as_str(),
                "=" | "<" | "<=" | ">" | ">=" | "not" | "and" | "or" | "zero?" | "positive?" | "negative?"
                    | "even?" | "odd?" | "nan?"
            ),
            _ => false,
        },
        _ => false,
    }
}

fn restore<'c>(env: &mut Env<'c>, saved: Vec<(SymId, Option<FloatValue<'c>>)>) {
    for (p, old) in saved {
        match old {
            Some(v) => env.insert(p, v),
            None => env.remove(&p),
        };
    }
}

fn name_of(vm: &Vm, s: SymId) -> String {
    vm.engine.interner.sym_name(s).to_string()
}

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}
