//! The T1 compiler: a procedure body, or a top-level form, to `Code`.
//!
//! Resolution happens once, here, instead of on every evaluation: a
//! variable becomes a (depth, index) into the environment chain or a global
//! cell; a macro is expanded; a special form is decided. Everything the
//! compiler is unsure of it **declines**, and the caller runs the form on
//! T0 -- so a malformed `if` still raises T0's error, `let-syntax` still
//! works, and agreement between the tiers is by construction. The count of
//! declines is the honest measure of how much of a program T1 ran.
//!
//! Scoping follows T0's `lookup` exactly: the chain for the name as
//! written, then a global of that name, then a global of its
//! hygiene-resolved name. A closure created by T0 and compiled on its
//! first call resolves its free variables against the *actual* runtime
//! chain it captured, so the two engines can nest in either order.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{Formals, Vm};
use crate::code::{Code, Lambda, Op};
use crate::heap::Obj;
use crate::value::{ObjRef, SymId, Value};

/// Why the compiler would not take a form. Not an error: T0 gets the form.
pub(super) struct Decline(pub String);

type R<T> = Result<T, Decline>;

fn decline<T>(why: impl Into<String>) -> R<T> {
    Err(Decline(why.into()))
}

enum Var {
    /// `checked`: a slot an internal `define` fills, read through
    /// `LocalDef` so a use before the define is "unbound" as on T0.
    Local { depth: u16, idx: u16, checked: bool },
    /// A slot of a permanent frame -- a library's -- by address.
    Slot { frame: ObjRef, idx: u16 },
    Global(SymId, SymId),
}

struct Cx<'a> {
    vm: &'a mut Vm,
    /// Compile-time frames, innermost last; each becomes one runtime
    /// environment frame.
    scopes: Vec<Vec<SymId>>,
    /// For each scope, the index from which its slots belong to internal
    /// defines rather than parameters or `let` variables.
    defs_from: Vec<usize>,
    /// The runtime chain beneath the compile-time frames, for a closure
    /// compiled after its environment already exists.
    runtime: Option<ObjRef>,
    ops: Vec<Op>,
    consts: Vec<Value>,
    globals: Vec<(SymId, SymId)>,
    global_index: HashMap<SymId, u32>,
    lambdas: Vec<Rc<Lambda>>,
    names: Vec<Rc<Vec<SymId>>>,
    forms: Vec<Value>,
    blocks: Vec<Rc<Code>>,
    calls: Vec<(u32, u32, u32)>,
    defines: Rc<Vec<SymId>>,
}

impl Vm {
    /// A top-level form. Declines what T0 should see: macro definitions,
    /// malformed syntax, anything with a local macro.
    pub(super) fn compile_toplevel(&mut self, form: Value) -> R<Rc<Code>> {
        let mut cx = Cx::new(self, Vec::new(), Vec::new(), None, Rc::new(Vec::new()));
        // A top-level `define-syntax` is the one syntax definition that can
        // be handed to T0 as a sub-form: nothing in this unit was compiled
        // against it. Inside a body the uses that follow it would have
        // been, so there it declines the unit.
        let expanded = cx.expand(form)?;
        if let Value::Obj(r) = expanded
            && let Obj::Pair(Value::Symbol(h), _) = *cx.vm.engine.obj(r)
            && let Some(sp) = cx.special(h)?
            && (sp == cx.vm.syms.define_syntax
                || sp == cx.vm.syms.define_library
                || sp == cx.vm.syms.import
                || sp == cx.vm.syms.cond_expand)
        {
            // Library forms are T0's as well: nothing in this unit was
            // compiled against what they bind.
            cx.interp(expanded)?;
            cx.ops.push(Op::Return);
            return Ok(cx.finish());
        }
        // Not in tail position: a top-level unit has no caller to return
        // to, so nothing is lost.
        cx.expr(expanded, false)?;
        cx.ops.push(Op::Return);
        Ok(cx.finish())
    }

