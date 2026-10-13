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

use crate::heap::{Engine, NamedParam, Obj, Param};
use crate::value::{KwId, ObjRef, PrimId, SymId, Value};
use crate::writer::write_str;

#[derive(Clone, Debug, PartialEq)]
pub enum VmError {
    /// The host asked for a stop. Not a bug: a runaway script being killed is
    /// the feature (requirement 8).
    Interrupted,
    OutOfFuel,
    /// The host's heap cap was exceeded. An ordinary error at a safepoint, so
    /// a runaway scene is a failed evaluation and not a dead modeler.
    OutOfMemory,
    Error(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::Interrupted => write!(f, "interrupted"),
            VmError::OutOfFuel => write!(f, "out of fuel"),
            VmError::OutOfMemory => write!(f, "heap cap exceeded"),
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
    /// A missing argument's default expression is being evaluated.
    ///
    /// Defaults cannot be computed by the binder, because a default is an
    /// expression and may mention earlier parameters -- so they run on the
    /// explicit stack like everything else, which keeps them interruptible
    /// and keeps the collector able to see them.
    Defaults {
        todo: Rc<Vec<(SymId, Value)>>,
        idx: usize,
        body: Rc<Vec<Value>>,
        env: ObjRef,
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

impl Frame {
    /// Every `Value` and environment this frame keeps alive. Missing one here
    /// is the classic collector bug: it does not fail at the next collection,
    /// it fails later when the slot has been reused for something else. The
    /// stress mode exists to turn "later" into "immediately".
    fn trace(&self, out: &mut Vec<Value>) {
        let env = match self {
            Frame::If { then_e, else_e, env } => {
                out.push(*then_e);
                out.extend(else_e.iter().copied());
                env
            }
            Frame::Seq { body, env, .. } => {
                out.extend(body.iter().copied());
                env
            }
            Frame::Operator { args, env } => {
                out.extend(args.iter().copied());
                env
            }
            Frame::Operand { done, args, env, .. } => {
                out.extend(done.iter().copied());
                out.extend(args.iter().copied());
                env
            }
            Frame::Assign { env, .. } => env,
            Frame::Defaults {
                todo, body, env, ..
            } => {
                out.extend(todo.iter().map(|(_, e)| *e));
                out.extend(body.iter().copied());
                out.push(Value::Obj(*env));
                return;
            }
            Frame::VecElem { done, items, env, .. } => {
                out.extend(done.iter().copied());
                out.extend(items.iter().copied());
                env
            }
        };
        if let Some(r) = env {
            out.push(Value::Obj(*r));
        }
    }
}

enum Step {
    Eval(Value, Option<ObjRef>),
    Return(Value),
}

impl Step {
    fn trace(&self, out: &mut Vec<Value>) {
        match self {
            Step::Eval(v, env) => {
                out.push(*v);
                if let Some(r) = env {
                    out.push(Value::Obj(*r));
                }
            }
            Step::Return(v) => out.push(*v),
        }
    }
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
    /// The lambda-list markers (plan §4). Janet's names, kept because they are
    /// unambiguous and familiar; `&` is an R7RS special initial, so these are
    /// ordinary symbols and only `lambda` treats them specially.
    amp_opt: SymId,
    amp_named: SymId,
    amp_keys: SymId,
}

/// A parsed lambda list.
#[derive(Default)]
struct Formals {
    params: Vec<SymId>,
    opt: Vec<Param>,
    named: Vec<NamedParam>,
    keys: Option<SymId>,
    rest: Option<SymId>,
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
            amp_opt: engine.interner.symbol("&opt"),
            amp_named: engine.interner.symbol("&named"),
            amp_keys: engine.interner.symbol("&keys"),
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
        // The forms not yet evaluated live in a Rust local, which the
        // collector cannot see -- so they are rooted for the duration. This is
        // the first real customer for the handle table, and the exact bug the
        // embedding API (S7) exists to make unavailable to the host.
        let handles: Vec<_> = forms.iter().map(|&f| self.engine.root(f)).collect();
        let mut last = Value::Unspecified;
        let mut result = Ok(());
        for &form in &forms {
            match self.eval(form) {
                Ok(v) => last = v,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        for h in handles {
            self.engine.unroot(h);
        }
        result.map(|()| last)
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
                self.safepoint(&step)?;
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

    /// The one place the VM can be stopped, and therefore the one place it can
    /// be collected. `step` is handed in because the computation in flight is
    /// a root that lives in a Rust local and nothing else can see it -- the
    /// subtlest rooting bug available here, so it is passed explicitly rather
    /// than remembered.
    fn safepoint(&mut self, step: &Step) -> VResult<()> {
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
        if self.engine.should_collect() {
            self.collect(step);
        }
        if self.engine.over_heap_cap() {
            return Err(VmError::OutOfMemory);
        }
        Ok(())
    }

    /// Gather the roots and collect. The root set is exactly plan §3.3's: the
    /// explicit control stack, the global cells, the handle table (inside
    /// `Engine::collect`), and the step in flight.
    fn collect(&mut self, step: &Step) {
        let mut roots: Vec<Value> = Vec::with_capacity(self.stack.len() * 2 + 8);
        step.trace(&mut roots);
        for frame in &self.stack {
            frame.trace(&mut roots);
        }
        roots.extend(self.globals.iter().flatten().copied());
        self.engine.collect(&roots);
    }

    /// Collect now, for tests and for a host that wants to.
    pub fn collect_now(&mut self) -> crate::heap::GcStats {
        let mut roots: Vec<Value> = Vec::new();
        for frame in &self.stack {
            frame.trace(&mut roots);
        }
        roots.extend(self.globals.iter().flatten().copied());
        self.engine.collect(&roots)
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
        let f = self.parse_formals(formals)?;
        if body.is_empty() {
            return Err(self.err("a procedure body cannot be empty"));
        }
        Ok(self.engine.alloc(Obj::Closure {
            name,
            params: Rc::new(f.params),
            opt: Rc::new(f.opt),
            named: Rc::new(f.named),
            keys: f.keys,
            rest: f.rest,
            body: Rc::new(body.to_vec()),
            env,
        }))
    }

    /// Parse §4's lambda list:
    ///
    /// ```text
    /// formals := sym | ( req* [&opt spec*] [&named spec*] [&keys sym] [. sym] )
    /// spec    := sym | (sym default-expr)
    /// ```
    ///
    /// The order is fixed, as it is in Janet, and a marker out of order is an
    /// error rather than a parameter that happens to be spelled `&opt`.
    fn parse_formals(&mut self, formals: Value) -> VResult<Formals> {
        #[derive(PartialEq, Clone, Copy)]
        enum Section {
            Req,
            Opt,
            Named,
            Keys,
        }

        let mut f = Formals::default();
        let mut section = Section::Req;
        let mut cur = formals;
        loop {
            match cur {
                Value::Null => break,
                // `(a . rest)` and a bare `args`.
                Value::Symbol(s) => {
                    if f.keys.is_some() {
                        return Err(self.err("&keys and a dotted rest cannot both appear"));
                    }
                    f.rest = Some(s);
                    break;
                }
                Value::Obj(r) => {
                    let Obj::Pair(car, cdr) = *self.engine.obj(r) else {
                        return Err(self.err("malformed parameter list"));
                    };
                    cur = cdr;
                    // A marker?
                    if let Value::Symbol(s) = car {
                        let next = if s == self.syms.amp_opt {
                            Some(Section::Opt)
                        } else if s == self.syms.amp_named {
                            Some(Section::Named)
                        } else if s == self.syms.amp_keys {
                            Some(Section::Keys)
                        } else {
                            None
                        };
                        if let Some(next) = next {
                            let order = |x: Section| match x {
                                Section::Req => 0,
                                Section::Opt => 1,
                                Section::Named => 2,
                                Section::Keys => 3,
                            };
                            if order(next) <= order(section) {
                                let name = self.engine.interner.sym_name(s).to_string();
                                return Err(self.err(format!(
                                    "{name} is out of order; the lambda list goes \
                                     required, &opt, &named, &keys"
                                )));
                            }
                            section = next;
                            continue;
                        }
                    }
                    match section {
                        Section::Req => match car {
                            Value::Symbol(s) => f.params.push(s),
                            // `(define (f a [b 10]) …)` is Racket's spelling
                            // for an optional with a default, and a required
                            // parameter cannot carry one -- so this is
                            // unambiguous. Accepted, normalised to `&opt`,
                            // never printed (plan §4.9).
                            _ => {
                                f.opt.push(self.parse_param_spec(car)?);
                                section = Section::Opt;
                            }
                        },
                        Section::Opt => f.opt.push(self.parse_param_spec(car)?),
                        Section::Named => {
                            let p = self.parse_param_spec(car)?;
                            let name = self.engine.interner.sym_name(p.name).to_string();
                            let kw = self.engine.interner.keyword(&name);
                            f.named.push(NamedParam {
                                name: p.name,
                                kw,
                                default: p.default,
                            });
                        }
                        Section::Keys => match car {
                            Value::Symbol(s) if f.keys.is_none() => f.keys = Some(s),
                            Value::Symbol(_) => {
                                return Err(self.err("&keys takes exactly one name"));
                            }
                            _ => return Err(self.err("&keys takes a symbol")),
                        },
                    }
                }
                _ => return Err(self.err("malformed parameter list")),
            }
        }
        if f.rest.is_some() && f.keys.is_some() {
            return Err(self.err("&keys and a dotted rest cannot both appear"));
        }
        Ok(f)
    }

    /// `name`, `(name default-expr)`, or Racket's `[name default-expr]` --
    /// which our reader makes a two-element vector, so it is handled here
    /// rather than in the reader (plan §4.9).
    fn parse_param_spec(&mut self, spec: Value) -> VResult<Param> {
        match spec {
            Value::Symbol(s) => Ok(Param {
                name: s,
                default: None,
            }),
            Value::Obj(r) if matches!(self.engine.obj(r), Obj::Vector { .. }) => {
                let Obj::Vector { items, .. } = self.engine.obj(r) else {
                    unreachable!("guarded")
                };
                match items.as_slice() {
                    [Value::Symbol(name), default] => Ok(Param {
                        name: *name,
                        default: Some(*default),
                    }),
                    [Value::Symbol(name)] => Ok(Param {
                        name: *name,
                        default: None,
                    }),
                    _ => Err(self.err("expected [NAME default]")),
                }
            }
            Value::Obj(r) => {
                let Obj::Pair(Value::Symbol(name), rest) = *self.engine.obj(r) else {
                    return Err(self.err("expected NAME or (NAME default)"));
                };
                let Value::Obj(r2) = rest else {
                    return Err(self.err("expected (NAME default)"));
                };
                let Obj::Pair(default, Value::Null) = *self.engine.obj(r2) else {
                    return Err(self.err("(NAME default) takes exactly one default"));
                };
                Ok(Param {
                    name,
                    default: Some(default),
                })
            }
            _ => Err(self.err("expected NAME or (NAME default)")),
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
            Frame::Defaults {
                todo,
                idx,
                body,
                env,
            } => {
                // Write the computed default into its slot, so a later
                // default can see it -- defaults are evaluated left to right
                // and may mention earlier parameters.
                let sym = todo[idx].0;
                if let Obj::Env { slots, .. } = self.engine.obj_mut(env)
                    && let Some(slot) = slots.iter_mut().rev().find(|(s, _)| *s == sym)
                {
                    slot.1 = v;
                }
                let next = idx + 1;
                if next < todo.len() {
                    let expr = todo[next].1;
                    self.push(Frame::Defaults {
                        todo,
                        idx: next,
                        body,
                        env,
                    });
                    return Ok(Step::Eval(expr, Some(env)));
                }
                Ok(self.enter_body(body, Some(env)))
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
                    opt,
                    named,
                    keys,
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

                // Plan §4's rule, in the form the cases actually require. A
                // keyword begins the named section if
                //
                //   (a) it names a declared `&named` parameter, or
                //   (b) `&keys` is declared -- declaring it *is* saying "I
                //       take keyword arguments", or
                //   (c) the procedure declares some `&named` parameter and
                //       every positional slot is already filled.
                //
                // (a) and (b) apply while positionals are still open; (c) only
                // after. That is what separates the two cases that matter:
                // `(set-tone-map :aces)` declares no `&named` at all, so
                // `:aces` is the value it looks like, while
                // `(sphere :radiuss 2)` against `&named radius` has no
                // positional slots left and so gets told its keyword is
                // unknown -- which is the typo Janet accepts in silence.
                let starts_named_while_filling = |v: Value| match v {
                    Value::Keyword(k) => named.iter().any(|p| p.kw == k) || keys.is_some(),
                    _ => false,
                };
                let starts_named_after_filling = |v: Value| match v {
                    Value::Keyword(_) => !named.is_empty() || keys.is_some(),
                    _ => false,
                };

                let mut slots: Vec<(SymId, Value)> =
                    Vec::with_capacity(params.len() + opt.len() + named.len() + 2);
                let mut todo: Vec<(SymId, Value)> = Vec::new();
                let mut i = 0usize;

                // Required positionals.
                for &p in params.iter() {
                    if i >= args.len() || starts_named_while_filling(args[i]) {
                        return Err(self.err(format!(
                            "{who} expects {} required argument{}, got {i}",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" }
                        )));
                    }
                    slots.push((p, args[i]));
                    i += 1;
                }

                // Optional positionals, in order.
                for p in opt.iter() {
                    if i < args.len() && !starts_named_while_filling(args[i]) {
                        slots.push((p.name, args[i]));
                        i += 1;
                    } else {
                        // Absent. A declared default is an expression and runs
                        // later; otherwise the binding is `#f`, which keeps
                        // Janet's falsy-nil ergonomics so an `(if material …)`
                        // supplied-check translates directly.
                        slots.push((p.name, Value::Bool(false)));
                        if let Some(d) = p.default {
                            todo.push((p.name, d));
                        }
                    }
                }

                // Leftover positionals go to a dotted rest, if there is one.
                let mut rest_items: Vec<Value> = Vec::new();
                while i < args.len() && !starts_named_after_filling(args[i]) {
                    if rest.is_none() {
                        return Err(self.err(format!(
                            "{who} got {} argument{}, which is more than it takes",
                            args.len(),
                            if args.len() == 1 { "" } else { "s" }
                        )));
                    }
                    rest_items.push(args[i]);
                    i += 1;
                }
                if let Some(rest_sym) = rest {
                    let tail = self.engine.list(&rest_items);
                    slots.push((rest_sym, tail));
                }

                // The named section: keyword/value pairs to the end.
                let mut supplied: Vec<(KwId, Value)> = Vec::new();
                while i < args.len() {
                    let Value::Keyword(k) = args[i] else {
                        return Err(self.err(format!(
                            "{who}: expected a keyword, got {}",
                            write_str(&self.engine, args[i])
                        )));
                    };
                    let known = named.iter().any(|p| p.kw == k);
                    if !known && keys.is_none() {
                        // Janet silently ignores an unknown named argument, so
                        // a misspelled `:radiuss` does nothing and says
                        // nothing. This is the cheapest place to catch it.
                        let kw = self.engine.interner.kw_name(k).to_string();
                        return Err(self.err(format!("{who}: unknown named argument :{kw}")));
                    }
                    if i + 1 >= args.len() {
                        let kw = self.engine.interner.kw_name(k).to_string();
                        return Err(self.err(format!("{who}: named argument :{kw} has no value")));
                    }
                    supplied.push((k, args[i + 1]));
                    i += 2;
                }

                for p in named.iter() {
                    match supplied.iter().rev().find(|(k, _)| *k == p.kw) {
                        Some((_, v)) => slots.push((p.name, *v)),
                        None => {
                            slots.push((p.name, Value::Bool(false)));
                            if let Some(d) = p.default {
                                todo.push((p.name, d));
                            }
                        }
                    }
                }

                // `&keys` gets every keyword argument, as an alist.
                if let Some(keys_sym) = keys {
                    let pairs: Vec<Value> = supplied
                        .iter()
                        .map(|(k, v)| self.engine.cons(Value::Keyword(*k), *v))
                        .collect();
                    let bag = self.engine.list(&pairs);
                    slots.push((keys_sym, bag));
                }

                let new_env = self.engine.env(env, slots);
                if todo.is_empty() {
                    // The common case allocates and evaluates nothing extra.
                    return Ok(self.enter_body(body, Some(new_env)));
                }
                let todo = Rc::new(todo);
                let first = todo[0].1;
                self.push(Frame::Defaults {
                    todo,
                    idx: 0,
                    body,
                    env: new_env,
                });
                Ok(Step::Eval(first, Some(new_env)))
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
