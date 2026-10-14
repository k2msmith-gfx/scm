//! The embedding API — plan §7a, stage S7.
//!
//! Requirement 2 is "easy embedding in a Rust application as a crate", and
//! until this module the embedding API was *every module being `pub`*. That is
//! not an API, it is the absence of one, and §7a records the three
//! consequences that were measured before this was designed. The sharpest:
//!
//! ```text
//! held:            (1 2 3)
//! after a GC:      #<environment>
//! ```
//!
//! A host held the result of `eval_str`, ran more script, and the value it was
//! holding became a different object. No `unsafe`, no warning, no error.
//!
//! Everything here exists to make that un-writable:
//!
//! * [`Interp`] is the only type a host constructs.
//! * [`Rooted`] is a handle that keeps its value alive and un-moved, and
//!   releases it on `Drop`.
//! * [`Val`] is the host's view of a value, with no `ObjRef` in it.
//! * Every entry point returns [`Result`], and a caught panic **poisons** the
//!   interpreter rather than pretending the VM is still intelligible.

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::heap::{Handle, Obj};
use crate::value::Value;
use crate::vm::{Vm, VmError};

/// What a host gets instead of a panic or a bare string.
///
/// Not `Clone`, because [`Error::Raised`] owns a [`Rooted`] and two owners of
/// one handle would need reference counting to know when to release it. A
/// host that wants to keep a raised object past the error calls
/// [`Interp::keep`].
#[derive(Debug, PartialEq)]
pub enum Error {
    /// The script could not be read: unbalanced parens, a bad escape. Held
    /// apart from [`Error::Incomplete`] because an editor needs the
    /// difference (S1's whole reason for two answers).
    Syntax(String),
    /// The source stops mid-datum. More input would fix it.
    Incomplete,
    /// The script raised, or a primitive refused.
    Script(String),
    /// `(raise obj)` reached the host. The object is rooted, so a host can
    /// inspect it rather than only read its printed form.
    Raised { text: String, value: Rooted },
    /// The host said stop (`interrupt`), or a timeout did.
    Interrupted,
    /// Out of fuel. Requirement 8: a runaway script is an error, not a hang.
    OutOfFuel,
    /// The heap cap was reached.
    OutOfMemory,
    /// A [`Rooted`] from a different [`Interp`], or one already released.
    ForeignHandle,
    /// A panic was caught at the boundary. The interpreter is now poisoned.
    Panicked(String),
    /// A previous call panicked, so the VM's state is not known.
    Poisoned,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Syntax(m) => write!(f, "syntax error: {m}"),
            Error::Incomplete => write!(f, "incomplete input"),
            Error::Script(m) => write!(f, "{m}"),
            Error::Raised { text, .. } => write!(f, "{text}"),
            Error::Interrupted => write!(f, "interrupted"),
            Error::OutOfFuel => write!(f, "out of fuel"),
            Error::OutOfMemory => write!(f, "heap cap exceeded"),
            Error::ForeignHandle => {
                write!(f, "that handle belongs to another interpreter, or was released")
            }
            Error::Panicked(m) => write!(f, "the interpreter panicked: {m}"),
            Error::Poisoned => write!(
                f,
                "this interpreter is poisoned by an earlier panic and cannot be used"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Handles whose `Rooted` has been dropped, waiting to be unrooted.
///
/// `Drop` cannot reach the engine -- a `Rooted` would have to borrow the
/// `Interp`, and then a host could only hold one at a time. So a dropped
/// handle is queued here and the `Interp` drains the queue at every entry
/// point. Deferred unrooting is conservative in the only direction that is
/// safe: a value lives slightly too long, never too briefly.
type Pending = Rc<RefCell<Vec<Handle>>>;

/// A value the host is holding, kept alive while this exists.
///
/// Not `Clone`: two owners would need reference counting to know when to
/// unroot, and [`Interp::keep`] is the cheap way to get a second handle.
/// Not `Send`: the heap is single-threaded by design (plan §3.3).
#[derive(Debug)]
pub struct Rooted {
    handle: Handle,
    /// Which interpreter this belongs to. A handle is an index, so without
    /// this a handle from another `Interp` would silently name *its* value.
    owner: u64,
    pending: Pending,
}

impl Drop for Rooted {
    fn drop(&mut self) {
        self.pending.borrow_mut().push(self.handle);
    }
}

impl PartialEq for Rooted {
    /// Handle identity, which is not value identity: two handles on the same
    /// value are not `==`. Use [`Interp::eqv`] for the Scheme question.
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.handle == other.handle
    }
}

/// What a name is, for a REPL's `describe`.
#[derive(Debug, Clone, PartialEq)]
pub struct Description {
    pub name: String,
    /// How to call it, when it is callable.
    pub signature: Option<String>,
    /// Its docstring, when it was defined with one.
    pub doc: Option<String>,
    /// `"procedure"`, `"primitive"`, `"parameter"` or `"variable"`.
    pub kind: String,
    /// Its current value, printed.
    pub value: String,
}

/// A host's view of a Scheme value.
///
/// Deliberately not every case the heap has: a closure, a port and a
/// parameter have no useful host representation, so they stay behind a
/// [`Rooted`] as [`Val::Opaque`]. A host should never need to know what an
/// `ObjRef` is.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// `'()`. Note this is the empty list, not a nil: plan §4 is explicit
    /// that this language has no nil.
    Null,
    Unspecified,
    Bool(bool),
    /// An exact integer. No bignums (§2.3), so this is the whole range.
    Int(i64),
    Float(f64),
    Char(char),
    Str(String),
    Sym(String),
    /// A keyword, carried without its colon -- `Keyword("at")` is `:at`.
    /// First-class and distinct from a symbol, which is the §4 decision ray
    /// needs.
    Keyword(String),
    /// A proper list. An improper one comes back as [`Val::Opaque`] rather
    /// than quietly losing its tail.
    List(Vec<Val>),
    Vector(Vec<Val>),
    Bytes(Vec<u8>),
    /// A procedure, rooted while any clone of this exists. Call it with
    /// [`Host::call`] from inside a host function, or [`Interp::call`]
    /// from outside; store it and call it later, since it stays rooted.
    Proc(Proc),
    /// Something with no host representation: a port, a parameter, an
    /// improper list, or a structure deeper than the depth limit. Reach it
    /// with [`Rooted`] and Scheme.
    Opaque,
}

