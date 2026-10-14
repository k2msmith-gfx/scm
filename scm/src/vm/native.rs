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

/// How many times a `Code` runs before T2 is asked to compile it. Each
/// compile is milliseconds, so a function has to earn it.
pub const HOT: u32 = 400;

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
    /// Direct reads, refreshed by `rt_refresh` at entry and after any
    /// helper that could move them: the globals buffer (an absent global
    /// is `None`, tag 12), and the slot buffers of the innermost two
    /// frames (null when there is no such frame). A slot is `(SymId,
    /// Value)`: the name at 0, the value at 8, 24 bytes each -- pinned by
    /// `layouts_native_code_relies_on` below.
    pub globals: *const Option<Value>,
    pub globals_len: usize,
    pub slots0: *mut (crate::value::SymId, Value),
    pub slots1: *mut (crate::value::SymId, Value),
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

    /// How many runs a `Code` needs before T2 compiles it. `HOT` by
    /// default; a test sets 1, a batch job that cannot amortise a compile
    /// sets more.
    pub fn set_native_threshold(&mut self, n: u32) {
        self.native_hot = n;
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
        let out = if hits >= self.native_hot && !code.native_declined.get() {
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
                globals: std::ptr::null(),
                globals_len: 0,
                slots0: std::ptr::null_mut(),
                slots1: std::ptr::null_mut(),
            };
            refresh(&mut st);
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

/// Recompute the direct-read pointers from the VM's current state.
fn refresh(st: &mut NativeState) {
    // Safety: `st.vm` is the live VM for the duration of the native run.
    let vm = unsafe { &mut *st.vm };
    st.globals = vm.globals.as_ptr();
    st.globals_len = vm.globals.len();
    let mut cur = env_of(st.env);
    let mut out = [std::ptr::null_mut(); 2];
    for slot in out.iter_mut() {
        let Some(r) = cur else { break };
        if let Obj::Env { parent, slots } = vm.engine.obj_mut(r) {
            *slot = slots.as_mut_ptr();
            cur = *parent;
        } else {
            break;
        }
    }
    st.slots0 = out[0];
    st.slots1 = out[1];
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

/// `LocalDef`: as `rt_local`, but `Undefined` is the unbound-variable error.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_local_def(st: *mut NativeState, depth: u32, idx: u32, sym: u32, out: *mut Value) -> u32 {
    unsafe {
        let r = rt_local(st, depth, idx, out);
        if r != 0 {
            return r;
        }
        if *out == Value::Undefined {
            let vm = vm!(st);
            let e = vm.err(format!("unbound variable: {}", vm.engine.interner.sym_name(crate::value::SymId(sym))));
            return fail(st, e);
        }
        0
    }
}

/// `SetLocalDef`: `set!` of a define's slot before the define ran is an error.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_set_local_def(st: *mut NativeState, depth: u32, idx: u32, sym: u32, v: *const Value) -> u32 {
    unsafe {
        let mut cur = Value::Undefined;
        let r = rt_local(st, depth, idx, &mut cur);
        if r != 0 {
            return r;
        }
        if cur == Value::Undefined {
            let vm = vm!(st);
            let e = vm.err(format!("set!: unbound variable: {}", vm.engine.interner.sym_name(crate::value::SymId(sym))));
            return fail(st, e);
        }
        rt_set_local(st, depth, idx, v)
    }
}

/// `Slot`: a permanent frame's slot by address.
///
/// # Safety
/// As [`rt_local`].
pub unsafe extern "C" fn rt_slot(st: *mut NativeState, frame: u32, idx: u32, out: *mut Value) -> u32 {
    unsafe {
        let vm = vm!(st);
        if let Obj::Env { slots, .. } = vm.engine.obj(ObjRef(frame)) {
            *out = slots[idx as usize].1;
        }
        0
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
        let r = vm.bind(sym, *v, define != 0, None);
        refresh(&mut *st);
        match r {
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
        let mut slots = vm.engine.slots(names.len());
        slots.extend(names.iter().enumerate().map(|(i, &n)| (n, vals.get(i).copied().unwrap_or(Value::Undefined))));
        let new_env = vm.engine.env(env_of((*st).env), slots);
        (*st).env = bits_of(Some(new_env));
        refresh(&mut *st);
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
        refresh(&mut *st);
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
                vm.current_call = Value::Unspecified;
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
        // The wrapper holds a strong count for the duration; borrow it to
        // clone one for the frame, and hand it back untouched.
        let borrowed = Rc::from_raw((*st).code);
        let code = Rc::clone(&borrowed);
        std::mem::forget(borrowed);
        vm.stack.push(Frame::Code { code, pc: pc as usize, env: env_of((*st).env), base: (*st).base });
        let r = vm.safepoint(&Step::Run);
        vm.stack.pop();
        match r {
            Ok(()) => 0,
            Err(e) => fail(st, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::SymId;

    /// Native code reads these two layouts directly. Rust does not promise
    /// either, so this test is the promise: if it fails, T2 is reading
    /// garbage and `rt_global`/`rt_local` must go back to being the only
    /// path.
    #[test]
    fn layouts_native_code_relies_on() {
        assert_eq!(std::mem::size_of::<Option<Value>>(), 16);
        let none: [u8; 16] = unsafe { std::mem::transmute::<Option<Value>, [u8; 16]>(None) };
        assert_eq!(none[0], 12);
        let some: [u8; 16] = unsafe { std::mem::transmute(Some(Value::Fixnum(3))) };
        assert_eq!((some[0], some[8]), (0, 3));
        assert_eq!(std::mem::size_of::<(SymId, Value)>(), 24);
        let slot: [u8; 24] = unsafe { std::mem::transmute((SymId(7), Value::Fixnum(9))) };
        assert_eq!((slot[0], slot[8], slot[16]), (7, 0, 9));
    }
}
