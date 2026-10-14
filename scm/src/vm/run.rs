//! The T1 run loop: one `Frame::Code` executing until it calls, returns,
//! or opens a sub-block. Calls go through the same `apply` as T0, so a
//! compiled frame calling an interpreted closure -- or the reverse -- is
//! nothing special.

use std::rc::Rc;

use super::{Frame, Step, VResult, Vm, VmError};
use crate::code::{CodeState, Op};
use crate::heap::Obj;
use crate::value::{ObjRef, Value};

impl Vm {
    /// Push a frame for `code` and ask the loop to run it.
    pub(super) fn enter_code(&mut self, code: Rc<crate::code::Code>, env: Option<ObjRef>) -> Step {
        let base = self.vstack.len();
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
    pub(super) fn plain_prim(&self, id: crate::value::PrimId) -> bool {
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
    /// stack. `Ok(None)` means it was a plain primitive and its result is
    /// on the stack: carry on in this frame. `Ok(Some)` is the step to take
    /// instead, with this frame already pushed if the call was not a tail
    /// call. Shared by T1 and T2.
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
    ) -> VResult<Option<Step>> {
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
            if tail {
                self.vstack.truncate(base);
                return Ok(Some(Step::Return(v)));
            }
            self.vstack.push(v);
            return Ok(None);
        }
        // A compiled closure with a plain lambda list: the frame is built
        // straight from the operand stack, no argument vector in between.
        // Everything else -- T0 closures, optional and named parameters,
        // arity errors, the special primitives -- goes through the one
        // `apply`, which also compiles on first call.
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
            slots.extend(c.defines.iter().map(|&d| (d, Value::Unspecified)));
            if tail {
                self.vstack.truncate(base);
            } else {
                self.vstack.truncate(start);
                self.push(Frame::Code { code: Rc::clone(code), pc, env, base });
            }
            let new_env = self.engine.env(cenv, slots);
            return Ok(Some(self.enter_code(c, Some(new_env))));
        }
        let args = self.vstack.split_off(start + 1);
        if tail {
            self.vstack.truncate(base);
        } else {
            self.vstack.truncate(start);
            self.push(Frame::Code { code: Rc::clone(code), pc, env, base });
        }
        self.apply(callee, args).map(Some)
    }

    pub(super) fn run_code(
        &mut self,
        code: Rc<crate::code::Code>,
        mut pc: usize,
        mut env: Option<ObjRef>,
        base: usize,
    ) -> VResult<Step> {
        // T2, if this code has it or has earned it.
        if let Some(f) = self.native_for(&code) {
            return self.run_native(code, f, pc, env, base);
        }
        let mut ticks: u32 = 0;
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
        loop {
            ticks += 1;
            if ticks >= 1024 {
                ticks = 0;
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
                Op::Jump(t) => pc = t as usize,
                Op::JumpIfFalse(t) => {
                    let v = self.vstack.pop().expect("a test value");
                    if !v.is_truthy() {
                        pc = t as usize;
                    }
                }
                Op::Call { argc, form } | Op::TailCall { argc, form } => {
                    let tail = matches!(op, Op::TailCall { .. });
                    match self.do_call(&code, pc, env, base, argc, form, tail) {
                        Ok(None) => {}
                        Ok(Some(step)) => return Ok(step),
                        Err(e) => fail!(e),
                    }
                }
                Op::Return => {
                    let v = self.vstack.pop().expect("a value to return");
                    self.vstack.truncate(base);
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
                        (n, self.vstack.get(start + i).copied().unwrap_or(Value::Unspecified))
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
