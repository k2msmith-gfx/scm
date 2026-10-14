//! T2: T1's `Code` compiled to native code (plan §5, S13).
//!
//! Op for op, with the operand stack at fixed offsets: every pc has a
//! statically known depth, so a cell is `stack[base + k]` and no stack
//! pointer exists at run time. Heap work goes through the helpers in
//! `scm::vm::native`, called directly with their addresses baked in; a
//! call that is not a plain primitive exits to the wrapper, which performs
//! it and re-enters at the resume pc -- the function is a `switch` on pc
//! at its entry. Fixnum `+ - * < > <= >= =` on the original primitives are
//! inline behind a guard on the callee value itself, which is what the
//! global cell held a moment ago, so a redefinition at the REPL is seen on
//! the next execution with nothing to invalidate.

use std::ffi::CString;
use std::mem;
use std::rc::Rc;

use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::{Context, ContextRef};
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::TargetMachine;
use inkwell::types::{BasicMetadataTypeEnum, IntType, PointerType, StructType};
use inkwell::values::{FunctionValue, IntValue, PointerValue};

use llvm_sys::orc2::LLVMOrcThreadSafeContextRef;
use llvm_sys::orc2::lljit::{LLVMOrcLLJITAddLLVMIRModule, LLVMOrcLLJITGetMainJITDylib, LLVMOrcLLJITLookup, LLVMOrcLLJITRef};
use llvm_sys::orc2::{LLVMOrcCreateNewThreadSafeContextFromLLVMContext, LLVMOrcCreateNewThreadSafeModule};

use scm::code::{Code, Op};
use scm::vm::native::{self, NativeFn, NativeState};
use scm::{Value, Vm};

use crate::jit;

/// Install T2 into a VM: from now on, a `Code` that runs `native::HOT`
/// times is compiled and runs natively.
pub fn install(vm: &mut Vm) {
    let mut t2 = T2::new();
    vm.install_native_compiler(Box::new(move |vm, code| t2.compile(vm, code)));
}

struct T2 {
    jit: LLVMOrcLLJITRef,
    tsctx: LLVMOrcThreadSafeContextRef,
    raw_ctx: *mut llvm_sys::LLVMContext,
    tm: TargetMachine,
    counter: u64,
}

// One JIT, one thread: the VM is single-threaded and so is this.
impl T2 {
    fn new() -> Self {
        jit::init();
        let owned = Context::create();
        let raw_ctx = owned.raw();
        mem::forget(owned);
        let (jit, _owner) = jit::create_lljit().expect("LLJIT");
        mem::forget(_owner);
        let tsctx = unsafe { LLVMOrcCreateNewThreadSafeContextFromLLVMContext(raw_ctx) };
        let tm = jit::target_machine().expect("target machine");
        T2 { jit, tsctx, raw_ctx, tm, counter: 0 }
    }

    fn compile(&mut self, vm: &mut Vm, code: &Rc<Code>) -> Option<NativeFn> {
        self.counter += 1;
        let name = format!("t2_{}", self.counter);
        let ctx = unsafe { ContextRef::new(self.raw_ctx) };
        let module = ctx.create_module(&name);
        let depths = depths(code)?;
        code.max_depth.set(depths.iter().flatten().copied().max().unwrap_or(0) as u32 + 2);
        let prims = Prims::of(vm);
        let mut g = Gen::new(ctx, &module, code, &depths, &prims);
        g.function(&name).ok()?;
        if let Err(e) = module.verify() {
            eprintln!("t2: module for {name} does not verify: {e}");
            return None;
        }
        // No IR passes: the code is straight-line loads and stores with
        // every helper call opaque, and measured, O1 bought nothing the
        // backend's own register allocation did not while costing most of
        // the compile time.
        if std::env::var("SCM_T2_PASSES").is_ok() {
            module.run_passes("default<O1>", &self.tm, PassBuilderOptions::create()).ok()?;
        }
        unsafe {
            let raw = module.as_mut_ptr();
            mem::forget(module);
            let tsm = LLVMOrcCreateNewThreadSafeModule(raw, self.tsctx);
            let jd = LLVMOrcLLJITGetMainJITDylib(self.jit);
            let err = LLVMOrcLLJITAddLLVMIRModule(self.jit, jd, tsm);
            if !err.is_null() {
                eprintln!("t2: {}", jit::message(err));
                return None;
            }
            let cname = CString::new(name).unwrap();
            let mut addr: u64 = 0;
            let err = LLVMOrcLLJITLookup(self.jit, &mut addr, cname.as_ptr());
            if !err.is_null() || addr == 0 {
                if !err.is_null() {
                    eprintln!("t2: {}", jit::message(err));
                }
                return None;
            }
            Some(mem::transmute::<usize, NativeFn>(addr as usize))
        }
    }
}