/// A Scheme procedure in the host's hands. Clones share one root, released
/// when the last clone is dropped.
#[derive(Clone, Debug)]
pub struct Proc(Rc<Rooted>);

impl PartialEq for Proc {
    fn eq(&self, other: &Self) -> bool {
        self.0.handle == other.0.handle
    }
}

impl Proc {
    pub fn rooted(&self) -> &Rooted {
        &self.0
    }
}

/// What a re-entrant host function gets besides its arguments: the machine
/// behind a small door. Nested evaluation leaves the caller's frames where
/// they are, so a host function that calls back into Scheme is ordinary.
pub struct Host<'a> {
    vm: &'a mut Vm,
    id: u64,
    pending: Pending,
}

impl Host<'_> {
    /// Apply a Scheme procedure to host values and wait for its result. A
    /// Scheme error comes back as the message, which the host function may
    /// pass on or handle.
    pub fn call(&mut self, f: &Proc, args: &[Val]) -> Result<Val, String> {
        let fv = self.vm.engine.handle_value(f.0.handle).ok_or("a dropped procedure")?;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(build_val(&mut self.vm.engine, a, 0).map_err(|e| format!("{e}"))?);
        }
        let out = self.vm.call_procedure(fv, vals).map_err(|e| format!("{e}"))?;
        Ok(read_val_rooting(&mut self.vm.engine, out, 0, self.id, &self.pending))
    }

    /// Evaluate source at top level.
    pub fn eval(&mut self, src: &str) -> Result<Val, String> {
        let out = self.vm.eval_str(src).map_err(|e| format!("{e}"))?;
        Ok(read_val_rooting(&mut self.vm.engine, out, 0, self.id, &self.pending))
    }
}

