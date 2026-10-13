//! The codegen half: emit LLVM IR for the compilable subset, JIT it, and
//! check it against T0 on a grid.
//!
//! Scope is deliberately the *innermost* functions. The point is not a
//! compiler, it is three answers the representation can only give by being
//! compiled against:
//!
//! 1. `let` expands to `((lambda (v ...) body) init ...)`, so the compiler
//!    must **beta-reduce an immediately-applied literal lambda** or allocate
//!    a frame per `let`. Every `let*` is a chain of them.
//! 2. A named `let` -- every loop in the corpus -- expands through `letrec`
//!    into a mutable cell plus `set!`, which erases the loop. Measured here
//!    rather than argued about.
//! 3. Item (5): whether a 3-float return actually lands in registers.

use std::collections::HashMap;

use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, FloatValue, FunctionValue};

use scm::heap::Obj;

type BB<'ctx> = inkwell::basic_block::BasicBlock<'ctx>;
use scm::{SymId, Value, Vm};

/// Primitives that become a single LLVM instruction or a libm call.
fn libm_name(n: &str) -> Option<&'static str> {
    Some(match n {
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "sqrt" => "sqrt",
        "exp" => "exp",
        "log" => "log",
        "floor" => "floor",
        "ceiling" => "ceil",
        "truncate" => "trunc",
        "round" => "rint",
        _ => return None,
    })
}

pub struct Cg<'ctx> {
    ctx: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    done: HashMap<String, FunctionValue<'ctx>>,
}

type R<T> = Result<T, String>;

impl<'ctx> Cg<'ctx> {
    pub fn new(ctx: &'ctx Context) -> Self {
        Self {
            ctx,
            module: ctx.create_module("leaf"),
            builder: ctx.create_builder(),
            done: HashMap::new(),
        }
    }

    /// Compile a global `(define (name a b ..) body)` whose parameters and
    /// result are all `f64`.
    pub fn function(&mut self, vm: &mut Vm, name: &str) -> R<FunctionValue<'ctx>> {
        if let Some(f) = self.done.get(name) {
            return Ok(*f);
        }
        let sym = vm.engine.interner.symbol(name);
        let Some(Value::Obj(r)) = vm.global(sym) else {
            return Err(format!("`{name}` is not defined"));
        };
        let Obj::Closure { params, body, .. } = vm.engine.obj(r).clone() else {
            return Err(format!("`{name}` is not an authored procedure"));
        };
        if body.len() != 1 {
            return Err(format!("`{name}` has a multi-form body"));
        }