/// The operand-stack depth at each pc, or `None` where no path reaches.
/// Structured code gives each pc one depth; a disagreement means the
/// compiler and this analysis differ on an op's effect, and T2 declines.
fn depths(code: &Code) -> Option<Vec<Option<i32>>> {
    let n = code.ops.len();
    let mut out: Vec<Option<i32>> = vec![None; n];
    let mut work = vec![(0usize, 0i32)];
    while let Some((pc, d)) = work.pop() {
        if pc >= n {
            return None;
        }
        match out[pc] {
            Some(x) if x == d => continue,
            Some(_) => return None,
            None => out[pc] = Some(d),
        }
        let (next, after) = match code.ops[pc] {
            Op::Const(_) | Op::Local { .. } | Op::Global(_) | Op::Unspecified | Op::MakeClosure(_) => (true, d + 1),
            Op::SetLocal { .. } | Op::SetGlobal(_) | Op::DefineGlobal(_) | Op::Pop => (true, d - 1),
            Op::Jump(t) => {
                work.push((t as usize, d));
                (false, d)
            }
            Op::JumpIfFalse(t) => {
                work.push((t as usize, d - 1));
                (true, d - 1)
            }
            Op::Call { argc, .. } => (true, d - argc as i32),
            Op::TailCall { .. } | Op::Return => (false, d),
            Op::PushEnv { ninit, .. } => (true, d - ninit as i32),
            Op::PopEnv | Op::InsertBelow(_) => (true, d),
            Op::MakeVector(k) => (true, d - k as i32 + 1),
            Op::Try(_) | Op::Interp(_) => (true, d + 1),
            Op::Parameterize { pairs, .. } => (true, d - 2 * pairs as i32 + 1),
        };
        if after < 0 {
            return None;
        }
        if next {
            work.push((pc + 1, after));
        }
    }
    Some(out)
}

/// The primitives with inline fixnum paths, if the globals still hold them
/// at compile time. The guard is on the value at run time; this only
/// decides which guards to emit.
#[derive(Default)]
struct Prims {
    add: Option<u32>,
    sub: Option<u32>,
    mul: Option<u32>,
    lt: Option<u32>,
    gt: Option<u32>,
    le: Option<u32>,
    ge: Option<u32>,
    eq: Option<u32>,
}

impl Prims {
    fn of(vm: &Vm) -> Self {
        let id = |n: &str| match vm.engine.interner.lookup(n).and_then(|s| vm.global(s)) {
            Some(Value::Primitive(p)) => Some(p.0),
            _ => None,
        };
        Prims { add: id("+"), sub: id("-"), mul: id("*"), lt: id("<"), gt: id(">"), le: id("<="), ge: id(">="), eq: id("=") }
    }
}

type R<T> = Result<T, String>;

struct Gen<'c, 'a> {
    ctx: ContextRef<'c>,
    module: &'a Module<'c>,
    b: Builder<'c>,
    code: &'a Code,
    depths: &'a [Option<i32>],
    prims: &'a Prims,
    i8: IntType<'c>,
    i16: IntType<'c>,
    i32: IntType<'c>,
    i64: IntType<'c>,
    ptr: PointerType<'c>,
    value_t: StructType<'c>,
    state_t: StructType<'c>,
    f: Option<FunctionValue<'c>>,
    st: Option<PointerValue<'c>>,
    stack: Option<PointerValue<'c>>,
    base: Option<IntValue<'c>>,
    blocks: Vec<BasicBlock<'c>>,
}

