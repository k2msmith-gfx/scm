//! S41: `map`, `for-each`, `filter`, `reduce` and `(srfi 1)`'s `fold` and
//! `for-each`, run by the machine rather than written in Scheme.
//!
//! A primitive cannot call a procedure -- a `PrimFn` gets the engine and
//! its arguments and nothing else -- so these are recognised in
//! [`Vm::apply`] like `dynamic-wind` and `call-with-values`, and keep their
//! state in a [`Frame::Iter`] on the explicit stack. Each element is one
//! call of the procedure from that frame, so everything a Scheme loop had
//! comes for free: the safepoint between elements (fuel, the interrupt,
//! collection), errors unwinding through it to a `guard`, an escape
//! continuation or a `dynamic-wind` below, and the collector tracing the
//! frame. No Rust recursion, and no nested evaluation.
//!
//! What it saves is the Scheme loop around the call: the named `let`, its
//! `null?`, `car`, `cdr` and `cons`, each a dispatch. The procedure itself
//! is entered the way `do_call` enters it: a plain primitive straight off
//! the value stack, a compiled flat-parameter closure with its arguments
//! where they were pushed, anything else through `apply`.
//!
//! **The order of everything is the old Scheme's**, which the test program
//! `scm/tests/srfi/native-map.scm` holds against copies of it: `map`,
//! `fold` and `reduce` take each list's tail *before* calling the
//! procedure, and `for-each` and `filter` after (so a procedure that
//! mutates the pair it was given sees what the Scheme loop saw), and
//! `filter` reads the element again after the predicate returns.

use super::{Frame, Step, VResult, Vm};
use crate::code::CodeState;
use crate::heap::Obj;
use crate::value::Value;
use crate::writer::write_str;

/// Which iteration a [`Frame::Iter`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IterKind {
    /// R7RS `map`, over one or more lists, stopping at the shortest.
    Map,
    /// R7RS `for-each`; the value is unspecified.
    ForEach,
    /// `(srfi 1)`'s `for-each`, which returns `#t`.
    ForEachTrue,
    /// `(srfi 1)`'s `fold`: `(kons e1 e2 ... acc)`.
    Fold,
    /// The prelude's `filter`: one list.
    Filter,
    /// The prelude's `reduce`: `(f acc elem)`, with an init, one list.
    Reduce,
}

impl IterKind {
    /// The name its errors carry -- the procedure's, not the primitive's.
    pub(super) fn name(self) -> &'static str {
        match self {
            IterKind::Map => "map",
            IterKind::ForEach | IterKind::ForEachTrue => "for-each",
            IterKind::Fold => "fold",
            IterKind::Filter => "filter",
            IterKind::Reduce => "reduce",
        }
    }

    /// Whether each list's tail is taken after the call returns rather
    /// than before it: the Scheme `for-each` and `filter` were
    /// `(begin (f (car l)) (loop (cdr l)))`.
    fn advance_after(self) -> bool {
        matches!(self, IterKind::ForEach | IterKind::ForEachTrue | IterKind::Filter)
    }

    /// The primitives, and the names they are installed under.
    pub(super) const ALL: [(&'static str, IterKind); 6] = [
        ("map", IterKind::Map),
        ("for-each", IterKind::ForEach),
        ("%srfi-1-for-each", IterKind::ForEachTrue),
        ("%fold", IterKind::Fold),
        ("filter", IterKind::Filter),
        ("reduce", IterKind::Reduce),
    ];
}

/// The `PrimFn` each iteration's slot holds. [`Vm::apply`] recognises the
/// slot first, and none is plain, so nothing reaches this.
pub(super) fn placeholder(_: &mut crate::heap::Engine, _: &[Value]) -> VResult<Value> {
    Err(super::VmError::Error("an iteration primitive was not dispatched by the machine".into()))
}

/// `(car . cdr)` of a pair, or `None`.
fn pair(vm: &Vm, v: Value) -> Option<(Value, Value)> {
    match v {
        Value::Obj(r) => match vm.engine.obj(r) {
            Obj::Pair(a, d) => Some((*a, *d)),
            _ => None,
        },
        _ => None,
    }
}

impl Vm {
    /// The kind of iteration primitive `id` is, if it is one.
    #[inline]
    pub(super) fn iter_kind(&self, id: crate::value::PrimId) -> Option<IterKind> {
        self.iter_prims.get(id.0 as usize).copied().flatten()
    }

    /// A call of one of the iteration primitives: check the arity, then
    /// take the first step.
    pub(super) fn iter_start(&mut self, kind: IterKind, args: Vec<Value>) -> VResult<Step> {
        let name = kind.name();
        // The arity messages are the ones a Scheme definition of the same
        // lambda list gave, so a wrong call reads as it did.
        let (required, fixed) = match kind {
            IterKind::Map | IterKind::ForEach | IterKind::ForEachTrue => (2, false),
            IterKind::Fold => (3, false),
            IterKind::Filter => (2, true),
            IterKind::Reduce => (3, true),
        };
        if args.len() < required {
            return Err(self.err(format!(
                "{name} expects {required} required arguments, got {}",
                args.len()
            )));
        }
        if fixed && args.len() > required {
            return Err(self.err(format!(
                "{name} got {} arguments, which is more than it takes",
                args.len()
            )));
        }
        let f = args[0];
        let (acc, lists) = match kind {
            IterKind::Fold | IterKind::Reduce => (args[1], &args[2..]),
            _ => (Value::Null, &args[1..]),
        };
        let list = lists[0];
        let more = lists[1..].to_vec();
        self.iter_next(kind, f, list, more, acc)
    }