    /// A closure's body, given the environment it captured. Called on the
    /// first application of a closure T0 created.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile_closure(
        &mut self,
        params: &[SymId],
        opt: &[crate::heap::Param],
        named: &[crate::heap::NamedParam],
        keys: Option<SymId>,
        rest: Option<SymId>,
        body: &[Value],
        env: Option<ObjRef>,
    ) -> R<Rc<Code>> {
        let mut frame: Vec<SymId> = params.to_vec();
        frame.extend(opt.iter().map(|p| p.name));
        frame.extend(rest);
        frame.extend(named.iter().map(|p| p.name));
        frame.extend(keys);
        let cx = Cx::new(self, Vec::new(), Vec::new(), env, Rc::new(Vec::new()));
        cx.body_unit(frame, body)
    }
}

impl<'a> Cx<'a> {
    fn new(vm: &'a mut Vm, scopes: Vec<Vec<SymId>>, defs_from: Vec<usize>, runtime: Option<ObjRef>, defines: Rc<Vec<SymId>>) -> Self {
        Cx {
            vm,
            scopes,
            defs_from,
            runtime,
            ops: Vec::new(),
            consts: Vec::new(),
            globals: Vec::new(),
            global_index: HashMap::new(),
            lambdas: Vec::new(),
            names: Vec::new(),
            forms: Vec::new(),
            blocks: Vec::new(),
            calls: Vec::new(),
            defines,
        }
    }

    fn finish(self) -> Rc<Code> {
        Rc::new(Code {
            ops: self.ops,
            consts: self.consts,
            globals: self.globals,
            lambdas: self.lambdas,
            names: self.names,
            forms: self.forms,
            blocks: self.blocks,
            calls: self.calls,
            defines: self.defines,
            macro_gen: self.vm.macro_gen,
            hits: Cell::new(0),
            native: Cell::new(None),
            native_declined: Cell::new(false),
            max_depth: Cell::new(0),
        })
    }

    /// A procedure body in a fresh frame of `frame` (its parameters), plus
    /// the slots its internal defines need. Ends in `Return`.
    fn body_unit(mut self, mut frame: Vec<SymId>, body: &[Value]) -> R<Rc<Code>> {
        if body.is_empty() {
            return decline("an empty body");
        }
        let mut defs = Vec::new();
        self.scan_defines(body, &mut defs)?;
        self.defs_from.push(frame.len());
        frame.extend(defs.iter().copied());
        self.defines = Rc::new(defs);
        self.scopes.push(frame);
        self.body(body, true)?;
        self.ops.push(Op::Return);
        Ok(self.finish())
    }

    // ---- resolution ------------------------------------------------------

    /// T0's `lookup_exact` over the chain, then its global rules.
    fn resolve(&self, sym: SymId) -> R<Var> {
        let mut depth: usize = 0;
        for (scope, &from) in self.scopes.iter().zip(&self.defs_from).rev() {
            if let Some(idx) = scope.iter().rposition(|&s| s == sym) {
                return Ok(Var::Local { depth: narrow(depth)?, idx: narrow(idx)?, checked: idx >= from });
            }
            depth += 1;
        }
        let mut cur = self.runtime;
        while let Some(r) = cur {
            let Obj::Env { parent, slots } = self.vm.engine.obj(r) else {
                return decline("a corrupt environment");
            };
            if let Some(idx) = slots.iter().rposition(|(s, _)| *s == sym) {
                if let Value::Obj(m) = slots[idx].1
                    && matches!(self.vm.engine.obj(m), Obj::Macro(_))
                {
                    return decline("a local macro");
                }
                // A frame that already exists: whether this slot is a
                // define's cannot be told, so it is read checked.
                return Ok(Var::Local { depth: narrow(depth)?, idx: narrow(idx)?, checked: true });
            }
            depth += 1;
            cur = *parent;
        }
        let resolved = self.vm.engine.resolve(sym);
        // A template-introduced name from a macro defined in a *local*
        // scope means what it meant there (T0's `alias_env` step). A
        // library's frame is such a scope and it is permanent, so a
        // reference into it is a direct slot read; any other local scope
        // has no address this compiler can give it, and T0 keeps those
        // closures -- the suite's "forward hygienic refs" case.
        if resolved != sym
            && let Some(Some(def)) = self.vm.engine.alias_env(sym)
        {
            if let Obj::Env { parent: None, slots } = self.vm.engine.obj(def)
                && let Some(idx) = slots.iter().rposition(|(s, _)| self.vm.engine.resolve(*s) == resolved)
            {
                if let Value::Obj(m) = slots[idx].1
                    && matches!(self.vm.engine.obj(m), Obj::Macro(_))
                {
                    return decline("a local macro");
                }
                return Ok(Var::Slot { frame: def, idx: narrow(idx)? });
            }
            return decline("a reference through a locally defined macro");
        }
        Ok(Var::Global(sym, resolved))
    }