// NativeState's fields, in order.
const ST_GLOBALS: u32 = 12;
const ST_GLOBALS_LEN: u32 = 13;
const ST_SLOTS0: u32 = 14;
const ST_SLOTS1: u32 = 15;
const ST_CODE: u32 = 1;
const ST_STACK: u32 = 2;
const ST_LEN: u32 = 3;
const ST_BASE: u32 = 4;
const ST_PC: u32 = 6;
const ST_ARGC: u32 = 7;
const ST_FORM: u32 = 8;
const ST_TAIL: u32 = 9;
const ST_BLOCK: u32 = 10;
const ST_PAIRS: u32 = 11;

impl<'c, 'a> Gen<'c, 'a> {
    fn new(ctx: ContextRef<'c>, module: &'a Module<'c>, code: &'a Code, depths: &'a [Option<i32>], prims: &'a Prims) -> Self {
        let i8 = ctx.i8_type();
        let i16 = ctx.i16_type();
        let i32 = ctx.i32_type();
        let i64 = ctx.i64_type();
        let ptr = ctx.ptr_type(inkwell::AddressSpace::default());
        let value_t = ctx.struct_type(&[i64.into(), i64.into()], false);
        let state_t = ctx.struct_type(
            &[
                ptr.into(), ptr.into(), ptr.into(), i64.into(), i64.into(), i64.into(),
                i32.into(), i32.into(), i32.into(), i32.into(), i32.into(), i32.into(),
                ptr.into(), i64.into(), ptr.into(), ptr.into(),
            ],
            false,
        );
        let _ = ST_CODE;
        Gen {
            ctx,
            module,
            b: ctx.create_builder(),
            code,
            depths,
            prims,
            i8,
            i16,
            i32,
            i64,
            ptr,
            value_t,
            state_t,
            f: None,
            st: None,
            stack: None,
            base: None,
            blocks: Vec::new(),
        }
    }

    fn e<T>(r: Result<T, impl std::fmt::Display>) -> R<T> {
        r.map_err(|e| e.to_string())
    }

