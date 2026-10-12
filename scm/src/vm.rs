//! T0: the interpreter, over an explicit control stack.
//!
//! This is the spine from plan §5, and the shape of it is the point. There is
//! **no Rust recursion in `eval`**. Every pending computation is a `Frame` in
//! `Vm::stack`, which buys four things that would each be expensive to
//! retrofit:
//!
//!   * **Proper tail calls**, because a call in tail position has nothing
//!     pushed behind it -- so the stack is exactly as deep after as before.
//!     This is not an optimization pass, it is the absence of a push.
//!   * **A precise GC root set** (S3): the roots are this stack, the
//!     environments it names, and the globals. Nothing lives in a Rust local
//!     across a step.
//!   * **Interrupt, timeout and fuel** (requirement 8), checked at a
//!     safepoint in the loop -- which is also where collection will run.
//!   * **No stack overflow.** Deep *non-tail* recursion grows a `Vec`, not
//!     the C stack. A Rust stack overflow aborts the process, and requirement
//!     8 says the host must survive anything the script does.
//!
//! T0 stays forever as the semantic oracle every later tier is differentially
//! tested against, so it is written for clarity over speed.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::heap::{Engine, Obj};
use crate::value::{ObjRef, PrimId, SymId, Value};
use crate::writer::write_str;

#[derive(Clone, Debug, PartialEq)]
pub enum VmError {
    /// The host asked for a stop. Not a bug: a runaway script being killed is
    /// the feature (requirement 8).
    Interrupted,
    OutOfFuel,
    Error(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::Interrupted => write!(f, "interrupted"),
            VmError::OutOfFuel => write!(f, "out of fuel"),
            VmError::Error(m) => write!(f, "{m}"),
        }
    }
}

type VResult<T> = Result<T, VmError>;

/// A primitive's Rust side. It gets the engine (to allocate) and its
/// arguments, and cannot touch the control stack -- a primitive that needed to
/// would have to become a special form.
pub type PrimFn = fn(&mut Engine, &[Value]) -> VResult<Value>;

/// One pending computation.
#[derive(Clone, Debug)]
enum Frame {
    /// The test of an `if` is being evaluated; these are the branches.
    If {
        then_e: Value,
        else_e: Option<Value>,
        env: Option<ObjRef>,
    },
    /// A body with more expressions after the one being evaluated. `idx`
    /// indexes `body`, so nothing is cloned to walk a body.
    Seq {
        body: Rc<Vec<Value>>,
        idx: usize,
        env: Option<ObjRef>,
    },
    /// The operator of a call is being evaluated; `args` are still unevaluated.
    Operator {
        args: Vec<Value>,
        env: Option<ObjRef>,
    },
    /// An argument is being evaluated. `done` holds the callee followed by the
    /// arguments already evaluated.
    Operand {
        done: Vec<Value>,
        args: Vec<Value>,
        idx: usize,
        env: Option<ObjRef>,
    },
    /// The value of a `set!` or a `define` is being evaluated.
    Assign {
        sym: SymId,
        define: bool,
        env: Option<ObjRef>,
    },
    /// An element of a `[…]` vector is being evaluated. This is the whole
    /// reason the reader keeps a `bracket` flag: a bracket vector *evaluates*
    /// its elements, where `#(…)` is a constant (plan §4.5).
    VecElem {
        done: Vec<Value>,
        items: Rc<Vec<Value>>,
        idx: usize,
        env: Option<ObjRef>,
    },
}

enum Step {
    Eval(Value, Option<ObjRef>),
    Return(Value),
}

pub struct Vm {
    pub engine: Engine,
    /// Globals are **cells, never inlined values** (plan §1's (6)-vs-(7)
    /// resolution). Indexed by `SymId`, so a redefinition at the REPL is a
    /// store into a slot every caller already reads through.
    globals: Vec<Option<Value>>,
    prims: Vec<PrimFn>,
    stack: Vec<Frame>,
    /// The deepest the stack has been this run. A tail loop must leave this
    /// flat, which is S2's exit criterion, so it is measured rather than
    /// asserted by inspection.
    high_water: usize,
    fuel: Option<u64>,
    interrupt: Arc<AtomicBool>,
    /// Cached symbols for the special forms, so the hot path compares `SymId`s
    /// rather than strings.
    syms: SpecialForms,
}