        let f64t = self.ctx.f64_type();
        let args: Vec<BasicMetadataTypeEnum> = vec![f64t.into(); params.len()];
        let fty = f64t.fn_type(&args, false);
        let f = self.module.add_function(name, fty, None);
        // Insert before compiling the body so self-recursion can find it.
        self.done.insert(name.to_string(), f);

        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);

        let mut env: HashMap<SymId, FloatValue<'ctx>> = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            env.insert(*p, f.get_nth_param(i as u32).unwrap().into_float_value());
        }
        // A failed body must not leave a terminator-less function behind, or
        // the whole module stops verifying and one refusal looks like a
        // broken compiler.
        match self.expr(vm, body[0], &mut env, f) {
            Ok(out) => {
                self.builder.build_return(Some(&out)).map_err(e)?;
                Ok(f)
            }
            Err(why) => {
                self.done.remove(name);
                unsafe { f.delete() };
                Err(why)
            }
        }
    }

    /// A texel-shaped function: `(define (name u v) (list e1 e2 e3))` with a
    /// `{double,double,double}` return. Item (5) -- the allocation per texel
    /// that the whole exercise exists to delete.
    pub fn function3(&mut self, vm: &mut Vm, name: &str) -> R<FunctionValue<'ctx>> {
        let sym = vm.engine.interner.symbol(name);
        let Some(Value::Obj(r)) = vm.global(sym) else {
            return Err(format!("`{name}` is not defined"));
        };
        let Obj::Closure { params, body, .. } = vm.engine.obj(r).clone() else {
            return Err(format!("`{name}` is not an authored procedure"));
        };

        let f64t = self.ctx.f64_type();
        let ret = self.ctx.struct_type(&[f64t.into(), f64t.into(), f64t.into()], false);
        let args: Vec<BasicMetadataTypeEnum> = vec![f64t.into(); params.len()];
        let f = self.module.add_function(name, ret.fn_type(&args, false), None);
        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);

        let mut env: HashMap<SymId, FloatValue<'ctx>> = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            env.insert(*p, f.get_nth_param(i as u32).unwrap().into_float_value());
        }
        // Peel the `let`s, then expect a literal 3-list.
        let three = self.three(vm, body[0], &mut env, f)?;
        let mut agg = ret.get_undef();
        for (i, v) in three.iter().enumerate() {
            agg = self
                .builder
                .build_insert_value(agg, *v, i as u32, "c")
                .map_err(e)?
                .into_struct_value();
        }
        self.builder.build_return(Some(&agg)).map_err(e)?;
        self.done.insert(name.to_string(), f);
        Ok(f)
    }

    /// Walk through `let`-shaped applications until a literal 3-element
    /// `list` or `map` call is reached, and return its three components.
    fn three(
        &mut self,
        vm: &mut Vm,
        expr: Value,
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<[FloatValue<'ctx>; 3]> {
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr)?;
        // Peel the `let`s wrapping the colour expression.
        if let Some(l) = as_let(vm, &parts)?
            && l.name.is_none()
        {
            for (p, i) in l.formals.iter().zip(l.inits) {
                let v = self.expr(vm, i, env, f)?;
                env.insert(*p, v);
            }
            return self.three(vm, l.body, env, f);
        }
        if let Some(&Value::Symbol(h)) = parts.first() {
            let hn = name_of(vm, h);
            if hn == "list" && parts.len() == 4 {
                let a = self.expr(vm, parts[1], env, f)?;
                let b = self.expr(vm, parts[2], env, f)?;
                let c = self.expr(vm, parts[3], env, f)?;
                return Ok([a, b, c]);
            }
            // `(map (lambda (c) ...) (list a b c))` -- the corpus's actual
            // spelling, specialised at the call site.
            if hn == "map" && parts.len() == 3 {
                let seq = expand(vm, parts[2])?;
                let items = list(vm, seq)?;
                if items.len() == 4 && matches!(items[0], Value::Symbol(s) if name_of(vm, s) == "list")
                {
                    let lam = expand(vm, parts[1])?;
                    let fparts = list(vm, lam)?;
                    let mut out = Vec::new();
                    for item in &items[1..] {
                        // Inline the lambda once per element.
                        let (formals, inner) = as_lambda(vm, &fparts)?;
                        let v = self.expr(vm, *item, env, f)?;
                        let saved: Vec<_> =
                            formals.iter().map(|p| (*p, env.get(p).copied())).collect();
                        for p in &formals {
                            env.insert(*p, v);
                        }
                        out.push(self.expr(vm, inner, env, f)?);
                        for (p, old) in saved {
                            match old {
                                Some(v) => env.insert(p, v),
                                None => env.remove(&p),
                            };
                        }
                    }
                    return Ok([out[0], out[1], out[2]]);
                }
            }
        }
        Err("not a 3-element return".into())
    }

    fn expr(
        &mut self,
        vm: &mut Vm,
        expr: Value,
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<FloatValue<'ctx>> {
        let f64t = self.ctx.f64_type();
        match expr {
            Value::Flonum(x) => Ok(f64t.const_float(x)),
            Value::Fixnum(n) => Ok(f64t.const_float(n as f64)),
            Value::Symbol(s) => {
                if let Some(v) = env.get(&s) {
                    return Ok(*v);
                }
                let r = vm.engine.resolve(s);
                if let Some(v) = env.get(&r) {
                    return Ok(*v);
                }
                // A global constant folds in; a global table does not, and
                // that is item (2) showing up as a hard edge.
                match vm.global(r) {
                    Some(Value::Flonum(x)) => Ok(f64t.const_float(x)),
                    Some(Value::Fixnum(n)) => Ok(f64t.const_float(n as f64)),
                    _ => Err(format!(
                        "`{}` is not a local or a scalar constant",
                        name_of(vm, r)
                    )),
                }
            }
            Value::Obj(_) => self.combination(vm, expr, env, f),
            other => Err(format!("{other:?} has no f64 form")),
        }
    }

    fn combination(
        &mut self,
        vm: &mut Vm,
        expr: Value,
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<FloatValue<'ctx>> {
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr)?;

        // `let` and named `let` are CORE FORMS. Findings 1 and 2 are gone as
        // a result: there is no applied literal lambda to beta-reduce and no
        // `letrec` cell to undo, because the binding structure and the loop
        // are in the form where they were written. This block replaced both,
        // and it is shorter than either.
        if let Some(l) = as_let(vm, &parts)? {
            return match l.name {
                None => {
                    let saved: Vec<_> =
                        l.formals.iter().map(|p| (*p, env.get(p).copied())).collect();
                    let vals: Vec<_> = l
                        .inits
                        .iter()
                        .map(|i| self.expr(vm, *i, env, f))
                        .collect::<R<Vec<_>>>()?;
                    for (p, v) in l.formals.iter().zip(vals) {
                        env.insert(*p, v);
                    }
                    let out = self.expr(vm, l.body, env, f);
                    for (p, old) in saved {
                        match old {
                            Some(v) => env.insert(p, v),
                            None => env.remove(&p),
                        };
                    }
                    out
                }
                // Item (3): a tail self-call becomes a branch back to a
                // header block with phi nodes.
                Some(_) => {
                    let inits = l.inits.clone();
                    self.loop_(vm, l, &inits, env, f)
                }
            };
        }

        let Some(&Value::Symbol(h)) = parts.first() else {
            // Finding 2, and it is the expensive one. A named `let` -- which
            // is how every loop in the corpus is written -- expands to
            //
            //   ((letrec ((loop (lambda (..) ..))) loop) init ..)
            //
            // and `letrec` expands again into a binding initialised to `#f`
            // and then `set!` to the closure. So the operator position holds
            // an applied lambda with a multi-form body that mutates its own
            // formal, and THE LOOP IS GONE: what is left is a closure
            // allocation and an indirect self-call per iteration. Say so
            // precisely, because the generic message hid it.
            return Err("a computed operator cannot be compiled".into());
        };
        let hn = name_of(vm, vm.engine.resolve(h));
        let args = &parts[1..];

        // n-ary arithmetic
        if matches!(hn.as_str(), "+" | "-" | "*" | "/") {
            let mut vals = Vec::new();
            for a in args {
                vals.push(self.expr(vm, *a, env, f)?);
            }
            if vals.is_empty() {
                return Err(format!("`{hn}` with no arguments"));
            }
            // Unary minus and reciprocal.
            if vals.len() == 1 {
                return match hn.as_str() {
                    "-" => self
                        .builder
                        .build_float_neg(vals[0], "neg")
                        .map_err(e),
                    "/" => self
                        .builder
                        .build_float_div(self.ctx.f64_type().const_float(1.0), vals[0], "rec")
                        .map_err(e),
                    _ => Ok(vals[0]),
                };
            }
            let mut acc = vals[0];
            for v in &vals[1..] {
                acc = match hn.as_str() {
                    "+" => self.builder.build_float_add(acc, *v, "add"),
                    "-" => self.builder.build_float_sub(acc, *v, "sub"),
                    "*" => self.builder.build_float_mul(acc, *v, "mul"),
                    _ => self.builder.build_float_div(acc, *v, "div"),
                }
                .map_err(e)?;
            }
            return Ok(acc);
        }

        if hn == "abs" && args.len() == 1 {
            let v = self.expr(vm, args[0], env, f)?;
            return self.call_intrinsic("llvm.fabs.f64", &[v]);
        }
        if hn == "square" && args.len() == 1 {
            let v = self.expr(vm, args[0], env, f)?;
            return self.builder.build_float_mul(v, v, "sq").map_err(e);
        }
        if let Some(m) = libm_name(&hn) {
            if args.len() != 1 {
                return Err(format!("`{hn}` wants one argument here"));
            }
            let v = self.expr(vm, args[0], env, f)?;
            return self.call_extern(m, &[v]);
        }
        if (hn == "min" || hn == "max") && args.len() == 2 {
            let a = self.expr(vm, args[0], env, f)?;
            let b = self.expr(vm, args[1], env, f)?;
            let n = if hn == "min" { "llvm.minnum.f64" } else { "llvm.maxnum.f64" };
            return self.call_intrinsic(n, &[a, b]);
        }
        if hn == "if" && (args.len() == 2 || args.len() == 3) {
            return self.conditional(vm, args, env, f);
        }

        // A call to another authored function in the subset.
        if vm.global(vm.engine.resolve(h)).is_some() {
            let callee = self.function(vm, &hn)?;
            let mut vals: Vec<BasicMetadataValueEnum> = Vec::new();
            for a in args {
                vals.push(self.expr(vm, *a, env, f)?.into());
            }
            let cs = self.builder.build_call(callee, &vals, "c").map_err(e)?;
            return Ok(cs
                .try_as_basic_value()
                // inkwell 0.10: this returns ValueKind, not Either, so the
                // accessor is `basic()` and not `left()`. A seventh API
                // gotcha for the s0-findings list.
                .basic()
                .ok_or("a call with no value")?
                .into_float_value());
        }
        Err(format!("`{hn}` has no compiled form"))
    }

    /// A self-tail-recursive loop: phi nodes in a header block, a branch back
    /// for each tail self-call, and a result phi at the exit. Plan §6 item (3)
    /// -- "bounded loops" -- which the expander had erased.
    fn loop_(
        &mut self,
        vm: &mut Vm,
        l: NamedLet,
        inits: &[Value],
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<FloatValue<'ctx>> {
        if l.name.is_none() {
            return Err("loop_ called on an unnamed `let`".into());
        }
        if inits.len() != l.formals.len() {
            return Err("a named `let` whose arity does not match its bindings".into());
        }
        let f64t = self.ctx.f64_type();
        let mut entry_vals = Vec::new();
        for i in inits {
            entry_vals.push(self.expr(vm, *i, env, f)?);
        }
        let pre = self.builder.get_insert_block().unwrap();
        let header = self.ctx.append_basic_block(f, "loop");
        let exit = self.ctx.append_basic_block(f, "done");
        self.builder.build_unconditional_branch(header).map_err(e)?;

        self.builder.position_at_end(header);
        let mut phis = Vec::new();
        for (i, v) in l.formals.iter().enumerate() {
            let phi = self.builder.build_phi(f64t, "i").map_err(e)?;
            phi.add_incoming(&[(&entry_vals[i], pre)]);
            env.insert(*v, phi.as_basic_value().into_float_value());
            phis.push(phi);
        }

        let mut backedges: Vec<(Vec<FloatValue<'ctx>>, BB<'ctx>)> = Vec::new();
        let mut results: Vec<(FloatValue<'ctx>, BB<'ctx>)> = Vec::new();
        self.tail(vm, l.body, &l, header, env, f, exit, &mut backedges, &mut results)?;

        for (args, blk) in &backedges {
            for (i, phi) in phis.iter().enumerate() {
                phi.add_incoming(&[(&args[i], *blk)]);
            }
        }
        self.builder.position_at_end(exit);
        if results.is_empty() {
            return Err("a loop with no exit".into());
        }
        let out = self.builder.build_phi(f64t, "r").map_err(e)?;
        for (v, blk) in &results {
            out.add_incoming(&[(v, *blk)]);
        }
        Ok(out.as_basic_value().into_float_value())
    }

    /// Compile `expr` in tail position of a loop: either a self-call (branch
    /// back to the header) or a value (branch to the exit).
    #[allow(clippy::too_many_arguments)]
    fn tail(
        &mut self,
        vm: &mut Vm,
        expr: Value,
        l: &NamedLet,
        header: BB<'ctx>,
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
        exit: BB<'ctx>,
        backedges: &mut Vec<(Vec<FloatValue<'ctx>>, BB<'ctx>)>,
        results: &mut Vec<(FloatValue<'ctx>, BB<'ctx>)>,
    ) -> R<()> {
        let expr = expand(vm, expr)?;
        if let Ok(parts) = list(vm, expr)
            && let Some(&Value::Symbol(h)) = parts.first()
        {
            let resolved = vm.engine.resolve(h);
            if Some(resolved) == l.name {
                let mut args = Vec::new();
                for a in &parts[1..] {
                    args.push(self.expr(vm, *a, env, f)?);
                }
                if args.len() != l.formals.len() {
                    return Err("a self-call with the wrong arity".into());
                }
                let blk = self.builder.get_insert_block().unwrap();
                backedges.push((args, blk));
                self.builder.build_unconditional_branch(header).map_err(e)?;
                return Ok(());
            }
            if name_of(vm, resolved) == "if" && parts.len() >= 3 {
                let cond = self.predicate(vm, parts[1], env, f)?;
                let tb = self.ctx.append_basic_block(f, "t");
                let eb = self.ctx.append_basic_block(f, "e");
                self.builder.build_conditional_branch(cond, tb, eb).map_err(e)?;
                self.builder.position_at_end(tb);
                self.tail(vm, parts[2], l, header, env, f, exit, backedges, results)?;
                self.builder.position_at_end(eb);
                match parts.get(3) {
                    Some(&x) => {
                        self.tail(vm, x, l, header, env, f, exit, backedges, results)?
                    }
                    None => {
                        let z = self.ctx.f64_type().const_float(0.0);
                        let blk = self.builder.get_insert_block().unwrap();
                        results.push((z, blk));
                        self.builder.build_unconditional_branch(exit).map_err(e)?;
                    }
                }
                return Ok(());
            }
        }
        let v = self.expr(vm, expr, env, f)?;
        let blk = self.builder.get_insert_block().unwrap();
        results.push((v, blk));
        self.builder.build_unconditional_branch(exit).map_err(e)?;
        Ok(())
    }

    fn conditional(
        &mut self,
        vm: &mut Vm,
        args: &[Value],
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<FloatValue<'ctx>> {
        let cond = self.predicate(vm, args[0], env, f)?;
        let then_b = self.ctx.append_basic_block(f, "then");
        let else_b = self.ctx.append_basic_block(f, "else");
        let join_b = self.ctx.append_basic_block(f, "join");
        self.builder.build_conditional_branch(cond, then_b, else_b).map_err(e)?;

        self.builder.position_at_end(then_b);
        let a = self.expr(vm, args[1], env, f)?;
        self.builder.build_unconditional_branch(join_b).map_err(e)?;
        let then_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(else_b);
        let b = match args.get(2) {
            Some(x) => self.expr(vm, *x, env, f)?,
            None => self.ctx.f64_type().const_float(0.0),
        };
        self.builder.build_unconditional_branch(join_b).map_err(e)?;
        let else_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(join_b);
        let phi = self.builder.build_phi(self.ctx.f64_type(), "v").map_err(e)?;
        phi.add_incoming(&[(&a, then_end), (&b, else_end)]);
        Ok(phi.as_basic_value().into_float_value())
    }

    fn predicate(
        &mut self,
        vm: &mut Vm,
        expr: Value,
        env: &mut HashMap<SymId, FloatValue<'ctx>>,
        f: FunctionValue<'ctx>,
    ) -> R<inkwell::values::IntValue<'ctx>> {
        use inkwell::FloatPredicate as P;
        let expr = expand(vm, expr)?;
        let parts = list(vm, expr)?;
        let Some(&Value::Symbol(h)) = parts.first() else {
            return Err("a non-comparison condition".into());
        };
        let hn = name_of(vm, vm.engine.resolve(h));
        let p = match hn.as_str() {
            "<" => P::OLT,
            "<=" => P::OLE,
            ">" => P::OGT,
            ">=" => P::OGE,
            "=" => P::OEQ,
            _ => return Err(format!("`{hn}` is not a compiled comparison")),
        };
        if parts.len() != 3 {
            return Err(format!("`{hn}` with {} arguments", parts.len() - 1));
        }
        let a = self.expr(vm, parts[1], env, f)?;
        let b = self.expr(vm, parts[2], env, f)?;
        self.builder.build_float_compare(p, a, b, "cmp").map_err(e)
    }

    fn call_intrinsic(&mut self, n: &str, args: &[FloatValue<'ctx>]) -> R<FloatValue<'ctx>> {
        self.call_extern(n, args)
    }

    fn call_extern(&mut self, n: &str, args: &[FloatValue<'ctx>]) -> R<FloatValue<'ctx>> {
        let f64t = self.ctx.f64_type();
        let f = self.module.get_function(n).unwrap_or_else(|| {
            let tys: Vec<BasicMetadataTypeEnum> = vec![f64t.into(); args.len()];
            self.module.add_function(n, f64t.fn_type(&tys, false), None)
        });
        let vals: Vec<BasicMetadataValueEnum> = args.iter().map(|v| (*v).into()).collect();
        let cs = self.builder.build_call(f, &vals, "x").map_err(e)?;
        Ok(cs
            .try_as_basic_value()
            .basic()
            .ok_or("an extern call with no value")?
            .into_float_value())
    }

    pub fn verify(&self) -> R<()> {
        self.module.verify().map_err(|s| s.to_string())
    }

    pub fn ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn jit(self) -> R<inkwell::execution_engine::ExecutionEngine<'ctx>> {
        self.module
            .create_jit_execution_engine(OptimizationLevel::Aggressive)
            .map_err(|s| s.to_string())
    }
}

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

fn name_of(vm: &Vm, s: SymId) -> String {
    vm.engine.interner.sym_name(s).to_string()
}

fn expand(vm: &mut Vm, mut expr: Value) -> R<Value> {
    for _ in 0..10_000 {
        match vm.expand_once(expr) {
            Ok(Some(next)) => expr = next,
            Ok(None) => return Ok(expr),
            Err(err) => return Err(format!("expansion failed: {err}")),
        }
    }
    Err("expansion did not terminate".into())
}

fn list(vm: &Vm, mut v: Value) -> R<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        match v {
            Value::Null => return Ok(out),
            Value::Obj(r) => match vm.engine.obj(r) {
                Obj::Pair(car, cdr) => {
                    out.push(*car);
                    v = *cdr;
                }
                _ => return Err("an improper list".into()),
            },
            _ => return Err("an improper list".into()),
        }
    }
}