    fn is_local(&self, sym: SymId) -> R<bool> {
        Ok(matches!(self.resolve(sym)?, Var::Local { .. } | Var::Slot { .. }))
    }

    /// Is this head a special form here? Only when nothing shadows it --
    /// local or global -- which is T0's rule.
    fn special(&self, head: SymId) -> R<Option<SymId>> {
        if self.is_local(head)? || self.vm.global(head).is_some() {
            return Ok(None);
        }
        Ok(Some(self.vm.engine.resolve(head)))
    }

    /// Expand a macro use to a fixed point, with T0's shadowing rule: a
    /// local binding of the head means it is a call, not a macro use.
    fn expand(&mut self, mut expr: Value) -> R<Value> {
        for _ in 0..10_000 {
            let Value::Obj(r) = expr else { return Ok(expr) };
            let Obj::Pair(Value::Symbol(head), _) = *self.vm.engine.obj(r) else {
                return Ok(expr);
            };
            if self.is_local(head)? {
                return Ok(expr);
            }
            match self.vm.expand_once(expr) {
                Ok(Some(next)) => expr = next,
                Ok(None) => return Ok(expr),
                Err(e) => return decline(format!("expansion failed: {e}")),
            }
        }
        decline("macro expansion did not terminate")
    }

    // ---- emission --------------------------------------------------------

    fn konst(&mut self, v: Value) {
        let i = self.consts.len();
        self.consts.push(v);
        self.ops.push(Op::Const(i as u32));
    }

    fn global_slot(&mut self, sym: SymId, resolved: SymId) -> u32 {
        if let Some(&i) = self.global_index.get(&sym) {
            return i;
        }
        let i = self.globals.len() as u32;
        self.globals.push((sym, resolved));
        self.global_index.insert(sym, i);
        i
    }

    fn here(&self) -> u32 {
        self.ops.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        self.ops[at] = match self.ops[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            other => other,
        };
    }

    // ---- forms -------------------------------------------------------------

    /// A sequence: every value but the last is dropped.
    fn body(&mut self, forms: &[Value], tail: bool) -> R<()> {
        let Some((last, rest)) = forms.split_last() else {
            self.ops.push(Op::Unspecified);
            return Ok(());
        };
        for f in rest {
            self.expr(*f, false)?;
            self.ops.push(Op::Pop);
        }
        self.expr(*last, tail)
    }