    /// Call a helper through its absolute address.
    fn call_helper(&mut self, name: &'static str, addr: usize, args: &[inkwell::values::BasicMetadataValueEnum<'c>]) -> R<IntValue<'c>> {
        let tys: Vec<BasicMetadataTypeEnum<'c>> = args
            .iter()
            .map(|a| match a {
                inkwell::values::BasicMetadataValueEnum::IntValue(v) => v.get_type().into(),
                inkwell::values::BasicMetadataValueEnum::PointerValue(p) => p.get_type().into(),
                _ => unreachable!("helper arguments are ints and pointers"),
            })
            .collect();
        let fty = self.i32.fn_type(&tys, false);
        let target = self.i64.const_int(addr as u64, false);
        let fptr = Self::e(self.b.build_int_to_ptr(target, self.ptr, name))?;
        let cs = Self::e(self.b.build_indirect_call(fty, fptr, args, "h"))?;
        Ok(cs.try_as_basic_value().basic().ok_or("helper returned nothing")?.into_int_value())
    }

    fn st_field(&mut self, idx: u32) -> R<PointerValue<'c>> {
        Self::e(self.b.build_struct_gep(self.state_t, self.st.unwrap(), idx, "f"))
    }

    fn store_st_i32(&mut self, idx: u32, v: u32) -> R<()> {
        let p = self.st_field(idx)?;
        Self::e(self.b.build_store(p, self.i32.const_int(v as u64, false)))?;
        Ok(())
    }

    fn store_len(&mut self, depth: i32) -> R<()> {
        let p = self.st_field(ST_LEN)?;
        let len = Self::e(self.b.build_int_add(self.base.unwrap(), self.i64.const_int(depth as u64, false), "len"))?;
        Self::e(self.b.build_store(p, len))?;
        Ok(())
    }

    /// `&stack[base + k]`.
    fn cell(&mut self, k: i32) -> R<PointerValue<'c>> {
        let idx = Self::e(self.b.build_int_add(self.base.unwrap(), self.i64.const_int(k as u64, false), "i"))?;
        unsafe { Self::e(self.b.build_in_bounds_gep(self.value_t, self.stack.unwrap(), &[idx], "c")) }
    }

    fn word(&mut self, cell: PointerValue<'c>, which: u32) -> R<IntValue<'c>> {
        let p = Self::e(self.b.build_struct_gep(self.value_t, cell, which, "w"))?;
        Ok(Self::e(self.b.build_load(self.i64, p, "v"))?.into_int_value())
    }

    fn store_words(&mut self, cell: PointerValue<'c>, w0: IntValue<'c>, w1: IntValue<'c>) -> R<()> {
        let p0 = Self::e(self.b.build_struct_gep(self.value_t, cell, 0, "w0"))?;
        let p1 = Self::e(self.b.build_struct_gep(self.value_t, cell, 1, "w1"))?;
        Self::e(self.b.build_store(p0, w0))?;
        Self::e(self.b.build_store(p1, w1))?;
        Ok(())
    }

    fn const_value(&self, v: Value) -> (IntValue<'c>, IntValue<'c>) {
        let words: [u64; 2] = unsafe { mem::transmute(v) };
        (self.i64.const_int(words[0], false), self.i64.const_int(words[1], false))
    }

    /// `&slots[idx].value` of the frame `depth` (0 or 1) levels up, from
    /// the refreshed pointers. A slot is 24 bytes; the value is at 8.
    fn slot_ptr(&mut self, depth: u16, idx: u16) -> R<PointerValue<'c>> {
        let field = if depth == 0 { ST_SLOTS0 } else { ST_SLOTS1 };
        let p = self.st_field(field)?;
        let slots = Self::e(self.b.build_load(self.ptr, p, "slots"))?.into_pointer_value();
        let off = self.i64.const_int(idx as u64 * 24 + 8, false);
        unsafe { Self::e(self.b.build_in_bounds_gep(self.i8, slots, &[off], "slot")) }
    }

    fn exit(&mut self, status: u32, pc_after: usize, depth: i32) -> R<()> {
        self.store_len(depth)?;
        self.store_st_i32(ST_PC, pc_after as u32)?;
        Self::e(self.b.build_return(Some(&self.i32.const_int(status as u64, false))))?;
        Ok(())
    }

    /// `status != 0` -> exit ERROR; else fall through into `ok`.
    fn check(&mut self, status: IntValue<'c>, pc_after: usize, depth: i32) -> R<()> {
        let f = self.f.unwrap();
        let ok = self.ctx.append_basic_block(f, "ok");
        let bad = self.ctx.append_basic_block(f, "bad");
        let z = Self::e(self.b.build_int_compare(IntPredicate::EQ, status, self.i32.const_zero(), "z"))?;
        Self::e(self.b.build_conditional_branch(z, ok, bad))?;
        self.b.position_at_end(bad);
        self.exit(native::ERROR, pc_after, depth)?;
        self.b.position_at_end(ok);
        Ok(())
    }

    fn function(&mut self, name: &str) -> R<()> {
        let fty = self.i32.fn_type(&[self.ptr.into(), self.i32.into()], false);
        let f = self.module.add_function(name, fty, None);
        self.f = Some(f);
        let entry = self.ctx.append_basic_block(f, "entry");
        self.b.position_at_end(entry);
        let st = f.get_nth_param(0).unwrap().into_pointer_value();
        self.st = Some(st);
        let p = self.st_field(ST_STACK)?;
        self.stack = Some(Self::e(self.b.build_load(self.ptr, p, "stack"))?.into_pointer_value());
        let p = self.st_field(ST_BASE)?;
        self.base = Some(Self::e(self.b.build_load(self.i64, p, "base"))?.into_int_value());
        let n = self.code.ops.len();
        self.blocks = (0..n).map(|i| self.ctx.append_basic_block(f, &format!("op{i}"))).collect();
        // The resume points: pc 0, and the pc after every op that exits
        // and comes back.
        let mut cases: Vec<(IntValue<'c>, BasicBlock<'c>)> = vec![(self.i32.const_zero(), self.blocks[0])];
        for (i, op) in self.code.ops.iter().enumerate() {
            if matches!(op, Op::Call { .. } | Op::Try(_) | Op::Parameterize { .. } | Op::Interp(_)) && i + 1 < n {
                cases.push((self.i32.const_int(i as u64 + 1, false), self.blocks[i + 1]));
            }
        }
        let dead = self.ctx.append_basic_block(f, "dead");
        let pc = f.get_nth_param(1).unwrap().into_int_value();
        Self::e(self.b.build_switch(pc, dead, &cases))?;
        self.b.position_at_end(dead);
        Self::e(self.b.build_unreachable())?;
        for i in 0..n {
            self.b.position_at_end(self.blocks[i]);
            match self.depths[i] {
                None => {
                    Self::e(self.b.build_unreachable())?;
                }
                Some(d) => self.op(i, d)?,
            }
        }
        Ok(())
    }

    fn fallthrough(&mut self, i: usize) -> R<()> {
        if i + 1 < self.blocks.len() {
            Self::e(self.b.build_unconditional_branch(self.blocks[i + 1]))?;
        } else {
            Self::e(self.b.build_unreachable())?;
        }
        Ok(())
    }

    fn st_ptr(&self) -> inkwell::values::BasicMetadataValueEnum<'c> {
        self.st.unwrap().into()
    }

    fn op(&mut self, i: usize, d: i32) -> R<()> {
        let after = i + 1;
        let c32 = |g: &Self, v: u32| -> inkwell::values::BasicMetadataValueEnum<'c> { g.i32.const_int(v as u64, false).into() };
        match self.code.ops[i] {
            Op::Const(k) => {
                let (w0, w1) = self.const_value(self.code.consts[k as usize]);
                let cell = self.cell(d)?;
                self.store_words(cell, w0, w1)?;
                self.fallthrough(i)
            }
            Op::Unspecified => {
                let (w0, w1) = self.const_value(Value::Unspecified);
                let cell = self.cell(d)?;
                self.store_words(cell, w0, w1)?;
                self.fallthrough(i)
            }
            Op::Pop | Op::PopEnv if matches!(self.code.ops[i], Op::Pop) => self.fallthrough(i),
            Op::Jump(t) => {
                if (t as usize) <= i {
                    // A backward jump is a loop: the safepoint lives here.
                    self.store_len(d)?;
                    let s = self.call_helper("rt_safepoint", native::rt_safepoint as *const () as usize, &[self.st_ptr(), c32(self, after as u32)])?;
                    self.check(s, after, d)?;
                }
                Self::e(self.b.build_unconditional_branch(self.blocks[t as usize]))?;
                Ok(())
            }
            Op::JumpIfFalse(t) => {
                let cell = self.cell(d - 1)?;
                let w0 = self.word(cell, 0)?;
                let low = Self::e(self.b.build_int_truncate(w0, self.i16, "tagb"))?;
                let is_false = Self::e(self.b.build_int_compare(IntPredicate::EQ, low, self.i16.const_int(2, false), "f"))?;
                if (t as usize) <= i {
                    self.store_len(d - 1)?;
                    let s = self.call_helper("rt_safepoint", native::rt_safepoint as *const () as usize, &[self.st_ptr(), c32(self, after as u32)])?;
                    self.check(s, after, d - 1)?;
                }
                let next = if after < self.blocks.len() { self.blocks[after] } else { self.blocks[t as usize] };
                Self::e(self.b.build_conditional_branch(is_false, self.blocks[t as usize], next))?;
                Ok(())
            }
            Op::Local { depth, idx } if depth <= 1 => {
                // The innermost two frames are read directly.
                let slot = self.slot_ptr(depth, idx)?;
                let (w0, w1) = (self.word(slot, 0)?, self.word(slot, 1)?);
                let out = self.cell(d)?;
                self.store_words(out, w0, w1)?;
                self.fallthrough(i)
            }
            Op::Local { depth, idx } => {
                let out = self.cell(d)?;
                let s = self.call_helper("rt_local", native::rt_local as *const () as usize, &[self.st_ptr(), c32(self, depth as u32), c32(self, idx as u32), out.into()])?;
                self.check(s, after, d)?;
                self.fallthrough(i)
            }
            Op::SetLocal { depth, idx } if depth <= 1 => {
                let v = self.cell(d - 1)?;
                let (w0, w1) = (self.word(v, 0)?, self.word(v, 1)?);
                let slot = self.slot_ptr(depth, idx)?;
                self.store_words(slot, w0, w1)?;
                self.fallthrough(i)
            }
            Op::SetLocal { depth, idx } => {
                let v = self.cell(d - 1)?;
                let s = self.call_helper("rt_set_local", native::rt_set_local as *const () as usize, &[self.st_ptr(), c32(self, depth as u32), c32(self, idx as u32), v.into()])?;
                self.check(s, after, d - 1)?;
                self.fallthrough(i)
            }
            Op::Global(g) => {
                // Direct: the cell at the symbol's index, if the table
                // reaches it and it is bound; otherwise the helper, which
                // has the resolved-name fallback and the error.
                let f = self.f.unwrap();
                let sym = self.code.globals[g as usize].0.0 as u64;
                let out = self.cell(d)?;
                let gp = self.st_field(ST_GLOBALS)?;
                let globals = Self::e(self.b.build_load(self.ptr, gp, "globals"))?.into_pointer_value();
                let lp = self.st_field(ST_GLOBALS_LEN)?;
                let len = Self::e(self.b.build_load(self.i64, lp, "glen"))?.into_int_value();
                let in_range = Self::e(self.b.build_int_compare(IntPredicate::UGT, len, self.i64.const_int(sym, false), "inr"))?;
                let direct = self.ctx.append_basic_block(f, "gdirect");
                let slow = self.ctx.append_basic_block(f, "gslow");
                let done = self.ctx.append_basic_block(f, "gdone");
                Self::e(self.b.build_conditional_branch(in_range, direct, slow))?;
                self.b.position_at_end(direct);
                let cellp = unsafe { Self::e(self.b.build_in_bounds_gep(self.value_t, globals, &[self.i64.const_int(sym, false)], "gc"))? };
                let w0 = self.word(cellp, 0)?;
                let tag = Self::e(self.b.build_int_truncate(w0, self.i8, "gt"))?;
                let bound = Self::e(self.b.build_int_compare(IntPredicate::NE, tag, self.i8.const_int(11, false), "bound"))?;
                let hit = self.ctx.append_basic_block(f, "ghit");
                Self::e(self.b.build_conditional_branch(bound, hit, slow))?;
                self.b.position_at_end(hit);
                let w1 = self.word(cellp, 1)?;
                self.store_words(out, w0, w1)?;
                Self::e(self.b.build_unconditional_branch(done))?;
                self.b.position_at_end(slow);
                let s = self.call_helper("rt_global", native::rt_global as *const () as usize, &[self.st_ptr(), c32(self, g), out.into()])?;
                self.check(s, after, d)?;
                Self::e(self.b.build_unconditional_branch(done))?;
                self.b.position_at_end(done);
                self.fallthrough(i)
            }
            Op::SetGlobal(g) | Op::DefineGlobal(g) => {
                let define = matches!(self.code.ops[i], Op::DefineGlobal(_)) as u32;
                let v = self.cell(d - 1)?;
                let s = self.call_helper("rt_set_global", native::rt_set_global as *const () as usize, &[self.st_ptr(), c32(self, g), v.into(), c32(self, define)])?;
                self.check(s, after, d - 1)?;
                self.fallthrough(i)
            }
            Op::MakeClosure(l) => {
                let out = self.cell(d)?;
                let s = self.call_helper("rt_make_closure", native::rt_make_closure as *const () as usize, &[self.st_ptr(), c32(self, l), out.into()])?;
                self.check(s, after, d)?;
                self.fallthrough(i)
            }
            Op::PushEnv { names, ninit } => {
                let vals = self.cell(d - ninit as i32)?;
                let s = self.call_helper("rt_push_env", native::rt_push_env as *const () as usize, &[self.st_ptr(), c32(self, names), c32(self, ninit), vals.into()])?;
                self.check(s, after, d - ninit as i32)?;
                self.fallthrough(i)
            }
            Op::PopEnv => {
                let s = self.call_helper("rt_pop_env", native::rt_pop_env as *const () as usize, &[self.st_ptr()])?;
                self.check(s, after, d)?;
                self.fallthrough(i)
            }
            Op::InsertBelow(n) => {
                // Rotate the top cell down below the `n` beneath it.
                let top = self.cell(d - 1)?;
                let (t0, t1) = (self.word(top, 0)?, self.word(top, 1)?);
                for k in 0..n as i32 {
                    let from = self.cell(d - 2 - k)?;
                    let to = self.cell(d - 1 - k)?;
                    let (a, b) = (self.word(from, 0)?, self.word(from, 1)?);
                    self.store_words(to, a, b)?;
                }
                let dest = self.cell(d - 1 - n as i32)?;
                self.store_words(dest, t0, t1)?;
                self.fallthrough(i)
            }
            Op::MakeVector(n) => {
                let items = self.cell(d - n as i32)?;
                let s = self.call_helper("rt_make_vector", native::rt_make_vector as *const () as usize, &[self.st_ptr(), c32(self, n), items.into(), items.into()])?;
                self.check(s, after, d - n as i32)?;
                self.fallthrough(i)
            }
            Op::Call { argc, form } => {
                let callee = self.cell(d - argc as i32 - 1)?;
                let f = self.f.unwrap();
                let slow = self.ctx.append_basic_block(f, "slow");
                let done = self.ctx.append_basic_block(f, "done");
                if argc == 2 {
                    self.fixnum_fast_paths(callee, d, slow, done)?;
                } else {
                    Self::e(self.b.build_unconditional_branch(slow))?;
                }
                self.b.position_at_end(slow);
                let s = self.call_helper("rt_call_prim", native::rt_call_prim as *const () as usize, &[self.st_ptr(), c32(self, argc), c32(self, form), callee.into()])?;
                // 0: handled, result in the callee cell. 1: exit CALL. 2: error.
                let handled = self.ctx.append_basic_block(f, "handled");
                let not = self.ctx.append_basic_block(f, "notprim");
                let bad = self.ctx.append_basic_block(f, "bad");
                let sw = [(self.i32.const_zero(), handled), (self.i32.const_int(1, false), not)];
                Self::e(self.b.build_switch(s, bad, &sw))?;
                self.b.position_at_end(bad);
                self.exit(native::ERROR, after, d)?;
                self.b.position_at_end(not);
                self.store_st_i32(ST_ARGC, argc)?;
                self.store_st_i32(ST_FORM, form)?;
                self.store_st_i32(ST_TAIL, 0)?;
                self.exit(native::CALL, after, d)?;
                self.b.position_at_end(handled);
                Self::e(self.b.build_unconditional_branch(done))?;
                self.b.position_at_end(done);
                self.fallthrough(i)
            }
            Op::TailCall { argc, form } => {
                self.store_st_i32(ST_ARGC, argc)?;
                self.store_st_i32(ST_FORM, form)?;
                self.store_st_i32(ST_TAIL, 1)?;
                self.exit(native::CALL, after, d)
            }
            Op::Return => self.exit(native::RETURN, after, d),
            Op::Try(bk) => {
                self.store_st_i32(ST_BLOCK, bk)?;
                self.exit(native::TRY, after, d)
            }
            Op::Parameterize { pairs, block } => {
                self.store_st_i32(ST_BLOCK, block)?;
                self.store_st_i32(ST_PAIRS, pairs)?;
                self.exit(native::PARAM, after, d)
            }
            Op::Interp(fm) => {
                self.store_st_i32(ST_FORM, fm)?;
                self.exit(native::INTERP, after, d)
            }
            Op::Pop => unreachable!("handled above"),
        }
    }

    /// Guards: the callee is `Primitive(id)` for one of the known
    /// arithmetic primitives and both arguments are fixnums. Then the
    /// operation is a few instructions, the result replaces the callee,
    /// and control goes to `done` -- otherwise to `slow`.
    fn fixnum_fast_paths(&mut self, callee: PointerValue<'c>, d: i32, slow: BasicBlock<'c>, done: BasicBlock<'c>) -> R<()> {
        let f = self.f.unwrap();
        let w0 = self.word(callee, 0)?;
        let tag = Self::e(self.b.build_int_truncate(w0, self.i8, "tag"))?;
        let is_prim = Self::e(self.b.build_int_compare(IntPredicate::EQ, tag, self.i8.const_int(6, false), "isprim"))?;
        let id64 = Self::e(self.b.build_right_shift(w0, self.i64.const_int(32, false), false, "idw"))?;
        let id = Self::e(self.b.build_int_truncate(id64, self.i32, "id"))?;
        let a = self.cell(d - 2)?;
        let bcell = self.cell(d - 1)?;
        let (a0, a1) = (self.word(a, 0)?, self.word(a, 1)?);
        let (b0, b1) = (self.word(bcell, 0)?, self.word(bcell, 1)?);
        let ta = Self::e(self.b.build_int_truncate(a0, self.i8, "ta"))?;
        let tb = Self::e(self.b.build_int_truncate(b0, self.i8, "tb"))?;
        let fa = Self::e(self.b.build_int_compare(IntPredicate::EQ, ta, self.i8.const_zero(), "fa"))?;
        let fb = Self::e(self.b.build_int_compare(IntPredicate::EQ, tb, self.i8.const_zero(), "fb"))?;
        let both = Self::e(self.b.build_and(fa, fb, "fix"))?;
        let guard = Self::e(self.b.build_and(is_prim, both, "g"))?;
        let dispatch = self.ctx.append_basic_block(f, "disp");
        Self::e(self.b.build_conditional_branch(guard, dispatch, slow))?;
        self.b.position_at_end(dispatch);
        let mut cases: Vec<(IntValue<'c>, BasicBlock<'c>)> = Vec::new();
        let arith: [(Option<u32>, &str, &str); 3] = [(self.prims.add, "llvm.sadd.with.overflow.i64", "add"), (self.prims.sub, "llvm.ssub.with.overflow.i64", "sub"), (self.prims.mul, "llvm.smul.with.overflow.i64", "mul")];
        for (pid, intrinsic, nm) in arith {
            let Some(pid) = pid else { continue };
            let blk = self.ctx.append_basic_block(f, nm);
            cases.push((self.i32.const_int(pid as u64, false), blk));
            self.b.position_at_end(blk);
            let res_t = self.ctx.struct_type(&[self.i64.into(), self.ctx.bool_type().into()], false);
            let intr = self.module.get_function(intrinsic).unwrap_or_else(|| {
                self.module.add_function(intrinsic, res_t.fn_type(&[self.i64.into(), self.i64.into()], false), None)
            });
            let cs = Self::e(self.b.build_call(intr, &[a1.into(), b1.into()], "r"))?;
            let agg = cs.try_as_basic_value().basic().ok_or("intrinsic")?.into_struct_value();
            let val = Self::e(self.b.build_extract_value(agg, 0, "v"))?.into_int_value();
            let ovf = Self::e(self.b.build_extract_value(agg, 1, "o"))?.into_int_value();
            let okb = self.ctx.append_basic_block(f, "fok");
            Self::e(self.b.build_conditional_branch(ovf, slow, okb))?;
            self.b.position_at_end(okb);
            self.store_words(callee, self.i64.const_zero(), val)?;
            Self::e(self.b.build_unconditional_branch(done))?;
        }
        let cmps: [(Option<u32>, IntPredicate, &str); 5] = [
            (self.prims.lt, IntPredicate::SLT, "lt"),
            (self.prims.gt, IntPredicate::SGT, "gt"),
            (self.prims.le, IntPredicate::SLE, "le"),
            (self.prims.ge, IntPredicate::SGE, "ge"),
            (self.prims.eq, IntPredicate::EQ, "eq"),
        ];
        for (pid, pred, nm) in cmps {
            let Some(pid) = pid else { continue };
            let blk = self.ctx.append_basic_block(f, nm);
            cases.push((self.i32.const_int(pid as u64, false), blk));
            self.b.position_at_end(blk);
            let c = Self::e(self.b.build_int_compare(pred, a1, b1, "c"))?;
            // Bool: tag 2 in byte 0, the bool in byte 1.
            let c64 = Self::e(self.b.build_int_z_extend(c, self.i64, "c64"))?;
            let shifted = Self::e(self.b.build_left_shift(c64, self.i64.const_int(8, false), "sh"))?;
            let w0 = Self::e(self.b.build_or(shifted, self.i64.const_int(2, false), "bw0"))?;
            self.store_words(callee, w0, self.i64.const_zero())?;
            Self::e(self.b.build_unconditional_branch(done))?;
        }
        self.b.position_at_end(dispatch);
        Self::e(self.b.build_switch(id, slow, &cases))?;
        Ok(())
    }
}

#[allow(dead_code)]
fn _state_layout_matches() {
    // NativeState is repr(C) with exactly these fields in this order; the
    // struct type above mirrors it. A mismatch would be a silent miscompile,
    // so keep them side by side.
    let _ = mem::size_of::<NativeState>();
}