/// A `let`, core form: `(let [name] ((v init) ...) body)`.
///
/// One extractor where there used to be two. Before `let` and named `let`
/// became core forms this file needed `as_let` to recognise an applied
/// literal lambda *and* `as_named_let` to undo a `letrec` cell holding a
/// self-reference -- and the second one only worked by refusing to expand the
/// operator, because expanding it destroyed the shape being looked for. The
/// structure is now in the source where the author put it.
pub struct NamedLet {
    /// `Some` for a named `let`: the loop variable.
    pub name: Option<SymId>,
    pub formals: Vec<SymId>,
    pub inits: Vec<Value>,
    pub body: Value,
}

fn as_let(vm: &mut Vm, parts: &[Value]) -> R<Option<NamedLet>> {
    let Some(&Value::Symbol(h)) = parts.first() else {
        return Ok(None);
    };
    if name_of(vm, vm.engine.resolve(h)) != "let" {
        return Ok(None);
    }
    let args = &parts[1..];
    let (name, rest) = match args.first() {
        Some(&Value::Symbol(n)) => (Some(vm.engine.resolve(n)), &args[1..]),
        _ => (None, args),
    };
    let Some((spec, body)) = rest.split_first() else {
        return Err("`let` with no binding list".into());
    };
    // One body form: the subset has no sequencing, and a `let` body with two
    // forms in a pure function means the first one was useless anyway.
    if body.len() != 1 {
        return Err("a `let` body with more than one form".into());
    }
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
    Ok(Some(NamedLet { name, formals, inits, body: body[0] }))
}

