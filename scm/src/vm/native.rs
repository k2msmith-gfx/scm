//! The runtime half of T2 (plan §5, S13): what native code compiled from a
//! `Code` calls into, and the wrapper that runs it.
//!
//! Native code **borrows the stack** -- it returns to the Rust loop at every
//! call, as `run_code` does, so proper tail calls, escape continuations,
//! interrupts and the collector are inherited, not re-solved. What it keeps
//! for itself is everything between calls: the operand stack is addressed
//! directly, constants and jumps are instructions, and fixnum arithmetic on
//! the original primitives is inline with a guard on the global cell -- so a
//! redefinition takes effect on the next execution with no invalidation
//! list. Everything that touches the heap is a helper below.
//!
//! The contract is a `NativeState` block passed by pointer and a status
//! word back: `RETURN`, `CALL` (the wrapper performs it, then re-enters at
//! the resume pc), `ERROR` (the helper stored the error), or one of the
//! sub-block exits. The compiled function is resumable at any call site,
//! which is how a frame suspended for a call picks up where it left off.

use std::rc::Rc;

use super::{Frame, Step, VResult, Vm, VmError};
use crate::code::{Code, CodeState};
use crate::heap::Obj;
use crate::value::{ObjRef, Value};

pub const RETURN: u32 = 0;
pub const CALL: u32 = 1;
pub const ERROR: u32 = 2;
pub const TRY: u32 = 3;
pub const PARAM: u32 = 4;
pub const INTERP: u32 = 5;

/// How many times a `Code` runs before T2 is asked to compile it.
pub const HOT: u32 = 50;

/// A compiled `Code`: `fn(state, pc) -> status`.
pub type NativeFn = unsafe extern "C" fn(*mut NativeState, u32) -> u32;

/// The compiler T2 installs: `None` to leave the code on T1.
pub type NativeCompiler = Box<dyn FnMut(&mut Vm, &Rc<Code>) -> Option<NativeFn>>;

#[repr(C)]
pub struct NativeState {
    pub vm: *mut Vm,
    pub code: *const Code,
    /// The operand stack's buffer. Reserved to the code's depth before
    /// entry, so nothing here reallocates it.
    pub stack: *mut Value,
    /// In: the current length. Out: the length when the function exited.
    pub len: usize,
    pub base: usize,
    /// The environment as an index, or -1 for none. In and out.
    pub env: i64,
    /// Out: the pc after the op in progress.
    pub pc: u32,
    pub argc: u32,
    pub form: u32,
    pub tail: u32,
    pub block: u32,
    pub pairs: u32,
}

fn env_of(bits: i64) -> Option<ObjRef> {
    u32::try_from(bits).ok().map(ObjRef)
}

fn bits_of(env: Option<ObjRef>) -> i64 {
    env.map_or(-1, |r| r.0 as i64)
}

impl Vm {
    pub fn install_native_compiler(&mut self, c: NativeCompiler) {
        self.native_compiler = Some(c);
    }

    /// How many `Code`s T2 has compiled.
    pub fn native_units(&self) -> u64 {
        self.native_units
    }

    /// Decide whether this code runs natively: already compiled, or hot
    /// enough to compile now.
    pub(super) fn native_for(&mut self, code: &Rc<Code>) -> Option<NativeFn> {
        if let Some(f) = code.native.get() {
            return Some(f);
        }
        let mut compiler = self.native_compiler.take()?;
        let hits = code.hits.get() + 1;
        code.hits.set(hits);
        let out = if hits >= HOT && !code.native_declined.get() {
            match compiler(self, code) {
                Some(f) => {
                    code.native.set(Some(f));
                    self.native_units += 1;
                    Some(f)
                }
                None => {
                    code.native_declined.set(true);
                    None
                }
            }
        } else {
            None
        };
        self.native_compiler = Some(compiler);
        out
    }