    fn expr(&mut self, expr: Value, tail: bool) -> R<()> {
        match expr {
            Value::Fixnum(_)
            | Value::Flonum(_)
            | Value::Bool(_)
            | Value::Char(_)
            | Value::Keyword(_)
            | Value::Primitive(_)
            | Value::Unspecified
            | Value::Eof
            | Value::Undefined => {
                self.konst(expr);
                Ok(())
            }
            Value::Null => decline("empty application: ()"),
            Value::Symbol(sym) => {
                match self.resolve(sym)? {
                    Var::Local { depth, idx, checked: false } => self.ops.push(Op::Local { depth, idx }),
                    Var::Local { depth, idx, checked: true } => self.ops.push(Op::LocalDef { depth, idx, sym: sym.0 }),
                    Var::Slot { frame, idx } => self.ops.push(Op::Slot { frame: frame.0, idx }),
                    Var::Global(s, r) => {
                        let i = self.global_slot(s, r);
                        self.ops.push(Op::Global(i));
                    }
                }
                Ok(())
            }
            Value::Obj(r) => match self.vm.engine.obj(r).clone() {
                Obj::Str(_) | Obj::Bytevector(_) => {
                    self.konst(expr);
                    Ok(())
                }
                Obj::Vector { items, bracket } => {
                    if !bracket {
                        self.konst(expr);
                        return Ok(());
                    }
                    for it in &items {
                        self.expr(*it, false)?;
                    }
                    self.ops.push(Op::MakeVector(items.len() as u32));
                    Ok(())
                }
                Obj::Pair(..) => self.combination(expr, tail),
                // A closure, parameter, port … evaluates to itself in T0.
                _ => {
                    self.konst(expr);
                    Ok(())
                }
            },
        }
    }

    fn combination(&mut self, expr: Value, tail: bool) -> R<()> {
        let expr = self.expand(expr)?;
        // An expansion need not be a combination: `(and)` is `#t`, `(and x)`
        // is `x`.
        if !matches!(expr, Value::Obj(r) if matches!(self.vm.engine.obj(r), Obj::Pair(..))) {
            return self.expr(expr, tail);
        }
        let parts = self.list(expr)?;
        let Some(&head) = parts.first() else {
            return decline("empty application: ()");
        };
        if let Value::Symbol(h) = head
            && let Some(resolved) = self.special(h)?
        {
            let s = &self.vm.syms;
            let args = &parts[1..];
            if resolved == s.quote {
                return match args {
                    [v] => {
                        let d = self.vm.strip_renames(*v);
                        self.konst(d);
                        Ok(())
                    }
                    _ => decline("quote"),
                };
            }
            if resolved == s.if_ {
                return self.if_form(args, tail);
            }
            if resolved == s.begin {
                return self.body(args, tail);
            }
            if resolved == s.lambda {
                return match args {
                    [formals, body @ ..] if !body.is_empty() => self.lambda(None, *formals, body),
                    _ => decline("lambda"),
                };
            }
            if resolved == s.let_ {
                return self.let_form(args, tail);
            }
            if resolved == s.quasiquote {
                return match args {
                    [tmpl] => {
                        let form = self.vm.qq(*tmpl, 1).map_err(|e| Decline(format!("{e}")))?;
                        self.expr(form, tail)
                    }
                    _ => decline("quasiquote"),
                };
            }
            if resolved == s.try_ {
                return match args {
                    [e] => {
                        let block = self.block(*e)?;
                        self.ops.push(Op::Try(block));
                        Ok(())
                    }
                    _ => decline("%try"),
                };
            }
            if resolved == s.parameterize {
                return self.parameterize(args);
            }
            if resolved == s.set || resolved == s.define {
                return self.assign(resolved == s.define, args);
            }
            if resolved == s.define_syntax {
                // The forms after it in this body would be compiled before
                // it ran. T0 takes the whole unit.
                return decline("a syntax definition in a body");
            }
            if resolved == s.define_library || resolved == s.import || resolved == s.cond_expand {
                return decline("a library form");
            }
            if resolved == s.let_syntax || resolved == s.letrec_syntax {
                // Self-contained: T0 evaluates it here, in this
                // environment, and this code resumes with the value. Not in
                // tail position, though: a T0 sub-evaluation returns to this
                // frame, which would make a tail call inside it grow the
                // stack.
                if tail {
                    return decline("a local syntax form in tail position");
                }
                return self.interp(expr);
            }
        }
        // An application. The callee is evaluated first, then the
        // arguments left to right, as T0 does.
        let start = self.here();
        for p in &parts {
            self.expr(*p, false)?;
        }
        let form = self.forms.len() as u32;
        self.forms.push(expr);
        let argc = (parts.len() - 1) as u32;
        let call_pc = self.here();
        self.ops.push(if tail { Op::TailCall { argc, form } } else { Op::Call { argc, form } });
        self.calls.push((start, call_pc, form));
        Ok(())
    }

