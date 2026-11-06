//! The T1 run loop: one `Frame::Code` executing until it calls, returns,
//! or opens a sub-block. Calls go through the same `apply` as T0, so a
//! compiled frame calling an interpreted closure -- or the reverse -- is
//! nothing special.

use std::rc::Rc;

use super::{Frame, Step, VResult, Vm, VmError};
use crate::code::{CodeState, Op};
use crate::heap::Obj;
use crate::value::{ObjRef, Value};

/// S31: what `do_call` did with a call.
pub(super) enum Called {
    /// A plain primitive: its result is on the value stack, carry on.
    Done,
    /// Take this step; the caller's frame is already pushed, or discarded
    /// for a tail call.
    Step(Step),
    /// Enter compiled `code` with its frame at `at`. With `suspend` the
    /// caller pushes its own frame first -- moving its `Rc<Code>` into it
    /// rather than cloning one for `do_call` to push, since the caller's
    /// loop is leaving anyway. A tail call has no frame to keep.
    Enter { code: Rc<crate::code::Code>, env: Option<ObjRef>, at: usize, suspend: bool },
}

impl Vm {
    /// Push a frame for `code` and ask the loop to run it.
    pub(super) fn enter_code(&mut self, code: Rc<crate::code::Code>, env: Option<ObjRef>) -> Step {
        let base = self.vstack.len();
        self.enter_code_at(code, env, base)
    }

    /// S14: enter `code` with its frame at `base`, reserving its flat slots.
    /// A flat-parameter procedure's callee and arguments are already at
    /// `base` -- where the call left them -- and the rest of its slots
    /// start undefined.
    pub(super) fn enter_code_at(&mut self, code: Rc<crate::code::Code>, env: Option<ObjRef>, base: usize) -> Step {
        let want = base + code.nslots as usize;
        if self.vstack.len() < want {
            self.vstack.resize(want, Value::Undefined);
        }
        self.push(Frame::Code { code, pc: 0, env, base });
        Step::Run
    }

    /// The frame `depth` levels up the chain from `env`.
    pub(super) fn frame_at(&self, env: Option<ObjRef>, depth: u16) -> VResult<ObjRef> {
        let mut cur = env;
        for _ in 0..depth {
            let Some(r) = cur else { break };
            let Obj::Env { parent, .. } = self.engine.obj(r) else {
                return Err(VmError::Error("corrupt environment".into()));
            };
            cur = *parent;
        }
        cur.ok_or_else(|| VmError::Error("corrupt environment: frame missing".into()))
    }

    /// A primitive that takes the engine and its arguments and nothing
    /// else: the one kind that can be called on a slice of the value stack
    /// with no vector built.
    #[inline]
    pub(super) fn plain_prim(&self, id: crate::value::PrimId) -> bool {
        let i = id.0 as usize;
        // One indexed load. The three tables and the five special ids are
        // all fixed at registration time, so the answer is precomputed in
        // `Vm::plain`; see its declaration for who must keep it true.
        let cached = self.plain.get(i).copied().unwrap_or(true);
        debug_assert_eq!(cached, self.plain_prim_uncached(id), "the plain-prim cache disagrees with the tables for prim {i}");
        cached
    }

    /// The question `plain_prim` answers, asked of the tables directly.
    /// Only the `debug_assert` above calls it; it exists so the cache has
    /// something to be checked against on every call under `cargo test`.
    fn plain_prim_uncached(&self, id: crate::value::PrimId) -> bool {
        let i = id.0 as usize;
        if self.vm_prims.get(i).is_some_and(|p| p.is_some()) {
            return false;
        }
        if self.host_fns.get(i).is_some_and(|h| h.is_some()) {
            return false;
        }
        let special = [
            self.apply_prim,
            self.call_with_values_prim,
            self.eval_prim,
            self.callcc_prim,
            self.dynamic_wind_prim,
        ];
        !special.contains(&Some(id))
    }