/// An embedded Scheme interpreter.
pub struct Interp {
    vm: Vm,
    id: u64,
    pending: Pending,
    poisoned: Option<String>,
}

/// How deep [`Interp::value`] will walk before giving up and saying
/// [`Val::Opaque`].
///
/// A host asking for a value as data must not be handed a stack overflow,
/// and a cyclic structure is constructible (`set-cdr!`), so the walk is
/// bounded rather than cycle-detecting: the bound is simpler and a host that
/// wants the whole thing can ask Scheme.
const MAX_DEPTH: usize = 128;

fn next_id() -> u64 {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Interp {
    /// A fresh interpreter with the standard environment loaded.
    pub fn new() -> Result<Self, Error> {
        // `Vm::new` panics if the prelude fails, which is a bug rather than a
        // host error -- but it must still not cross this boundary.
        let vm = guard(Vm::new)?;
        Ok(Interp {
            vm,
            id: next_id(),
            pending: Rc::new(RefCell::new(Vec::new())),
            poisoned: None,
        })
    }

    // ---- the host's controls (requirement 8) ----------------------------

    /// A flag the host can set from **another thread** to stop a runaway
    /// evaluation at its next safepoint.
    ///
    /// Deliberately not catchable by the script: `(guard (e (else …)) …)`
    /// must not be a way to ignore the host.
    /// The machine itself, for a host that installs a tier into it.
    pub fn vm_mut(&mut self) -> &mut crate::Vm {
        &mut self.vm
    }

    pub fn interrupt_handle(&self) -> Arc<AtomicBool> {
        self.vm.interrupt_handle()
    }

    /// Stop after `n` safepoints. `None` removes the limit.
    pub fn set_fuel(&mut self, n: Option<u64>) {
        self.vm.set_fuel(n);
    }

    /// Refuse to grow the heap past `n` objects.
    pub fn set_heap_cap(&mut self, n: Option<usize>) {
        self.vm.engine.set_heap_cap(n);
    }

    /// Whether an earlier call panicked. A poisoned interpreter refuses
    /// everything: catching a panic does not make the VM's state known again.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }

    // ---- evaluation -----------------------------------------------------

    /// Evaluate every datum in `src` and root the last value.
    pub fn eval(&mut self, src: &str) -> Result<Rooted, Error> {
        self.enter()?;
        let vm = &mut self.vm;
        // A panic here leaves the stack, the parameter bindings and the heap
        // in a state nothing has reasoned about, so it poisons rather than
        // being reported and shrugged off.
        let r = match guard(|| vm.eval_str(src)) {
            Ok(r) => r,
            Err(e) => {
                self.poisoned = Some(format!("{e}"));
                return Err(e);
            }
        };
        match r {
            Ok(v) => Ok(self.root(v)),
            Err(e) => Err(self.convert(e)),
        }
    }

    /// Evaluate, naming the input. A backtrace from this evaluation will say
    /// that name, so a host should pass a file path or something a user can
    /// recognise.
    pub fn eval_named(&mut self, src: &str, source: &str) -> Result<Rooted, Error> {
        self.enter()?;
        let vm = &mut self.vm;
        let r = match guard(|| vm.eval_str_named(src, source)) {
            Ok(r) => r,
            Err(e) => {
                self.poisoned = Some(format!("{e}"));
                return Err(e);
            }
        };
        match r {
            Ok(v) => Ok(self.root(v)),
            Err(e) => Err(self.convert(e)),
        }
    }

    /// How many source locations the heap is holding. For leak tests: the
    /// table is program-sized, and the collector prunes it.
    pub fn loc_count(&self) -> usize {
        self.vm.engine.loc_count()
    }

    /// The pending calls at the most recent failure, innermost first.
    ///
    /// Empty when the failure happened with nothing pending -- a read error,
    /// or a top-level atom. A tail call leaves no frame, so a tail-recursive
    /// loop contributes one entry however many times it went round.
    pub fn backtrace(&self) -> Vec<String> {
        self.vm.backtrace().iter().map(|f| f.to_string()).collect()
    }

    /// Evaluate with a wall-clock limit.
    ///
    /// Implemented over [`Interp::interrupt_handle`] and a watchdog thread,
    /// which is why it stops at a *safepoint* and not instantly: a primitive
    /// that blocks forever in Rust is outside what this can promise, and
    /// saying so is better than implying otherwise.
    pub fn eval_with_timeout(&mut self, src: &str, limit: Duration) -> Result<Rooted, Error> {
        let flag = self.interrupt_handle();
        let done = Arc::new(AtomicBool::new(false));
        let watchdog = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let step = Duration::from_millis(1);
                let mut waited = Duration::ZERO;
                while waited < limit {
                    if done.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(step);
                    waited += step;
                }
                if !done.load(Ordering::Relaxed) {
                    flag.store(true, Ordering::Relaxed);
                }
            })
        };
        let out = self.eval(src);
        done.store(true, Ordering::Relaxed);
        let _ = watchdog.join();
        out
    }

    // ---- host functions -------------------------------------------------

    /// Register a Rust function the script can call, which **may capture
    /// state** -- the point, since a ray verb needs the scene store.
    ///
    /// The function is handed [`Val`]s and returns one. That is a safety
    /// property, not a limitation to work around: a host function never sees
    /// a heap reference, so it cannot hold one across a collection, and the
    /// hazard §7a opens with is unreachable from here. A procedure or a port
    /// passed to it arrives as [`Val::Opaque`].
    ///
    /// `Err(String)` becomes an ordinary Scheme error, catchable by `guard`,
    /// because the host's *code* failing is not the host saying stop. A
    /// **panic** inside the function is caught and becomes a Scheme error
    /// too, naming the function.
    pub fn define_fn(
        &mut self,
        name: &str,
        f: impl FnMut(&[Val]) -> Result<Val, String> + 'static,
    ) -> Result<(), Error> {
        self.enter()?;
        let vm = &mut self.vm;
        match guard(|| vm.define_host_fn(name, f)) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.poisoned = Some(format!("{e}"));
                Err(e)
            }
        }
    }

    /// Register a host function that can call back into Scheme: it gets a
    /// [`Host`] and its arguments, with procedures among them as
    /// [`Val::Proc`]. A panic inside is caught, the machine's stacks are put
    /// back, and the call fails as an ordinary Scheme error.
    ///
    /// `Fn`, so it may re-enter itself through a callback; keep its state in
    /// a `RefCell` or `Cell`.
    pub fn define_fn_with(
        &mut self,
        name: &str,
        f: impl Fn(&mut Host, &[Val]) -> Result<Val, String> + 'static,
    ) -> Result<(), Error> {
        self.enter()?;
        let (id, pending, who) = (self.id, Rc::clone(&self.pending), name.to_string());
        let closure: crate::vm::HostClosure = Rc::new(move |vm: &mut Vm, args: &[Value]| {
            let vals: Vec<Val> = args.iter().map(|&a| read_val_rooting(&mut vm.engine, a, 0, id, &pending)).collect();
            let mut host = Host { vm, id, pending: Rc::clone(&pending) };
            let out = f(&mut host, &vals);
            match out {
                Err(msg) => Err(vm.err(format!("{who}: {msg}"))),
                Ok(v) => build_val(&mut vm.engine, &v, 0).map_err(|e| vm.err(format!("{who}: {e}"))),
            }
        });
        let vm = &mut self.vm;
        match guard(|| vm.define_host_closure(name, closure)) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.poisoned = Some(format!("{e}"));
                Err(e)
            }
        }
    }

    /// Apply a procedure the host holds -- a callback a script registered
    /// earlier, say -- and get its result.
    pub fn call(&mut self, f: &Proc, args: &[Val]) -> Result<Val, Error> {
        self.enter()?;
        if f.0.owner != self.id {
            return Err(Error::ForeignHandle);
        }
        let fv = self.vm.engine.handle_value(f.0.handle).ok_or(Error::ForeignHandle)?;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.build(a, 0)?);
        }
        let (id, pending) = (self.id, Rc::clone(&self.pending));
        let vm = &mut self.vm;
        match guard(|| vm.call_procedure(fv, vals)) {
            Ok(Ok(v)) => Ok(read_val_rooting(&mut self.vm.engine, v, 0, id, &pending)),
            Ok(Err(e)) => Err(self.convert(e)),
            Err(e) => {
                self.poisoned = Some(format!("{e}"));
                Err(e)
            }
        }
    }

    // ---- values ---------------------------------------------------------

    /// Root a value so the host can hold it.
    fn root(&mut self, v: Value) -> Rooted {
        Rooted {
            handle: self.vm.engine.root(v),
            owner: self.id,
            pending: Rc::clone(&self.pending),
        }
    }

    /// A second handle on the same value.
    pub fn keep(&mut self, v: &Rooted) -> Result<Rooted, Error> {
        let raw = self.raw(v)?;
        Ok(self.root(raw))
    }

    /// The `Value` behind a handle, checking it is ours and still live.
    fn raw(&self, v: &Rooted) -> Result<Value, Error> {
        if v.owner != self.id {
            return Err(Error::ForeignHandle);
        }
        self.vm
            .engine
            .handle_value(v.handle)
            .ok_or(Error::ForeignHandle)
    }

    /// Read a value as host data, to [`MAX_DEPTH`].
    pub fn value(&mut self, v: &Rooted) -> Result<Val, Error> {
        self.enter()?;
        let raw = self.raw(v)?;
        Ok(self.read(raw, 0))
    }

    /// Hand a host value to Scheme.
    pub fn make(&mut self, v: &Val) -> Result<Rooted, Error> {
        self.enter()?;
        let raw = self.build(v, 0)?;
        Ok(self.root(raw))
    }

    /// `(write v)` as a string: the round-trippable spelling.
    pub fn write(&mut self, v: &Rooted) -> Result<String, Error> {
        self.enter()?;
        let raw = self.raw(v)?;
        Ok(crate::writer::write_str(&self.vm.engine, raw))
    }

    /// `(display v)` as a string: the human one.
    /// `write`, pretty-printed to `width` columns, with multiple values on
    /// separate lines: what a REPL shows for a result.
    pub fn pretty(&mut self, v: &Rooted, width: usize) -> Result<String, Error> {
        self.enter()?;
        let value = self.raw(v)?;
        if let Value::Obj(r) = value
            && let Obj::Values(vs) = self.vm.engine.obj(r)
        {
            let vs = vs.clone();
            return Ok(vs
                .iter()
                .map(|x| crate::writer::pretty_str(&self.vm.engine, *x, width))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        Ok(crate::writer::pretty_str(&self.vm.engine, value, width))
    }

    pub fn display(&mut self, v: &Rooted) -> Result<String, Error> {
        self.enter()?;
        let raw = self.raw(v)?;
        Ok(crate::writer::display_str(&self.vm.engine, raw))
    }

    /// Scheme's `eqv?`, which is the question [`Rooted`]'s `PartialEq` is not
    /// answering.
    pub fn eqv(&self, a: &Rooted, b: &Rooted) -> Result<bool, Error> {
        Ok(self.raw(a)? == self.raw(b)?)
    }

    /// How many values the host is currently holding. For leak tests: a host
    /// that drops its handles should see this return to where it started.
    pub fn rooted_count(&self) -> usize {
        self.vm.engine.rooted_count()
    }

    // ---- introspection (plan §8 obligation 6) ---------------------------

    /// Global names beginning with `prefix`, sorted. The REPL's completion.
    pub fn complete(&self, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .vm
            .global_names()
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            // A renamed template identifier (`tmp.1274`) is an implementation
            // detail of hygiene and must not be offered as a completion.
            .filter(|n| !n.contains('.') || n.parse::<f64>().is_ok())
            .map(str::to_string)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// What a name is: how to call it, and what it says about itself.
    pub fn describe(&mut self, name: &str) -> Option<Description> {
        let sym = self.vm.engine.interner.symbol(name);
        let v = self.vm.global(sym)?;
        Some(Description {
            name: name.to_string(),
            signature: self.vm.signature_of(sym),
            doc: self.vm.doc_of(sym).map(str::to_string),
            kind: match v {
                Value::Primitive(_) => "primitive",
                Value::Obj(r) if matches!(self.vm.engine.obj(r), Obj::Closure { .. }) => {
                    "procedure"
                }
                Value::Obj(r) if matches!(self.vm.engine.obj(r), Obj::Parameter { .. }) => {
                    "parameter"
                }
                _ => "variable",
            }
            .to_string(),
            value: crate::writer::write_str(&self.vm.engine, v),
        })
    }

    /// Whether a name is bound as syntax rather than as a variable, which a
    /// client needs to know before offering to show its value.
    pub fn is_syntax(&mut self, name: &str) -> bool {
        let sym = self.vm.engine.interner.symbol(name);
        self.vm.is_macro(sym)
    }

    // ---- evaluation with output captured --------------------------------

    /// Evaluate, and return whatever the form *printed* alongside its value.
    ///
    /// A socket client cannot see the server's stdout, so output has to come
    /// back in the answer. Done by rebinding `current-output-port`'s
    /// parameter slot **from Rust** rather than by wrapping the source in
    /// `parameterize`: wrapping would put the form inside a `lambda` and a
    /// top-level `(define x 1)` would silently become a local.
    ///
    /// The output is returned even when the evaluation fails, because what a
    /// form printed before it broke is usually the most useful part.
    pub fn eval_capturing(&mut self, src: &str) -> (Result<Rooted, Error>, String) {
        self.eval_capturing_named(src, None)
    }

    /// As [`Interp::eval_capturing`], with a name for the input.
    pub fn eval_capturing_named(
        &mut self,
        src: &str,
        source: Option<&str>,
    ) -> (Result<Rooted, Error>, String) {
        let Some(slot) = self.output_slot() else {
            // No `current-output-port` means a prelude older than ports;
            // evaluate without capturing rather than refuse.
            return match source {
                Some(name) => (self.eval_named(src, name), String::new()),
                None => (self.eval(src), String::new()),
            };
        };
        let port = self.vm.engine.alloc(Obj::Port {
            kind: crate::heap::PortKind::StringOut(String::new()),
            open: true,
        });
        // Root it: the eval below will collect, and the only other reference
        // is the parameter slot, which is not a root.
        let keep = self.vm.engine.root(port);
        let saved = self.vm.param_value(slot);
        self.vm.set_param_value(slot, port);
        let out = match source {
            Some(name) => self.eval_named(src, name),
            None => self.eval(src),
        };
        self.vm.set_param_value(slot, saved);
        let text = match port {
            Value::Obj(r) => match self.vm.engine.obj(r) {
                Obj::Port {
                    kind: crate::heap::PortKind::StringOut(buf),
                    ..
                } => buf.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        self.vm.engine.unroot(keep);
        (out, text)
    }

    /// The parameter slot behind `current-output-port`.
    fn output_slot(&mut self) -> Option<usize> {
        let sym = self.vm.engine.interner.symbol("current-output-port");
        let Value::Obj(r) = self.vm.global(sym)? else {
            return None;
        };
        match self.vm.engine.obj(r) {
            Obj::Parameter { slot, .. } => Some(*slot),
            _ => None,
        }
    }

    // ---- internals ------------------------------------------------------

    /// Every entry point starts here: release dropped handles, and refuse if
    /// poisoned.
    fn enter(&mut self) -> Result<(), Error> {
        if let Some(_m) = &self.poisoned {
            return Err(Error::Poisoned);
        }
        let dropped: Vec<Handle> = std::mem::take(&mut *self.pending.borrow_mut());
        for h in dropped {
            self.vm.engine.unroot(h);
        }
        Ok(())
    }

    fn convert(&mut self, e: VmError) -> Error {
        match e {
            VmError::Interrupted => Error::Interrupted,
            VmError::OutOfFuel => Error::OutOfFuel,
            VmError::OutOfMemory => Error::OutOfMemory,
            VmError::Error(m) => {
                // The reader's two answers have to survive to the host, since
                // an editor's RET depends on the difference.
                match crate::parse_status(&m) {
                    _ if m.starts_with("read error: ") => {
                        let rest = m.trim_start_matches("read error: ").to_string();
                        if rest.contains("unexpected end of input") || rest.contains("incomplete") {
                            Error::Incomplete
                        } else {
                            Error::Syntax(rest)
                        }
                    }
                    _ => Error::Script(m),
                }
            }
            VmError::Raised(v, text) => {
                let value = self.root(v);
                Error::Raised { text, value }
            }
        }
    }

    fn read(&mut self, v: Value, depth: usize) -> Val {
        let (id, pending) = (self.id, Rc::clone(&self.pending));
        read_val_rooting(&mut self.vm.engine, v, depth, id, &pending)
    }

    fn build(&mut self, v: &Val, depth: usize) -> Result<Value, Error> {
        build_val(&mut self.vm.engine, v, depth)
    }
}

/// A `Value` as host data, to [`MAX_DEPTH`].
///
/// A free function rather than a method because the host-function dispatch in
/// [`crate::vm`] needs it too, and it only ever needs the heap.
/// As [`read_val`], but a procedure becomes a rooted [`Val::Proc`] owned by
/// the interpreter `id`.
pub(crate) fn read_val_rooting(engine: &mut crate::heap::Engine, v: Value, depth: usize, id: u64, pending: &Pending) -> Val {
    let is_proc = match v {
        Value::Primitive(_) => true,
        Value::Obj(r) => matches!(engine.obj(r), Obj::Closure { .. } | Obj::Parameter { .. } | Obj::Continuation { .. }),
        _ => false,
    };
    if is_proc {
        return Val::Proc(Proc(Rc::new(Rooted { handle: engine.root(v), owner: id, pending: Rc::clone(pending) })));
    }
    if depth >= MAX_DEPTH {
        return Val::Opaque;
    }
    // Containers are read element by element so a procedure inside one is
    // rooted too.
    if let Value::Obj(r) = v {
        match engine.obj(r).clone() {
            Obj::Vector { items, .. } => {
                return Val::Vector(items.iter().map(|&x| read_val_rooting(engine, x, depth + 1, id, pending)).collect());
            }
            Obj::Pair(..) => {
                let mut out = Vec::new();
                let mut cur = v;
                let cap = engine.obj_count() + 1;
                loop {
                    match cur {
                        Value::Null => return Val::List(out),
                        Value::Obj(p) => match engine.obj(p) {
                            Obj::Pair(car, cdr) => {
                                let (car, cdr) = (*car, *cdr);
                                out.push(read_val_rooting(engine, car, depth + 1, id, pending));
                                cur = cdr;
                                if out.len() > cap {
                                    return Val::Opaque;
                                }
                            }
                            _ => return Val::Opaque,
                        },
                        _ => return Val::Opaque,
                    }
                }
            }
            _ => {}
        }
    }
    read_val(engine, v, depth)
}

pub(crate) fn read_val(engine: &crate::heap::Engine, v: Value, depth: usize) -> Val {
    if depth > MAX_DEPTH {
        return Val::Opaque;
    }
    {
        match v {
            Value::Null => Val::Null,
            Value::Unspecified | Value::Eof | Value::Undefined => Val::Unspecified,
            Value::Bool(b) => Val::Bool(b),
            Value::Fixnum(n) => Val::Int(n),
            Value::Flonum(x) => Val::Float(x),
            Value::Char(c) => Val::Char(c),
            Value::Symbol(s) => Val::Sym(engine.interner.sym_name(s).to_string()),
            Value::Keyword(k) => Val::Keyword(engine.interner.kw_name(k).to_string()),
            Value::Primitive(_) => Val::Opaque,
            Value::Obj(r) => match engine.obj(r) {
                Obj::Str(cs) => Val::Str(cs.iter().collect()),
                Obj::Bytevector(b) => Val::Bytes(b.clone()),
                Obj::Vector { items, .. } => {
                    Val::Vector(items.iter().map(|&i| read_val(engine, i, depth + 1)).collect())
                }
                Obj::Pair(..) => {
                    let mut out = Vec::new();
                    let mut cur = v;
                    loop {
                        match cur {
                            Value::Null => return Val::List(out),
                            Value::Obj(r2) => match engine.obj(r2) {
                                Obj::Pair(car, cdr) => {
                                    if out.len() > MAX_DEPTH {
                                        return Val::Opaque;
                                    }
                                    out.push(read_val(engine, *car, depth + 1));
                                    cur = *cdr;
                                }
                                // An improper tail. Saying `Opaque` is better
                                // than returning a list that lost it.
                                _ => return Val::Opaque,
                            },
                            _ => return Val::Opaque,
                        }
                    }
                }
                _ => Val::Opaque,
            },
        }
    }
}

/// A host value as a `Value`.
pub(crate) fn build_val(
    engine: &mut crate::heap::Engine,
    v: &Val,
    depth: usize,
) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Script("value nested too deeply".into()));
        }
        Ok(match v {
            Val::Null => Value::Null,
            Val::Unspecified => Value::Unspecified,
            Val::Bool(b) => Value::Bool(*b),
            Val::Int(n) => Value::Fixnum(*n),
            Val::Float(x) => Value::Flonum(*x),
            Val::Char(c) => Value::Char(*c),
            Val::Str(s) => engine.string(s),
            Val::Sym(s) => engine.sym(s),
            Val::Keyword(k) => engine.kw(k),
            Val::Bytes(b) => engine.alloc(Obj::Bytevector(b.clone())),
            Val::List(items) => {
                let mut built = Vec::with_capacity(items.len());
                for i in items {
                    built.push(build_val(engine, i, depth + 1)?);
                }
                engine.list(&built)
            }
            Val::Vector(items) => {
                let mut built = Vec::with_capacity(items.len());
                for i in items {
                    built.push(build_val(engine, i, depth + 1)?);
                }
                engine.vector(built, false)
            }
            // A host cannot manufacture a procedure or a port, and silently
            // substituting something would be worse than refusing.
            Val::Proc(p) => engine.handle_value(p.0.handle).ok_or(Error::ForeignHandle)?,
            Val::Opaque => {
                return Err(Error::Script(
                    "Val::Opaque cannot be turned back into a value".into(),
                ));
            }
        })
    }

/// Run `f`, turning a panic into an [`Error`] rather than letting it cross
/// the API.
///
/// Risk 8, in ray's own blood: a panic cannot unwind out of `extern "C"` and
/// **aborts the process** -- a zero weight killed Kevin's modeler mid-edit
/// (@7d8c8a1). A panic crossing this API into ray would be the same bug with
/// a new door, and there are 88 panic sites behind 95 public items.
fn guard<T>(f: impl FnOnce() -> T) -> Result<T, Error> {
    let hushed = std::panic::catch_unwind(AssertUnwindSafe(f));
    hushed.map_err(|e| {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "a panic with no message".to_string()
        };
        Error::Panicked(msg)
    })
}