    pub(super) fn run_native(
        &mut self,
        code: Rc<Code>,
        f: NativeFn,
        mut pc: usize,
        mut env: Option<ObjRef>,
        base: usize,
    ) -> VResult<Step> {
        loop {
            self.vstack.reserve(code.max_depth.get() as usize + 8);
            let mut st = NativeState {
                vm: self as *mut Vm,
                code: Rc::as_ptr(&code),
                stack: self.vstack.as_mut_ptr(),
                len: self.vstack.len(),
                base,
                env: bits_of(env),
                pc: pc as u32,
                argc: 0,
                form: 0,
                tail: 0,
                block: 0,
                pairs: 0,
            };
            // Safety: `f` was compiled from `code` by the installed
            // compiler against exactly this state layout, and the helpers
            // it calls take `self` back through `st.vm`.
            let status = unsafe { f(&mut st, pc as u32) };
            // Safety: every value below `st.len` was written by native code
            // as a whole `Value`, and the buffer was reserved for it.
            unsafe { self.vstack.set_len(st.len) };
            env = env_of(st.env);
            pc = st.pc as usize;
            match status {
                RETURN => {
                    let v = self.vstack.pop().expect("a value to return");
                    self.vstack.truncate(base);
                    return Ok(Step::Return(v));
                }
                CALL => {
                    match self.do_call(&code, pc, env, base, st.argc, st.form, st.tail != 0) {
                        Ok(Some(step)) => return Ok(step),
                        Ok(None) => continue,
                        Err(e) => {
                            self.stack.push(Frame::Code { code, pc, env, base });
                            return Err(e);
                        }
                    }
                }
                TRY => {
                    let block = Rc::clone(&code.blocks[st.block as usize]);
                    let param_depth = self.param_bindings.len();
                    self.push(Frame::Code { code, pc, env, base });
                    self.push(Frame::Catch { param_depth });
                    return Ok(self.enter_code(block, env));
                }
                PARAM => {
                    let block = Rc::clone(&code.blocks[st.block as usize]);
                    match self.install_params(st.pairs as usize) {
                        Ok(count) => {
                            self.push(Frame::Code { code, pc, env, base });
                            self.push(Frame::ParamRestore { count });
                            return Ok(self.enter_code(block, env));
                        }
                        Err(e) => {
                            self.stack.push(Frame::Code { code, pc, env, base });
                            return Err(e);
                        }
                    }
                }
                INTERP => {
                    let form = code.forms[st.form as usize];
                    self.push(Frame::Code { code, pc, env, base });
                    return Ok(Step::Eval(form, env));
                }
                _ => {
                    let e = self.native_err.take().unwrap_or_else(|| VmError::Error("native code failed".into()));
                    self.stack.push(Frame::Code { code, pc, env, base });
                    return Err(e);
                }
            }
        }
    }
}

macro_rules! vm {
    ($st:expr) => {
        &mut *(*$st).vm
    };
}

fn fail(st: *mut NativeState, e: VmError) -> u32 {
    unsafe {
        vm!(st).native_err = Some(e);
    }
    ERROR
}

/// `Local { depth, idx }` -> `*out`.
///
/// # Safety
/// `st` is the live state of a native run; `out` is writable.
pub unsafe extern "C" fn rt_local(st: *mut NativeState, depth: u32, idx: u32, out: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        match vm.frame_at(env_of((*st).env), depth as u16) {
            Ok(r) => {
                if let Obj::Env { slots, .. } = vm.engine.obj(r) {
                    *out = slots[idx as usize].1;
                }
                0
            }
            Err(e) => fail(st, e),
        }
    }
}

/// `SetLocal { depth, idx }` <- `*v`.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_set_local(st: *mut NativeState, depth: u32, idx: u32, v: *const Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        match vm.frame_at(env_of((*st).env), depth as u16) {
            Ok(r) => {
                if let Obj::Env { slots, .. } = vm.engine.obj_mut(r) {
                    slots[idx as usize].1 = *v;
                }
                0
            }
            Err(e) => fail(st, e),
        }
    }
}

/// `Global(i)` -> `*out`, with T0's unbound-variable error.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_global(st: *mut NativeState, i: u32, out: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        let (sym, resolved) = (&*(*st).code).globals[i as usize];
        let v = vm
            .global(sym)
            .or_else(|| if resolved != sym { vm.global(resolved) } else { None });
        match v {
            Some(v) => {
                *out = v;
                0
            }
            None => {
                let msg = format!("unbound variable: {}", vm.engine.interner.sym_name(sym));
                let e = vm.err(msg);
                fail(st, e)
            }
        }
    }
}