    /// `(%parameterize)`: the `pairs` parameter/value pairs on top of the
    /// value stack become dynamic bindings; returns how many to restore.
    pub(super) fn install_params(&mut self, pairs: usize) -> VResult<usize> {
        let values = self.vstack.split_off(self.vstack.len() - 2 * pairs);
        let mut count = 0;
        for pair in values.chunks(2) {
            let (param, value) = (pair[0], pair[1]);
            let slot = match param {
                Value::Obj(r) => match self.engine.obj(r) {
                    Obj::Parameter { slot, .. } => *slot,
                    _ => return Err(self.err("parameterize: not a parameter")),
                },
                _ => return Err(self.err("parameterize: not a parameter")),
            };
            let old = self.param_values[slot];
            self.param_bindings.push((slot, old));
            self.param_values[slot] = value;
            count += 1;
        }
        Ok(count)
    }

    /// A call whose callee and `argc` arguments are on top of the value
    /// stack. `Called::Done` means it was a plain primitive and its result
    /// is on the stack: carry on in this frame. `Called::Enter` is a
    /// compiled flat-parameter procedure to enter, the caller pushing its
    /// own frame; `Called::Step` is any other step, with this frame already
    /// pushed if the call was not a tail call. Shared by T1 and T2.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn do_call(
        &mut self,
        code: &Rc<crate::code::Code>,
        pc: usize,
        env: Option<ObjRef>,
        base: usize,
        argc: u32,
        form: u32,
        tail: bool,
    ) -> VResult<Called> {
        let start = self.vstack.len() - argc as usize - 1;
        let callee = self.vstack[start];
        self.current_call = code.forms[form as usize];
        if let Value::Primitive(id) = callee
            && self.plain_prim(id)
        {
            let f = self.prims[id.0 as usize];
            let r = f(&mut self.engine, &self.vstack[start + 1..]);
            self.vstack.truncate(start);
            let v = r?;
            self.current_call = Value::Unspecified;
            if tail {
                self.vstack.truncate(base);
                return Ok(Called::Step(Step::Return(v)));
            }
            self.vstack.push(v);
            return Ok(Called::Done);
        }
        if let Value::Obj(r) = callee
            && let Obj::Closure { params, env: cenv, code: CodeState::Compiled(c), .. } = self.engine.obj(r)
        {
            // S14: a flat-parameter procedure takes its arguments where they
            // are. The callee becomes slot 0 and the arguments slots 1..;
            // nothing is allocated. A tail call slides them down over this
            // frame first. `flat_params` already means a plain lambda list
            // (`body_unit`), so arity and the macro generation are all
            // there is to check.
            if c.flat_params && params.len() == argc as usize && c.macro_gen == self.macro_gen {
                let (cenv, c) = (*cenv, Rc::clone(c));
                let at = if tail {
                    self.vstack.drain(base..start);
                    base
                } else {
                    start
                };
                return Ok(Called::Enter { code: c, env: cenv, at, suspend: !tail });
            }
        }
        // A compiled closure with a plain lambda list but a heap frame: the
        // frame is built straight from the operand stack, no argument
        // vector in between. Everything else -- T0 closures, optional and
        // named parameters, arity errors, the special primitives -- goes
        // through the one `apply`, which also compiles on first call.
        if let Value::Obj(r) = callee
            && let Obj::Closure { params, opt, named, keys, rest, env: cenv, code: CodeState::Compiled(c), .. } =
                self.engine.obj(r)
            && opt.is_empty()
            && named.is_empty()
            && keys.is_none()
            && rest.is_none()
            && params.len() == argc as usize
            && c.macro_gen == self.macro_gen
        {
            let (params, cenv, c) = (Rc::clone(params), *cenv, Rc::clone(c));
            let mut slots: Vec<(crate::value::SymId, Value)> = self.engine.slots(params.len() + c.defines.len());
            slots.extend(params.iter().copied().zip(self.vstack[start + 1..].iter().copied()));
            slots.extend(c.defines.iter().map(|&d| (d, Value::Undefined)));
            if tail {
                self.vstack.truncate(base);
            } else {
                self.vstack.truncate(start);
                self.push(Frame::Code { code: Rc::clone(code), pc, env, base });
            }
            let new_env = self.engine.env(cenv, slots);
            return Ok(Called::Step(self.enter_code(c, Some(new_env))));
        }
        let args = self.vstack.split_off(start + 1);
        if tail {
            self.vstack.truncate(base);
        } else {
            self.vstack.truncate(start);
            self.push(Frame::Code { code: Rc::clone(code), pc, env, base });
        }
        self.apply(callee, args).map(Called::Step)
    }

    pub(super) fn run_code(
        &mut self,
        mut code: Rc<crate::code::Code>,
        mut pc: usize,
        mut env: Option<ObjRef>,
        mut base: usize,
    ) -> VResult<Step> {
        // T2, if this code has it or has earned it.
        if let Some(f) = self.native_for(&code) {
            return self.run_native(code, f, pc, env, base);
        }
        // S31: frames above this are callers this loop suspended itself, so
        // a `Return` can pop one and carry on here without going back to
        // the trampoline.
        let floor = self.stack.len();
        // On any failure the frame goes back on the stack with `pc` just
        // past the op in progress, so the backtrace can name the calls
        // pending in it -- and so the collector, on a safepoint, sees it.
        macro_rules! fail {
            ($e:expr) => {{
                let e = $e;
                self.stack.push(Frame::Code { code, pc, env, base });
                return Err(e);
            }};
        }
        // A call: carry on after a plain primitive, enter a compiled
        // flat-parameter procedure with this frame suspended under it (its
        // `code` moved into the frame, not cloned), or take any other step.
        macro_rules! call {
            ($argc:expr, $form:expr, $tail:expr) => {
                match self.do_call(&code, pc, env, base, $argc, $form, $tail) {
                    Ok(Called::Done) => {}
                    Ok(Called::Step(step)) => return Ok(step),
                    Ok(Called::Enter { code: callee, env: cenv, at, suspend }) => {
                        // S31: enter the callee here. A flat unit never has
                        // native code (`native_for` declines them), so this
                        // is always the loop that would have run it.
                        if suspend {
                            self.push(Frame::Code { code, pc, env, base });
                        }
                        let want = at + callee.nslots as usize;
                        if self.vstack.len() < want {
                            self.vstack.resize(want, Value::Undefined);
                        }
                        code = callee;
                        pc = 0;
                        env = cenv;
                        base = at;
                        continue;
                    }
                    Err(e) => fail!(e),
                }
            };
        }
        loop {
            self.ticks += 1;
            if self.ticks >= 1024 {
                self.ticks = 0;
                self.stack.push(Frame::Code { code: Rc::clone(&code), pc, env, base });
                let r = self.safepoint(&Step::Run);
                self.stack.pop();
                if let Err(e) = r {
                    fail!(e);
                }
            }
            let op = code.ops[pc];
            pc += 1;
            match op {
                Op::Const(i) => self.vstack.push(code.consts[i as usize]),
                Op::Unspecified => self.vstack.push(Value::Unspecified),
                Op::Pop => {
                    self.vstack.pop();
                }
                Op::Local { depth, idx } => {
                    let r = match self.frame_at(env, depth) {
                        Ok(r) => r,
                        Err(e) => fail!(e),
                    };
                    let Obj::Env { slots, .. } = self.engine.obj(r) else { unreachable!() };
                    let v = slots[idx as usize].1;
                    self.vstack.push(v);
                }
                Op::LocalDef { depth, idx, sym } => {
                    let r = match self.frame_at(env, depth) {
                        Ok(r) => r,
                        Err(e) => fail!(e),
                    };
                    let Obj::Env { slots, .. } = self.engine.obj(r) else { unreachable!() };
                    let v = slots[idx as usize].1;
                    if v == Value::Undefined {
                        fail!(self.err(format!("unbound variable: {}", self.engine.interner.sym_name(crate::value::SymId(sym)))));
                    }
                    self.vstack.push(v);
                }
                Op::Arg(i) => {
                    let v = self.vstack[base + i as usize];
                    self.vstack.push(v);
                }
                Op::SetArg(i) => {
                    let v = self.vstack.pop().expect("a value to store");
                    self.vstack[base + i as usize] = v;
                }
                Op::ArgDef { slot, sym } => {
                    let v = self.vstack[base + slot as usize];
                    if v == Value::Undefined {
                        fail!(self.err(format!("unbound variable: {}", self.engine.interner.sym_name(crate::value::SymId(sym)))));
                    }
                    self.vstack.push(v);
                }
                Op::SetArgDef { slot, sym } => {
                    let v = self.vstack.pop().expect("a value to store");
                    if self.vstack[base + slot as usize] == Value::Undefined {
                        fail!(self.err(format!("set!: unbound variable: {}", self.engine.interner.sym_name(crate::value::SymId(sym)))));
                    }
                    self.vstack[base + slot as usize] = v;
                }
                Op::Slot { frame, idx } => {
                    let Obj::Env { slots, .. } = self.engine.obj(ObjRef(frame)) else { unreachable!() };
                    let v = slots[idx as usize].1;
                    self.vstack.push(v);
                }
                Op::SetLocalDef { depth, idx, sym } => {
                    let v = self.vstack.pop().expect("a value to store");
                    let r = match self.frame_at(env, depth) {
                        Ok(r) => r,
                        Err(e) => fail!(e),
                    };
                    if let Obj::Env { slots, .. } = self.engine.obj_mut(r) {
                        if slots[idx as usize].1 == Value::Undefined {
                            fail!(self.err(format!("set!: unbound variable: {}", self.engine.interner.sym_name(crate::value::SymId(sym)))));
                        }
                        slots[idx as usize].1 = v;
                    }
                }
                Op::SetLocal { depth, idx } => {
                    let v = self.vstack.pop().expect("a value to store");
                    let r = match self.frame_at(env, depth) {
                        Ok(r) => r,
                        Err(e) => fail!(e),
                    };
                    if let Obj::Env { slots, .. } = self.engine.obj_mut(r) {
                        slots[idx as usize].1 = v;
                    }
                }
                Op::Global(i) => {
                    let (sym, resolved) = code.globals[i as usize];
                    let v = match self.global(sym) {
                        Some(v) => v,
                        None => match if resolved != sym { self.global(resolved) } else { None } {
                            Some(v) => v,
                            None => fail!(self.err(format!(
                                "unbound variable: {}",
                                self.engine.interner.sym_name(sym)
                            ))),
                        },
                    };
                    self.vstack.push(v);
                }
                Op::SetGlobal(i) | Op::DefineGlobal(i) => {
                    let (sym, _) = code.globals[i as usize];
                    let v = self.vstack.pop().expect("a value to store");
                    if let Err(e) = self.bind(sym, v, matches!(op, Op::DefineGlobal(_)), None) {
                        fail!(e);
                    }
                }
                Op::Arith { op, prim, form } => {
                    let unary = op == crate::code::ArithOp::Neg;
                    let n = if unary { 2 } else { 3 };
                    let at = self.vstack.len() - n;
                    let (callee, a) = (self.vstack[at], self.vstack[at + 1]);
                    let b = if unary { Value::Unspecified } else { self.vstack[at + 2] };
                    let pid = crate::value::PrimId(prim);
                    if callee == Value::Primitive(pid) {
                        if let Some(v) = arith_fast(op, a, b) {
                            self.vstack.truncate(at);
                            self.current_call = Value::Unspecified;
                            self.vstack.push(v);
                            continue;
                        }
                    }
                    // Not a case the fast path answers, or not the
                    // primitive any more: an ordinary call, which for the
                    // primitive is `do_call`'s plain path.
                    call!((n - 1) as u32, form, false);
                }
                Op::ArithSrc(i) => {
                    let s = code.ariths[i as usize];
                    let unary = s.op == crate::code::ArithOp::Neg;
                    // How many of callee, a, b are on the stack, callee lowest.
                    let on_stack = s.global.is_none() as usize
                        + (s.a == crate::code::Src::Stack) as usize
                        + (!unary && s.b == crate::code::Src::Stack) as usize;
                    let at = self.vstack.len() - on_stack;
                    let mut next = at;
                    let callee = match s.global {
                        None => {
                            next += 1;
                            self.vstack[at]
                        }
                        Some(g) => {
                            let (sym, resolved) = code.globals[g as usize];
                            match self.global(sym).or_else(|| if resolved != sym { self.global(resolved) } else { None }) {
                                Some(v) => v,
                                None => fail!(self.err(format!("unbound variable: {}", self.engine.interner.sym_name(sym)))),
                            }
                        }
                    };
                    let mut read = |src: crate::code::Src| match src {
                        crate::code::Src::Stack => {
                            let v = self.vstack[next];
                            next += 1;
                            v
                        }
                        crate::code::Src::Arg(i) => self.vstack[base + i as usize],
                        crate::code::Src::Const(c) => code.consts[c as usize],
                    };
                    let a = read(s.a);
                    let b = if unary { Value::Unspecified } else { read(s.b) };
                    if callee == Value::Primitive(crate::value::PrimId(s.prim))
                        && let Some(v) = arith_fast(s.op, a, b)
                    {
                        self.vstack.truncate(at);
                        self.current_call = Value::Unspecified;
                        match s.branch {
                            Some(t) => {
                                if v.is_truthy() {
                                    pc += 1;
                                } else {
                                    pc = t as usize;
                                }
                            }
                            None => self.vstack.push(v),
                        }
                        continue;
                    }
                    // The primitive's own path, or another callee: lay the
                    // call out as `Arith` has it and make the call. With a
                    // branch, the `JumpIfFalse` at `pc` tests the result.
                    self.vstack.truncate(at);
                    self.vstack.push(callee);
                    self.vstack.push(a);
                    if !unary {
                        self.vstack.push(b);
                    }
                    call!(if unary { 1 } else { 2 }, s.form, false);
                }
                Op::Jump(t) => pc = t as usize,
                Op::JumpCall { target, form } => {
                    self.current_call = code.forms[form as usize];
                    pc = target as usize;
                }
                Op::JumpIfFalse(t) => {
                    let v = self.vstack.pop().expect("a test value");
                    if !v.is_truthy() {
                        pc = t as usize;
                    }
                }
                Op::Call { argc, form } | Op::TailCall { argc, form } => {
                    call!(argc, form, matches!(op, Op::TailCall { .. }));
                }
                Op::Return => {
                    let v = self.vstack.pop().expect("a value to return");
                    self.vstack.truncate(base);
                    // S31: back into a compiled caller this loop suspended,
                    // unless that caller runs natively -- the trampoline
                    // resumes those.
                    if self.stack.len() > floor
                        && let Some(Frame::Code { code: c, .. }) = self.stack.last()
                        && c.native.get().is_none()
                    {
                        let Some(Frame::Code { code: c, pc: p, env: e, base: b }) = self.stack.pop() else { unreachable!() };
                        self.expansions = 0;
                        self.current_call = Value::Unspecified;
                        self.vstack.push(v);
                        code = c;
                        pc = p;
                        env = e;
                        base = b;
                        continue;
                    }
                    return Ok(Step::Return(v));
                }
                Op::MakeClosure(i) => {
                    let l = &code.lambdas[i as usize];
                    let v = self.engine.alloc(Obj::Closure {
                        name: l.name,
                        params: Rc::clone(&l.params),
                        opt: Rc::clone(&l.opt),
                        named: Rc::clone(&l.named),
                        keys: l.keys,
                        rest: l.rest,
                        body: Rc::clone(&l.body),
                        env,
                        code: CodeState::Compiled(Rc::clone(&l.code)),
                    });
                    self.vstack.push(v);
                }
                Op::PushEnv { names, ninit } => {
                    let names = &code.names[names as usize];
                    let start = self.vstack.len() - ninit as usize;
                    let mut slots = self.engine.slots(names.len());
                    slots.extend(names.iter().enumerate().map(|(i, &n)| {
                        (n, self.vstack.get(start + i).copied().unwrap_or(Value::Undefined))
                    }));
                    self.vstack.truncate(start);
                    env = Some(self.engine.env(env, slots));
                }
                Op::PopEnv => {
                    let Some(r) = env else { fail!(self.err("corrupt environment")) };
                    let Obj::Env { parent, .. } = self.engine.obj(r) else { unreachable!() };
                    env = *parent;
                }
                Op::InsertBelow(n) => {
                    let v = self.vstack.pop().expect("a value to insert");
                    let at = self.vstack.len() - n as usize;
                    self.vstack.insert(at, v);
                }
                Op::MakeVector(n) => {
                    let items = self.vstack.split_off(self.vstack.len() - n as usize);
                    let v = self.engine.vector(items, true);
                    self.vstack.push(v);
                }
                Op::Try(b) => {
                    let block = Rc::clone(&code.blocks[b as usize]);
                    let param_depth = self.param_bindings.len();
                    self.push(Frame::Code { code, pc, env, base });
                    self.push(Frame::Catch { param_depth });
                    return Ok(self.enter_code(block, env));
                }
                Op::Interp(f) => {
                    let form = code.forms[f as usize];
                    self.push(Frame::Code { code, pc, env, base });
                    return Ok(Step::Eval(form, env));
                }
                Op::Parameterize { pairs, block } => {
                    let count = match self.install_params(pairs as usize) {
                        Ok(c) => c,
                        Err(e) => fail!(e),
                    };
                    let block = Rc::clone(&code.blocks[block as usize]);
                    self.push(Frame::Code { code, pc, env, base });
                    self.push(Frame::ParamRestore { count });
                    return Ok(self.enter_code(block, env));
                }
            }
        }
    }
}