    fn if_form(&mut self, args: &[Value], tail: bool) -> R<()> {
        let (test, then_e, else_e) = match args {
            [t, c] => (*t, *c, None),
            [t, c, a] => (*t, *c, Some(*a)),
            _ => return decline("if"),
        };
        self.expr(test, false)?;
        let jf = self.ops.len();
        self.ops.push(Op::JumpIfFalse(0));
        self.expr(then_e, tail)?;
        let jend = self.ops.len();
        self.ops.push(Op::Jump(0));
        let else_at = self.here();
        self.patch(jf, else_at);
        match else_e {
            Some(e) => self.expr(e, tail)?,
            None => self.ops.push(Op::Unspecified),
        }
        let end = self.here();
        self.patch(jend, end);
        Ok(())
    }

    /// `(let [name] ((v init) …) body …)`, a core form in T0 too.
    fn let_form(&mut self, args: &[Value], tail: bool) -> R<()> {
        let (name, rest) = match args.first() {
            Some(&Value::Symbol(n)) => (Some(n), &args[1..]),
            _ => (None, args),
        };
        let Some((spec, body)) = rest.split_first() else {
            return decline("let");
        };
        if body.is_empty() {
            return decline("let");
        }
        let mut vars = Vec::new();
        let mut inits = Vec::new();
        for pair in self.binding_items(*spec)? {
            match self.binding_items(pair)?.as_slice() {
                [Value::Symbol(v), init] => {
                    vars.push(*v);
                    inits.push(*init);
                }
                _ => return decline("let"),
            }
        }
        // Inits in the enclosing scope.
        for init in &inits {
            self.expr(*init, false)?;
        }
        match name {
            None => {
                let mut frame = vars.clone();
                let mut defs = Vec::new();
                self.scan_defines(body, &mut defs)?;
                frame.extend(defs);
                let ni = self.names.len() as u32;
                self.names.push(Rc::new(frame.clone()));
                self.ops.push(Op::PushEnv { names: ni, ninit: vars.len() as u32 });
                self.scopes.push(frame);
                self.defs_from.push(vars.len());
                let out = self.body(body, tail);
                self.scopes.pop();
                self.defs_from.pop();
                out?;
                if !tail {
                    self.ops.push(Op::PopEnv);
                }
                Ok(())
            }
            Some(n) => {
                // The loop variable lives in a frame of its own, as in T0,
                // so the closure can see itself; the inits are already on
                // the stack, evaluated outside it.
                let ni = self.names.len() as u32;
                self.names.push(Rc::new(vec![n]));
                self.ops.push(Op::PushEnv { names: ni, ninit: 0 });
                self.scopes.push(vec![n]);
                self.defs_from.push(1);
                let lam = self.lambda_inner(Some(n), vars.clone(), Vec::new(), Vec::new(), None, None, body);
                self.scopes.pop();
                self.defs_from.pop();
                let lam = lam?;
                self.scopes.push(vec![n]);
                self.defs_from.push(1);
                self.ops.push(Op::MakeClosure(lam));
                self.ops.push(Op::SetLocal { depth: 0, idx: 0 });
                self.ops.push(Op::Local { depth: 0, idx: 0 });
                self.scopes.pop();
                self.defs_from.pop();
                let argc = vars.len() as u32;
                self.ops.push(Op::InsertBelow(argc));
                let form = self.forms.len() as u32;
                let mut call = vec![Value::Symbol(n)];
                call.extend(inits.iter().copied());
                let form_v = self.vm.engine.list(&call);
                self.forms.push(form_v);
                let call_pc = self.here();
                self.ops.push(if tail { Op::TailCall { argc, form } } else { Op::Call { argc, form } });
                self.calls.push((call_pc, call_pc, form));
                if !tail {
                    self.ops.push(Op::PopEnv);
                }
                Ok(())
            }
        }
    }