/// `SetGlobal(i)` / `DefineGlobal(i)` <- `*v`.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_set_global(st: *mut NativeState, i: u32, v: *const Value, define: u32) -> u32 {
    unsafe {
        let vm = vm!(st);
        let (sym, _) = (&*(*st).code).globals[i as usize];
        match vm.bind(sym, *v, define != 0, None) {
            Ok(()) => 0,
            Err(e) => fail(st, e),
        }
    }
}

/// `MakeClosure(i)` -> `*out`, over the current environment.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_make_closure(st: *mut NativeState, i: u32, out: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        let l = &(&*(*st).code).lambdas[i as usize];
        *out = vm.engine.alloc(Obj::Closure {
            name: l.name,
            params: Rc::clone(&l.params),
            opt: Rc::clone(&l.opt),
            named: Rc::clone(&l.named),
            keys: l.keys,
            rest: l.rest,
            body: Rc::clone(&l.body),
            env: env_of((*st).env),
            code: CodeState::Compiled(Rc::clone(&l.code)),
        });
        0
    }
}

/// `PushEnv { names, ninit }`: the `ninit` values at `vals` become the
/// first slots; `st.env` moves to the new frame.
///
/// # Safety
/// As [`rt_local`]; `vals` points at `ninit` live values.
pub unsafe extern "C" fn rt_push_env(st: *mut NativeState, names: u32, ninit: u32, vals: *const Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        let names = &(&*(*st).code).names[names as usize];
        let vals = std::slice::from_raw_parts(vals, ninit as usize);
        let slots: Vec<(crate::value::SymId, Value)> = names
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, vals.get(i).copied().unwrap_or(Value::Unspecified)))
            .collect();
        let new_env = vm.engine.env(env_of((*st).env), slots);
        (*st).env = bits_of(Some(new_env));
        0
    }
}

/// `PopEnv`.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_pop_env(st: *mut NativeState) -> u32 {
    unsafe {
        let vm = vm!(st);
        let Some(r) = env_of((*st).env) else {
            let e = vm.err("corrupt environment");
            return fail(st, e);
        };
        let Obj::Env { parent, .. } = vm.engine.obj(r) else { unreachable!() };
        (*st).env = bits_of(*parent);
        0
    }
}

/// `MakeVector(n)` from the `n` values at `items` -> `*out`.
///
/// # Safety
/// As [`rt_push_env`].
pub unsafe extern "C" fn rt_make_vector(st: *mut NativeState, n: u32, items: *const Value, out: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        let items = std::slice::from_raw_parts(items, n as usize).to_vec();
        *out = vm.engine.vector(items, true);
        0
    }
}

/// A call whose callee is at `*callee` with `argc` arguments after it. If
/// the callee is a plain primitive it is called here, the result replaces
/// the callee, and 0 comes back: the caller drops `argc` cells. Otherwise
/// 1: the caller exits with `CALL` and the wrapper does the rest. `form`
/// names the call for `current_call`.
///
/// # Safety
/// As [`rt_push_env`]; `callee` points at `argc + 1` live values.
pub unsafe extern "C" fn rt_call_prim(st: *mut NativeState, argc: u32, form: u32, callee: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        vm.current_call = (&*(*st).code).forms[form as usize];
        let Value::Primitive(id) = *callee else { return 1 };
        if !vm.plain_prim(id) {
            return 1;
        }
        let f = vm.prims[id.0 as usize];
        let args = std::slice::from_raw_parts(callee.add(1), argc as usize);
        match f(&mut vm.engine, args) {
            Ok(v) => {
                *callee = v;
                0
            }
            Err(e) => fail(st, e),
        }
    }
}

/// The safepoint, at every backward jump. The frame is on the stack for
/// the duration so the collector sees it.
///
/// # Safety
/// As [`rt_local`]; `st.len` and `st.env` must be current.
pub unsafe extern "C" fn rt_safepoint(st: *mut NativeState, pc: u32) -> u32 {
    unsafe {
        let vm = vm!(st);
        vm.vstack.set_len((*st).len);
        let code = Rc::increment_strong_count((*st).code);
        let _ = code;
        let code = Rc::from_raw((*st).code);
        vm.stack.push(Frame::Code { code: Rc::clone(&code), pc: pc as usize, env: env_of((*st).env), base: (*st).base });
        let r = vm.safepoint(&Step::Run);
        vm.stack.pop();
        std::mem::forget(code);
        match r {
            Ok(()) => 0,
            Err(e) => fail(st, e),
        }
    }
}