struct SpecialForms {
    quote: SymId,
    if_: SymId,
    define: SymId,
    set: SymId,
    lambda: SymId,
    begin: SymId,
}

impl Vm {
    pub fn new() -> Self {
        let mut engine = Engine::new();
        let syms = SpecialForms {
            quote: engine.interner.symbol("quote"),
            if_: engine.interner.symbol("if"),
            define: engine.interner.symbol("define"),
            set: engine.interner.symbol("set!"),
            lambda: engine.interner.symbol("lambda"),
            begin: engine.interner.symbol("begin"),
        };
        let mut vm = Vm {
            engine,
            globals: Vec::new(),
            prims: Vec::new(),
            stack: Vec::new(),
            high_water: 0,
            fuel: None,
            interrupt: Arc::new(AtomicBool::new(false)),
            syms,
        };
        crate::prim::install(&mut vm);
        vm
    }

    // ---- the host's controls (requirement 8) ----------------------------

    /// A handle the host can set from another thread to stop a runaway
    /// evaluation. The VM notices at its next safepoint.
    pub fn interrupt_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.interrupt)
    }

    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.fuel = fuel;
    }

    pub fn stack_high_water(&self) -> usize {
        self.high_water
    }

    pub fn define_prim(&mut self, name: &str, f: PrimFn) {
        let id = self.engine.register_prim_name(name);
        debug_assert_eq!(id as usize, self.prims.len());
        self.prims.push(f);
        let sym = self.engine.interner.symbol(name);
        self.set_global(sym, Value::Primitive(PrimId(id)));
    }

    pub fn global(&self, sym: SymId) -> Option<Value> {
        self.globals.get(sym.0 as usize).copied().flatten()
    }

    pub fn set_global(&mut self, sym: SymId, v: Value) {
        let i = sym.0 as usize;
        if self.globals.len() <= i {
            self.globals.resize(i + 1, None);
        }
        self.globals[i] = Some(v);
    }

    // ---- evaluation -----------------------------------------------------

    /// Evaluate every datum in `src`, returning the last value.
    pub fn eval_str(&mut self, src: &str) -> VResult<Value> {
        let forms = crate::reader::Reader::read_all(src, &mut self.engine)
            .map_err(|e| VmError::Error(format!("read error: {e}")))?;
        let mut last = Value::Unspecified;
        for form in forms {
            last = self.eval(form)?;
        }
        Ok(last)
    }

    pub fn eval(&mut self, expr: Value) -> VResult<Value> {
        let base = self.stack.len();
        let mut step = Step::Eval(expr, None);
        let mut ticks: u32 = 0;
        loop {
            // The safepoint. Collection will live here too (S3), which is why
            // it is one place rather than scattered through the match.
            ticks += 1;
            if ticks >= 1024 {
                ticks = 0;
                self.safepoint()?;
            }

            step = match step {
                Step::Eval(expr, env) => self.eval_step(expr, env)?,
                Step::Return(v) => {
                    if self.stack.len() <= base {
                        return Ok(v);
                    }
                    let frame = self.stack.pop().expect("stack.len() > base");
                    self.resume(frame, v)?
                }
            };
        }
    }

    fn safepoint(&mut self) -> VResult<()> {
        if self.interrupt.load(Ordering::Relaxed) {
            self.interrupt.store(false, Ordering::Relaxed);
            return Err(VmError::Interrupted);
        }
        if let Some(fuel) = self.fuel.as_mut() {
            if *fuel == 0 {
                return Err(VmError::OutOfFuel);
            }
            *fuel = fuel.saturating_sub(1);
        }
        Ok(())
    }

    fn push(&mut self, frame: Frame) {
        self.stack.push(frame);
        if self.stack.len() > self.high_water {
            self.high_water = self.stack.len();
        }
    }

    fn eval_step(&mut self, expr: Value, env: Option<ObjRef>) -> VResult<Step> {
        match expr {
            // Self-evaluating.
            Value::Fixnum(_)
            | Value::Flonum(_)
            | Value::Bool(_)
            | Value::Char(_)
            | Value::Keyword(_)
            | Value::Primitive(_)
            | Value::Unspecified
            | Value::Eof => Ok(Step::Return(expr)),

            Value::Symbol(sym) => match self.lookup(sym, env) {
                Some(v) => Ok(Step::Return(v)),
                None => Err(self.err(format!(
                    "unbound variable: {}",
                    self.engine.interner.sym_name(sym)
                ))),
            },

            // `()` is not an expression in R7RS.
            Value::Null => Err(self.err("empty application: ()")),

            Value::Obj(r) => match self.engine.obj(r).clone() {
                // Strings, bytevectors and `#(…)` vectors are constants.
                Obj::Str(_) | Obj::Bytevector(_) => Ok(Step::Return(expr)),
                Obj::Vector { items, bracket } => {
                    if !bracket {
                        return Ok(Step::Return(expr));
                    }
                    // `[a b c]` is a constructor: evaluate the elements.
                    if items.is_empty() {
                        let v = self.engine.vector(Vec::new(), true);
                        return Ok(Step::Return(v));
                    }
                    let items = Rc::new(items);
                    let first = items[0];
                    self.push(Frame::VecElem {
                        done: Vec::with_capacity(items.len()),
                        items,
                        idx: 0,
                        env,
                    });
                    Ok(Step::Eval(first, env))
                }
                Obj::Closure { .. } | Obj::Env { .. } => Ok(Step::Return(expr)),
                Obj::Pair(car, cdr) => {
                    let args = self.list_to_vec(cdr)?;
                    if let Value::Symbol(head) = car
                        && self.lookup(head, env).is_none()
                        && let Some(step) = self.special_form(head, &args, env)?
                    {
                        // A special form, and only because nothing shadows its
                        // name: a local called `if` is a variable, not syntax.
                        return Ok(step);
                    }
                    self.push(Frame::Operator { args, env });
                    Ok(Step::Eval(car, env))
                }
            },
        }
    }

    /// `Ok(None)` means "not a special form after all, treat it as a call".
    fn special_form(
        &mut self,
        head: SymId,
        args: &[Value],
        env: Option<ObjRef>,
    ) -> VResult<Option<Step>> {
        let s = &self.syms;
        if head == s.quote {
            return match args {
                [v] => Ok(Some(Step::Return(*v))),
                _ => Err(self.err("quote takes exactly one datum")),
            };
        }
        if head == s.if_ {
            return match args {
                [test, then_e] | [test, then_e, _] => {
                    let else_e = args.get(2).copied();
                    let (test, then_e) = (*test, *then_e);
                    self.push(Frame::If {
                        then_e,
                        else_e,
                        env,
                    });
                    Ok(Some(Step::Eval(test, env)))
                }
                _ => Err(self.err("if takes a test, a consequent and an optional alternative")),
            };
        }
        if head == s.begin {
            if args.is_empty() {
                return Ok(Some(Step::Return(Value::Unspecified)));
            }
            let body = Rc::new(args.to_vec());
            return Ok(Some(self.enter_body(body, env)));
        }
        if head == s.lambda {
            return match args {
                [] => Err(self.err("lambda needs a parameter list")),
                [formals, body @ ..] => {
                    let v = self.make_closure(None, *formals, body, env)?;
                    Ok(Some(Step::Return(v)))
                }
            };
        }
        if head == s.set || head == s.define {
            let define = head == s.define;
            return match args {
                // `(define (f a) …)` -- the procedure shorthand.
                [Value::Obj(r), body @ ..] if define => {
                    let Obj::Pair(name, formals) = *self.engine.obj(*r) else {
                        return Err(self.err("define: expected a name or (name . formals)"));
                    };
                    let Value::Symbol(name_sym) = name else {
                        return Err(self.err("define: procedure name must be a symbol"));
                    };
                    let f = self.make_closure(Some(name_sym), formals, body, env)?;
                    self.bind(name_sym, f, define, env)?;
                    Ok(Some(Step::Return(Value::Unspecified)))
                }
                [Value::Symbol(sym)] if define => {
                    self.bind(*sym, Value::Unspecified, define, env)?;
                    Ok(Some(Step::Return(Value::Unspecified)))
                }
                [Value::Symbol(sym), value] => {
                    let (sym, value) = (*sym, *value);
                    self.push(Frame::Assign { sym, define, env });
                    Ok(Some(Step::Eval(value, env)))
                }
                _ => Err(self.err(if define {
                    "define: expected (define name value) or (define (name . formals) body …)"
                } else {
                    "set!: expected (set! name value)"
                })),
            };
        }
        Ok(None)
    }

    /// Begin evaluating a body, pushing a `Seq` frame only if there is more
    /// than one expression -- the last one is in tail position and must have
    /// nothing behind it.
    fn enter_body(&mut self, body: Rc<Vec<Value>>, env: Option<ObjRef>) -> Step {
        let first = body[0];
        if body.len() > 1 {
            self.push(Frame::Seq { body, idx: 0, env });
        }
        Step::Eval(first, env)
    }

    fn make_closure(
        &mut self,
        name: Option<SymId>,
        formals: Value,
        body: &[Value],
        env: Option<ObjRef>,
    ) -> VResult<Value> {
        let (params, rest) = self.parse_formals(formals)?;
        if body.is_empty() {
            return Err(self.err("a procedure body cannot be empty"));
        }
        Ok(self.engine.alloc(Obj::Closure {
            name,
            params: Rc::new(params),
            rest,
            body: Rc::new(body.to_vec()),
            env,
        }))
    }

    /// Required parameters plus an optional rest, which is as much of §4's
    /// lambda list as S2 needs. `&opt` and `&named` are argument *binding*
    /// and land with the verb bridge; they change no part of the machine.
    fn parse_formals(&mut self, formals: Value) -> VResult<(Vec<SymId>, Option<SymId>)> {
        let mut params = Vec::new();
        let mut cur = formals;
        loop {
            match cur {
                Value::Null => return Ok((params, None)),
                Value::Symbol(s) => return Ok((params, Some(s))),
                Value::Obj(r) => match *self.engine.obj(r) {
                    Obj::Pair(Value::Symbol(s), cdr) => {
                        params.push(s);
                        cur = cdr;
                    }
                    Obj::Pair(..) => return Err(self.err("parameter names must be symbols")),
                    _ => return Err(self.err("malformed parameter list")),
                },
                _ => return Err(self.err("malformed parameter list")),
            }
        }
    }

    fn resume(&mut self, frame: Frame, v: Value) -> VResult<Step> {
        match frame {
            Frame::If {
                then_e,
                else_e,
                env,
            } => {
                if v.is_truthy() {
                    Ok(Step::Eval(then_e, env))
                } else {
                    match else_e {
                        Some(e) => Ok(Step::Eval(e, env)),
                        None => Ok(Step::Return(Value::Unspecified)),
                    }
                }
            }
            Frame::Seq { body, idx, env } => {
                let next = idx + 1;
                let expr = body[next];
                if next + 1 < body.len() {
                    self.push(Frame::Seq {
                        body,
                        idx: next,
                        env,
                    });
                }
                Ok(Step::Eval(expr, env))
            }
            Frame::Operator { args, env } => {
                if args.is_empty() {
                    return self.apply(v, Vec::new());
                }
                let first = args[0];
                self.push(Frame::Operand {
                    done: vec![v],
                    args,
                    idx: 0,
                    env,
                });
                Ok(Step::Eval(first, env))
            }
            Frame::Operand {
                mut done,
                args,
                idx,
                env,
            } => {
                done.push(v);
                let next = idx + 1;
                if next < args.len() {
                    let expr = args[next];
                    self.push(Frame::Operand {
                        done,
                        args,
                        idx: next,
                        env,
                    });
                    return Ok(Step::Eval(expr, env));
                }
                // Every argument is evaluated and this frame is already
                // popped, so a call in tail position leaves the stack exactly
                // as deep as it found it. That is the whole of proper tail
                // calls.
                let callee = done[0];
                let argv = done.split_off(1);
                self.apply(callee, argv)
            }
            Frame::Assign { sym, define, env } => {
                self.bind(sym, v, define, env)?;
                Ok(Step::Return(Value::Unspecified))
            }
            Frame::VecElem {
                mut done,
                items,
                idx,
                env,
            } => {
                done.push(v);
                let next = idx + 1;
                if next < items.len() {
                    let expr = items[next];
                    self.push(Frame::VecElem {
                        done,
                        items,
                        idx: next,
                        env,
                    });
                    return Ok(Step::Eval(expr, env));
                }
                let out = self.engine.vector(done, true);
                Ok(Step::Return(out))
            }
        }
    }

    fn apply(&mut self, callee: Value, args: Vec<Value>) -> VResult<Step> {
        match callee {
            Value::Primitive(id) => {
                let f = self.prims[id.0 as usize];
                let v = f(&mut self.engine, &args)?;
                Ok(Step::Return(v))
            }
            Value::Obj(r) => {
                let Obj::Closure {
                    params,
                    rest,
                    body,
                    env,
                    name,
                } = self.engine.obj(r).clone()
                else {
                    return Err(self.err(format!(
                        "not applicable: {}",
                        write_str(&self.engine, callee)
                    )));
                };
                let who = match name {
                    Some(n) => self.engine.interner.sym_name(n).to_string(),
                    None => "#<procedure>".to_string(),
                };
                if args.len() < params.len() || (rest.is_none() && args.len() > params.len()) {
                    return Err(self.err(format!(
                        "{who} expects {}{} argument{}, got {}",
                        params.len(),
                        if rest.is_some() { " or more" } else { "" },
                        if params.len() == 1 { "" } else { "s" },
                        args.len()
                    )));
                }
                let mut slots: Vec<(SymId, Value)> = Vec::with_capacity(params.len() + 1);
                for (i, &p) in params.iter().enumerate() {
                    slots.push((p, args[i]));
                }
                if let Some(rest_sym) = rest {
                    let tail = self.engine.list(&args[params.len()..]);
                    slots.push((rest_sym, tail));
                }
                let new_env = self.engine.env(env, slots);
                Ok(self.enter_body(body, Some(new_env)))
            }
            other => Err(self.err(format!(
                "not applicable: {}",
                write_str(&self.engine, other)
            ))),
        }
    }

    // ---- environments ---------------------------------------------------

    fn lookup(&self, sym: SymId, env: Option<ObjRef>) -> Option<Value> {
        let mut cur = env;
        while let Some(r) = cur {
            let Obj::Env { parent, slots } = self.engine.obj(r) else {
                return None;
            };
            for &(s, v) in slots.iter().rev() {
                if s == sym {
                    return Some(v);
                }
            }
            cur = *parent;
        }
        self.global(sym)
    }

    /// `set!` must find an existing binding; `define` creates one, in the
    /// innermost environment or in the globals at top level.
    fn bind(&mut self, sym: SymId, v: Value, define: bool, env: Option<ObjRef>) -> VResult<()> {
        let mut cur = env;
        while let Some(r) = cur {
            let parent = match self.engine.obj_mut(r) {
                Obj::Env { parent, slots } => {
                    if let Some(slot) = slots.iter_mut().rev().find(|(s, _)| *s == sym) {
                        slot.1 = v;
                        return Ok(());
                    }
                    *parent
                }
                _ => return Err(self.err("corrupt environment")),
            };
            if define && cur == env {
                // An internal define adds to the innermost frame. R7RS makes
                // internal defines `letrec*`; S5 can tighten this when the
                // library system arrives.
                if let Obj::Env { slots, .. } = self.engine.obj_mut(r) {
                    slots.push((sym, v));
                    return Ok(());
                }
            }
            cur = parent;
        }
        // At top level a `define` creates the global and a `set!` requires
        // one to exist already -- which is the whole difference between them.
        if !define && self.global(sym).is_none() {
            return Err(self.err(format!(
                "set!: unbound variable: {}",
                self.engine.interner.sym_name(sym)
            )));
        }
        self.set_global(sym, v);
        Ok(())
    }

    // ---- helpers --------------------------------------------------------

    fn list_to_vec(&self, mut v: Value) -> VResult<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            match v {
                Value::Null => return Ok(out),
                Value::Obj(r) => match self.engine.obj(r) {
                    Obj::Pair(car, cdr) => {
                        out.push(*car);
                        v = *cdr;
                    }
                    _ => return Err(VmError::Error("improper list in source".into())),
                },
                _ => return Err(VmError::Error("improper list in source".into())),
            }
        }
    }

    /// Errors unwind the explicit stack, which is just truncating it. A later
    /// stage turns this into a condition with the frames still live (§8).
    fn err(&mut self, msg: impl Into<String>) -> VmError {
        VmError::Error(msg.into())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