    fn lambda(&mut self, name: Option<SymId>, formals: Value, body: &[Value]) -> R<()> {
        let f = self.vm.parse_formals(formals).map_err(|e| Decline(format!("{e}")))?;
        let Formals { params, opt, named, keys, rest } = f;
        let i = self.lambda_inner(name, params, opt, named, keys, rest, body)?;
        self.ops.push(Op::MakeClosure(i));
        Ok(())
    }

    /// Compile a nested lambda in the current scope chain; returns its index.
    #[allow(clippy::too_many_arguments)]
    fn lambda_inner(
        &mut self,
        name: Option<SymId>,
        params: Vec<SymId>,
        opt: Vec<crate::heap::Param>,
        named: Vec<crate::heap::NamedParam>,
        keys: Option<SymId>,
        rest: Option<SymId>,
        body: &[Value],
    ) -> R<u32> {
        if body.is_empty() {
            return decline("a procedure body cannot be empty");
        }
        let mut frame: Vec<SymId> = params.clone();
        frame.extend(opt.iter().map(|p| p.name));
        frame.extend(rest);
        frame.extend(named.iter().map(|p| p.name));
        frame.extend(keys);
        let inner = Cx::new(self.vm, self.scopes.clone(), self.defs_from.clone(), self.runtime, Rc::new(Vec::new()));
        let code = inner.body_unit(frame, body)?;
        let i = self.lambdas.len() as u32;
        self.lambdas.push(Rc::new(Lambda {
            name,
            params: Rc::new(params),
            opt: Rc::new(opt),
            named: Rc::new(named),
            keys,
            rest,
            body: Rc::new(body.to_vec()),
            code,
        }));
        Ok(i)
    }

    /// A sub-block for `%try` and `%parameterize`: same scope, own code,
    /// run as a nested frame that returns to this one.
    fn block(&mut self, e: Value) -> R<u32> {
        let mut inner = Cx::new(self.vm, self.scopes.clone(), self.defs_from.clone(), self.runtime, Rc::new(Vec::new()));
        inner.expr(e, true)?;
        inner.ops.push(Op::Return);
        let code = inner.finish();
        let i = self.blocks.len() as u32;
        self.blocks.push(code);
        Ok(i)
    }

    fn parameterize(&mut self, args: &[Value]) -> R<()> {
        let Some((spec, body)) = args.split_first() else {
            return decline("%parameterize");
        };
        if body.is_empty() {
            return decline("%parameterize");
        }
        let pairs = self.list(*spec)?;
        let mut n = 0u32;
        for p in pairs {
            match self.list(p)?.as_slice() {
                [param, value] => {
                    self.expr(*param, false)?;
                    self.expr(*value, false)?;
                    n += 1;
                }
                _ => return decline("%parameterize"),
            }
        }
        let mut forms = vec![self.vm.engine.sym("begin")];
        forms.extend_from_slice(body);
        let body_form = self.vm.engine.list(&forms);
        let block = self.block(body_form)?;
        self.ops.push(Op::Parameterize { pairs: n, block });
        Ok(())
    }