/// S14e: the cases `Op::Arith` computes in place, each exactly as the
/// primitive would (prim.rs `arith`, `div`, `compare`): `None` for every
/// other case, which then goes to the primitive.
///
/// - `+ - *`: two fixnums by checked arithmetic (overflow -> `None`: the
///   primitive widens to a bignum); a fixnum and a flonum, or two flonums,
///   as the primitive's float fold does -- each operand `as f64`, in order.
/// - unary `-`: negation, so `(- 0.0)` is `-0.0` (not `0 - x`, which
///   loses the sign of zero); a fixnum is `0 - x`, checked.
/// - `/`: only with a flonum operand (the float fold); two fixnums have the
///   exact-division and exact-zero rules, which are the primitive's.
/// - comparisons: two fixnums, or two flonums (a NaN compares false); a
///   mixed pair is compared EXACTLY by the primitive, not by conversion.
fn arith_fast(op: crate::code::ArithOp, a: Value, b: Value) -> Option<Value> {
    use crate::code::ArithOp as A;
    use std::cmp::Ordering;
    let fl = |v: Value| match v {
        Value::Fixnum(n) => Some(n as f64),
        Value::Flonum(x) => Some(x),
        _ => None,
    };
    let floats = matches!((a, b), (Value::Flonum(_), Value::Flonum(_) | Value::Fixnum(_)) | (Value::Fixnum(_), Value::Flonum(_)));
    match op {
        A::Add | A::Sub | A::Mul => {
            if let (Value::Fixnum(x), Value::Fixnum(y)) = (a, b) {
                let r = match op {
                    A::Add => x.checked_add(y),
                    A::Sub => x.checked_sub(y),
                    _ => x.checked_mul(y),
                }?;
                return Some(Value::Fixnum(r));
            }
            if !floats {
                return None;
            }
            let (x, y) = (fl(a)?, fl(b)?);
            Some(Value::Flonum(match op {
                A::Add => x + y,
                A::Sub => x - y,
                _ => x * y,
            }))
        }
        A::Neg => match a {
            Value::Fixnum(x) => 0i64.checked_sub(x).map(Value::Fixnum),
            Value::Flonum(x) => Some(Value::Flonum(-x)),
            _ => None,
        },
        A::Div => {
            if !floats {
                return None;
            }
            Some(Value::Flonum(fl(a)? / fl(b)?))
        }
        A::Eq | A::Lt | A::Gt | A::Le | A::Ge => {
            let ord = match (a, b) {
                (Value::Fixnum(x), Value::Fixnum(y)) => Some(x.cmp(&y)),
                (Value::Flonum(x), Value::Flonum(y)) => x.partial_cmp(&y),
                _ => return None,
            };
            let holds = |o: Ordering| match op {
                A::Eq => o == Ordering::Equal,
                A::Lt => o == Ordering::Less,
                A::Gt => o == Ordering::Greater,
                A::Le => o != Ordering::Greater,
                _ => o != Ordering::Less,
            };
            Some(Value::Bool(ord.is_some_and(holds)))
        }
    }
}