/// A binding list or a binding: a list, or a bracket vector (plan §4.9).
fn binding_items(vm: &Vm, v: Value) -> R<Vec<Value>> {
    if let Value::Obj(r) = v
        && let Obj::Vector {
            items,
            bracket: true,
        } = vm.engine.obj(r)
    {
        return Ok(items.clone());
    }
    list(vm, v)
}

fn as_lambda(vm: &mut Vm, parts: &[Value]) -> R<(Vec<SymId>, Value)> {
    let Some(&Value::Symbol(h)) = parts.first() else {
        return Err("not a lambda".into());
    };
    if name_of(vm, vm.engine.resolve(h)) != "lambda" || parts.len() != 3 {
        return Err("not a one-body lambda".into());
    }
    let formals = list(vm, parts[1])?
        .into_iter()
        .filter_map(|x| match x {
            Value::Symbol(s) => Some(s),
            _ => None,
        })
        .collect();
    Ok((formals, parts[2]))
}

// ---- the report ---------------------------------------------------------

pub fn run(vm: &mut Vm) {
    let ctx = Context::create();
    let mut cg = Cg::new(&ctx);

    let mut built = Vec::new();
    for n in ["fract", "fade", "mix", "hash2", "vnoise", "fbm", "ridge"] {
        match cg.function(vm, n) {
            Ok(_) => {
                println!("  compiled    {n}");
                built.push(n);
            }
            Err(why) => println!("  NOT BUILT   {n:<10} {why}"),
        }
    }
    // Item (5): the 3-float return.
    let three = match cg.function3(vm, "texel3") {
        Ok(_) => {
            println!("  compiled    texel3     -> {{double,double,double}}");
            true
        }
        Err(why) => {
            println!("  NOT BUILT   texel3     {why}");
            false
        }
    };

    if let Err(why) = cg.verify() {
        println!("\n  !! the module does not verify: {why}");
        return;
    }
    println!("\n  the module verifies.");
    let ir = cg.ir();
    let frames = ir.matches("alloca").count();
    println!(
        "  IR: {} lines, {} alloca ({} = no frame allocated for any `let`)",
        ir.lines().count(),
        frames,
        if frames == 0 { "0" } else { "NOT 0" }
    );

    let ee = match cg.jit() {
        Ok(ee) => ee,
        Err(why) => {
            println!("  !! no JIT: {why}");
            return;
        }
    };

    println!("\n## 3. Differential test against T0\n");
    let mut worst2: f64 = 0.0;
    let mut n2 = 0usize;
    for name in &built {
        type F2 = unsafe extern "C" fn(f64, f64) -> f64;
        type F1 = unsafe extern "C" fn(f64) -> f64;
        type F3 = unsafe extern "C" fn(f64, f64, f64) -> f64;
        for i in 0..7 {
            for j in 0..7 {
                let x = -2.5 + i as f64 * 0.9;
                let y = 0.125 + j as f64 * 0.37;
                let (native, src) = unsafe {
                    match *name {
                        "fract" | "fade" => {
                            let f: F1 = std::mem::transmute(
                                ee.get_function_address(name).unwrap(),
                            );
                            (f(x), format!("({name} {x:?})"))
                        }
                        "mix" => {
                            let f: F3 = std::mem::transmute(
                                ee.get_function_address(name).unwrap(),
                            );
                            (f(x, y, 0.25), format!("({name} {x:?} {y:?} 0.25)"))
                        }
                        // The loops: `fbm` takes its octave count as a third
                        // argument, which is a fixnum in the source and an
                        // f64 here -- itself worth noting, since the subset
                        // has one numeric type and Scheme has two.
                        "fbm" => {
                            let f: F3 = std::mem::transmute(
                                ee.get_function_address(name).unwrap(),
                            );
                            (f(x, y, 4.0), format!("({name} {x:?} {y:?} 4)"))
                        }
                        _ => {
                            let f: F2 = std::mem::transmute(
                                ee.get_function_address(name).unwrap(),
                            );
                            (f(x, y), format!("({name} {x:?} {y:?})"))
                        }
                    }
                };
                let interp = match vm.eval_str(&src) {
                    Ok(Value::Flonum(v)) => v,
                    Ok(Value::Fixnum(v)) => v as f64,
                    other => {
                        println!("  T0 refused {src}: {other:?}");
                        continue;
                    }
                };
                let d = (native - interp).abs();
                worst2 = worst2.max(d);
                n2 += 1;
                if d > 1e-12 {
                    println!("  DISAGREE {src}: native {native} vs T0 {interp}");
                }
            }
        }
    }
    println!(
        "  {} samples across {} functions, worst |native - T0| = {:.3e}",
        n2,
        built.len(),
        worst2
    );

    if three {
        type F3v = unsafe extern "C" fn(f64, f64) -> [f64; 3];
        let f: F3v = unsafe { std::mem::transmute(ee.get_function_address("texel3").unwrap()) };
        let got = unsafe { f(0.35, 0.72) };
        let want = vm.eval_str("(texel3 0.35 0.72)").unwrap();
        println!(
            "  texel3 native  {:?}\n  texel3 T0      {}",
            got,
            scm::write_str(&vm.engine, want)
        );

        // The cost per texel, which is the whole justification in §3.2 and
        // the one number leaf-survey.md measured on ray's side. A small grid:
        // the point is the ratio, not a benchmark.
        println!("\n## 4. Cost per texel (the §3.2 justification)\n");
        const N: usize = 64;
        let t0 = std::time::Instant::now();
        let mut sink = 0.0f64;
        for i in 0..N {
            for j in 0..N {
                let c = unsafe { f(i as f64 / N as f64, j as f64 / N as f64) };
                sink += c[0] + c[1] + c[2];
            }
        }
        let native_ns = t0.elapsed().as_nanos() as f64 / (N * N) as f64;

        let t1 = std::time::Instant::now();
        let src = format!(
            "(let loop ([i 0] [acc 0.0])
               (if (= i {N}) acc
                 (loop (+ i 1)
                   (let inner ([j 0] [a acc])
                     (if (= j {N}) a
                       (inner (+ j 1)
                         (+ a (apply + (texel3 (/ i {N}.0) (/ j {N}.0)))))))))) "
        );
        let interp = vm.eval_str(&src);
        let t0_ns = t1.elapsed().as_nanos() as f64 / (N * N) as f64;
        match interp {
            Ok(_) => println!(
                "  {}x{} texels:  native {:>8.1} ns/texel   T0 {:>9.1} ns/texel   {:.0}x",
                N, N, native_ns, t0_ns, t0_ns / native_ns
            ),
            Err(why) => println!("  T0 could not run the grid: {why}"),
        }
        std::hint::black_box(sink);
    }
}