    /// `define` and `set!`. An internal define writes a pre-allocated slot;
    /// a top-level define creates a global; `set!` finds its binding as T0's
    /// `bind` does.
    fn assign(&mut self, define: bool, args: &[Value]) -> R<()> {
        let (sym, value_is_closure) = match args {
            [Value::Obj(r), body @ ..] if define => {
                let Obj::Pair(name, formals) = *self.vm.engine.obj(*r) else {
                    return decline("define");
                };
                let Value::Symbol(name_sym) = name else {
                    return decline("define");
                };
                if body.is_empty() {
                    return decline("define");
                }
                // A docstring is a property of the definition, as in T0.
                if body.len() > 1
                    && let Value::Obj(r0) = body[0]
                    && let Obj::Str(chars) = self.vm.engine.obj(r0)
                {
                    let doc: String = chars.iter().collect();
                    let key = self.vm.engine.resolve(name_sym);
                    self.vm.docs.insert(key, doc);
                }
                self.lambda(Some(name_sym), formals, body)?;
                (name_sym, true)
            }
            [Value::Symbol(sym)] if define => {
                self.ops.push(Op::Unspecified);
                (*sym, false)
            }
            [Value::Symbol(sym), value] => {
                self.expr(*value, false)?;
                (*sym, false)
            }
            _ => return decline(if define { "define" } else { "set!" }),
        };
        let _ = value_is_closure;
        if define {
            // T0 defines into the innermost frame when there is one. The
            // slot was reserved by the body scan; if it was not, the form
            // sits somewhere the scan did not look, and T0 can have it.
            if let Some(scope) = self.scopes.last() {
                match scope.iter().rposition(|&s| s == sym) {
                    Some(idx) => self.ops.push(Op::SetLocal { depth: 0, idx: narrow(idx)? }),
                    None => return decline("a define outside a body"),
                }
            } else if self.runtime.is_some() {
                return decline("a define into an interpreted frame");
            } else {
                let r = self.vm.engine.resolve(sym);
                let i = self.global_slot(sym, r);
                self.ops.push(Op::DefineGlobal(i));
            }
        } else {
            match self.resolve(sym)? {
                Var::Local { depth, idx, checked: false } => self.ops.push(Op::SetLocal { depth, idx }),
                Var::Local { depth, idx, checked: true } => self.ops.push(Op::SetLocalDef { depth, idx, sym: sym.0 }),
                Var::Slot { .. } => return decline("set! of a library binding"),
                Var::Global(s, r) => {
                    let i = self.global_slot(s, r);
                    self.ops.push(Op::SetGlobal(i));
                }
            }
        }
        self.ops.push(Op::Unspecified);
        Ok(())
    }

    /// The names a body's internal defines bind, looking through `begin`
    /// and the branches of `if` as T0's dynamic `bind` would reach them,
    /// but not into forms that open frames of their own.
    fn scan_defines(&mut self, forms: &[Value], out: &mut Vec<SymId>) -> R<()> {
        for &f in forms {
            let f = self.expand(f)?;
            let Value::Obj(r) = f else { continue };
            if !matches!(self.vm.engine.obj(r), Obj::Pair(..)) {
                continue;
            }
            let Obj::Pair(Value::Symbol(h), _) = *self.vm.engine.obj(r) else { continue };
            let Some(resolved) = self.special(h)? else { continue };
            let s = &self.vm.syms;
            if resolved == s.define {
                let parts = self.list(f)?;
                let name = match parts.get(1) {
                    Some(Value::Symbol(n)) => *n,
                    Some(&Value::Obj(p)) => match *self.vm.engine.obj(p) {
                        Obj::Pair(Value::Symbol(n), _) => n,
                        _ => return decline("define"),
                    },
                    _ => return decline("define"),
                };
                if !out.contains(&name) {
                    out.push(name);
                }
            } else if resolved == s.begin {
                let parts = self.list(f)?;
                self.scan_defines(&parts[1..], out)?;
            } else if resolved == s.if_ {
                let parts = self.list(f)?;
                if parts.len() > 2 {
                    self.scan_defines(&parts[2..], out)?;
                }
            }
        }
        Ok(())
    }

    /// Hand `form` to T0 at run time.
    fn interp(&mut self, form: Value) -> R<()> {
        let i = self.forms.len() as u32;
        self.forms.push(form);
        self.ops.push(Op::Interp(i));
        self.vm.interp_forms += 1;
        Ok(())
    }

    // ---- helpers -------------------------------------------------------------

    fn list(&self, v: Value) -> R<Vec<Value>> {
        self.vm.list_to_vec(v).map_err(|e| {
            Decline(format!("{e}: {}", super::abbreviate(&crate::writer::write_str(&self.vm.engine, v))))
        })
    }

    fn binding_items(&self, v: Value) -> R<Vec<Value>> {
        self.vm.binding_items(v).map_err(|e| Decline(format!("{e}")))
    }
}

fn narrow(n: usize) -> R<u16> {
    u16::try_from(n).map_err(|_| Decline("too many variables for one frame".into()))
}