    /// The next element, or the result. `list` and `more` are each list's
    /// remaining tail, `acc` what has been gathered.
    fn iter_next(
        &mut self,
        kind: IterKind,
        f: Value,
        list: Value,
        mut more: Vec<Value>,
        acc: Value,
    ) -> VResult<Step> {
        // Stop at the shortest: any list empty, before any is taken apart,
        // as the Scheme's `any-null?` did.
        if list == Value::Null || more.contains(&Value::Null) {
            return Ok(Step::Return(self.iter_finish(kind, acc)));
        }
        let name = kind.name();
        let not_list = |vm: &mut Vm, v: Value| {
            let shown = write_str(&vm.engine, v);
            vm.err(format!("{name}: not a list: {shown}"))
        };
        let Some((x, rest)) = pair(self, list) else {
            return Err(not_list(self, list));
        };
        // The callee and its arguments go on the value stack, where
        // `iter_call` enters a compiled procedure without a vector.
        let at = self.vstack.len();
        self.vstack.push(f);
        if kind == IterKind::Reduce {
            self.vstack.push(acc);
        }
        self.vstack.push(x);
        // What the frame resumes from: each list's tail already taken, or
        // for `for-each` and `filter` the pairs themselves.
        let after = kind.advance_after();
        let next = if after { list } else { rest };
        for tail in more.iter_mut() {
            let Some((y, r)) = pair(self, *tail) else {
                let bad = *tail;
                self.vstack.truncate(at);
                return Err(not_list(self, bad));
            };
            self.vstack.push(y);
            if !after {
                *tail = r;
            }
        }
        if kind == IterKind::Fold {
            self.vstack.push(acc);
        }
        self.push(Frame::Iter { kind, f, list: next, more, acc });
        self.iter_call(at)
    }

    /// Call the procedure at `vstack[at]` on the arguments above it.
    fn iter_call(&mut self, at: usize) -> VResult<Step> {
        let f = self.vstack[at];
        let argc = self.vstack.len() - at - 1;
        if let Value::Primitive(id) = f
            && self.plain_prim(id)
        {
            let p = self.prims[id.0 as usize];
            let r = p(&mut self.engine, &self.vstack[at + 1..]);
            self.vstack.truncate(at);
            return r.map(Step::Return);
        }
        if let Value::Obj(r) = f
            && let Obj::Closure { params, env, code: CodeState::Compiled(c), .. } = self.engine.obj(r)
            && c.flat_params
            && params.len() == argc
            && c.macro_gen == self.macro_gen
        {
            let (env, c) = (*env, std::rc::Rc::clone(c));
            return Ok(self.enter_code_at(c, env, at));
        }
        let args = self.vstack.split_off(at + 1);
        self.vstack.truncate(at);
        self.apply(f, args)
    }

    /// The procedure returned `v` to a [`Frame::Iter`].
    pub(super) fn iter_resume(
        &mut self,
        kind: IterKind,
        f: Value,
        list: Value,
        mut more: Vec<Value>,
        acc: Value,
        v: Value,
    ) -> VResult<Step> {
        match kind {
            IterKind::Map => {
                let acc = self.engine.cons(v, acc);
                self.iter_next(kind, f, list, more, acc)
            }
            IterKind::Fold | IterKind::Reduce => self.iter_next(kind, f, list, more, v),
            IterKind::ForEach | IterKind::ForEachTrue | IterKind::Filter => {
                // `list` and `more` are the pairs the call was given; they
                // were pairs then, and only a `set-cdr!` could change
                // what follows them, which is read now, as the Scheme did.
                let (x, rest) = pair(self, list).expect("the frame holds the pair it called on");
                for tail in more.iter_mut() {
                    *tail = pair(self, *tail).expect("the frame holds the pairs it called on").1;
                }
                let acc = if kind == IterKind::Filter && v.is_truthy() {
                    self.engine.cons(x, acc)
                } else {
                    acc
                };
                self.iter_next(kind, f, rest, more, acc)
            }
        }
    }

    /// What the iteration returns once a list has run out.
    fn iter_finish(&mut self, kind: IterKind, acc: Value) -> Value {
        match kind {
            IterKind::Map | IterKind::Filter => {
                let mut out = Value::Null;
                let mut cur = acc;
                while let Some((x, rest)) = pair(self, cur) {
                    out = self.engine.cons(x, out);
                    cur = rest;
                }
                out
            }
            IterKind::ForEach => Value::Unspecified,
            IterKind::ForEachTrue => Value::Bool(true),
            IterKind::Fold | IterKind::Reduce => acc,
        }
    }
}
