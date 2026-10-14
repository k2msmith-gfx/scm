//! Ownership of the compiled code: one ORC LLJIT per compiled leaf.
//!
//! The S0 path, as verified on this machine: inkwell builds the module in a
//! context ORC then adopts, the module is optimised, handed to LLJIT, and
//! the entry symbol looked up. The `Jit` keeps the LLJIT -- and so the code
//! memory -- alive until it is dropped, which `Compiled` arranges to be
//! after the last caller. One LLJIT per leaf is the simplest lifetime
//! story; S11a's generations can move to resource trackers on one JIT if
//! forty LLJITs per scene turn out to matter.

use std::ffi::{CStr, CString};
use std::mem;
use std::sync::{Arc, OnceLock};

use inkwell::OptimizationLevel;
use inkwell::context::{Context, ContextRef};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};

use llvm_sys::error::{LLVMConsumeError, LLVMErrorRef, LLVMGetErrorMessage};
use llvm_sys::orc2::lljit::{
    LLVMOrcCreateLLJIT, LLVMOrcCreateLLJITBuilder, LLVMOrcDisposeLLJIT, LLVMOrcLLJITAddLLVMIRModule,
    LLVMOrcLLJITGetGlobalPrefix, LLVMOrcLLJITGetMainJITDylib, LLVMOrcLLJITLookup, LLVMOrcLLJITRef,
};
use llvm_sys::orc2::{
    LLVMOrcCreateDynamicLibrarySearchGeneratorForProcess, LLVMOrcCreateNewThreadSafeContextFromLLVMContext,
    LLVMOrcCreateNewThreadSafeModule, LLVMOrcDefinitionGeneratorRef, LLVMOrcDisposeThreadSafeContext,
    LLVMOrcJITDylibAddGenerator,
};

use scm::Vm;

use crate::codegen::{Cg, entry_symbol};
use crate::subset::Summary;
use crate::{Refusal, Shape};

pub struct Jit {
    jit: LLVMOrcLLJITRef,
}

// The handle is only ever used again in `Drop`, which runs once, after the
// last `Compiled` clone -- and so the last caller -- is gone. The code it
// owns is pure and re-entrant; that is what the subset guarantees.
unsafe impl Send for Jit {}
unsafe impl Sync for Jit {}

impl Drop for Jit {
    fn drop(&mut self) {
        // Safety: created by `LLVMOrcCreateLLJIT` below, disposed once.
        unsafe {
            let err = LLVMOrcDisposeLLJIT(self.jit);
            if !err.is_null() {
                LLVMConsumeError(err);
            }
        }
    }
}

pub(crate) fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        Target::initialize_native(&InitializationConfig::default()).expect("native target initialisation");
    });
}

pub(crate) fn message(err: LLVMErrorRef) -> String {
    unsafe {
        let msg = LLVMGetErrorMessage(err);
        let s = CStr::from_ptr(msg).to_string_lossy().into_owned();
        LLVMConsumeError(err);
        s
    }
}

pub(crate) fn target_machine() -> Result<TargetMachine, String> {
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
    target
        .create_target_machine(
            &triple,
            &TargetMachine::get_host_cpu_name().to_string(),
            &TargetMachine::get_host_cpu_features().to_string(),
            OptimizationLevel::Aggressive,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| "no target machine for the host".to_string())
}

/// An LLJIT with the process's symbols -- libm -- visible to it.
pub(crate) fn create_lljit() -> Result<(LLVMOrcLLJITRef, Arc<Jit>), String> {
    unsafe {
        let mut jit: LLVMOrcLLJITRef = std::ptr::null_mut();
        let err = LLVMOrcCreateLLJIT(&mut jit, LLVMOrcCreateLLJITBuilder());
        if !err.is_null() {
            return Err(format!("LLJIT: {}", message(err)));
        }
        let owner = Arc::new(Jit { jit });
        let jd = LLVMOrcLLJITGetMainJITDylib(jit);
        let mut generator: LLVMOrcDefinitionGeneratorRef = std::ptr::null_mut();
        let err = LLVMOrcCreateDynamicLibrarySearchGeneratorForProcess(
            &mut generator,
            LLVMOrcLLJITGetGlobalPrefix(jit),
            None,
            std::ptr::null_mut(),
        );
        if !err.is_null() {
            return Err(format!("process symbols: {}", message(err)));
        }
        LLVMOrcJITDylibAddGenerator(jd, generator);
        Ok((jit, owner))
    }
}

/// What a successful build hands back: the code's address, the JIT that
/// owns its memory, and the data snapshots its address constants point at.
pub struct Built {
    pub addr: usize,
    pub owner: Arc<Jit>,
    pub data: Vec<Arc<Vec<f64>>>,
}

/// Emit, optimise, JIT, and look up the entry point.
pub fn build(vm: &mut Vm, name: &str, shape: Shape, _summary: &Summary) -> Result<Built, Refusal> {
    init();
    let refuse = |m: String| Refusal { message: m, via: vec![name.to_string()] };
    unsafe {
        // We create the context; ORC adopts it (LLVM 21 removed the other
        // direction). inkwell must not dispose it, hence the forget.
        let owned = Context::create();
        let raw_ctx = owned.raw();
        mem::forget(owned);
        let tsctx = LLVMOrcCreateNewThreadSafeContextFromLLVMContext(raw_ctx);
        let ctx = ContextRef::new(raw_ctx);

        let mut cg = Cg::new(ctx);
        cg.entry(vm, name, shape).map_err(refuse)?;
        cg.module.verify().map_err(|s| refuse(format!("the emitted IR does not verify: {s}")))?;
        let tm = target_machine().map_err(refuse)?;
        cg.module
            .run_passes("default<O2>", &tm, PassBuilderOptions::create())
            .map_err(|s| refuse(format!("optimisation failed: {s}")))?;

        let (jit, owner) = create_lljit().map_err(refuse)?;
        let jd = LLVMOrcLLJITGetMainJITDylib(jit);

        // ORC takes the module; the builder is ours to drop first. The
        // snapshots come with us: the code holds their addresses.
        let Cg { module, b, snapshots, .. } = cg;
        drop(b);
        let raw = module.as_mut_ptr();
        mem::forget(module);
        let tsm = LLVMOrcCreateNewThreadSafeModule(raw, tsctx);
        LLVMOrcDisposeThreadSafeContext(tsctx);
        let err = LLVMOrcLLJITAddLLVMIRModule(jit, jd, tsm);
        if !err.is_null() {
            return Err(refuse(format!("adding the module: {}", message(err))));
        }
        let cname = CString::new(entry_symbol(name)).unwrap();
        let mut addr: u64 = 0;
        let err = LLVMOrcLLJITLookup(jit, &mut addr, cname.as_ptr());
        if !err.is_null() {
            return Err(refuse(format!("looking up the entry: {}", message(err))));
        }
        if addr == 0 {
            return Err(refuse("the entry resolved to a null address".into()));
        }
        Ok(Built { addr: addr as usize, owner, data: snapshots })
    }
}

/// The optimised IR, for reading.
pub fn ir(vm: &mut Vm, name: &str, shape: Shape, _summary: &Summary) -> Result<String, Refusal> {
    init();
    let refuse = |m: String| Refusal { message: m, via: vec![name.to_string()] };
    let owned = Context::create();
    let ctx = unsafe { ContextRef::new(owned.raw()) };
    let mut cg = Cg::new(ctx);
    cg.entry(vm, name, shape).map_err(refuse)?;
    cg.module.verify().map_err(|s| refuse(format!("the emitted IR does not verify: {s}")))?;
    let tm = target_machine().map_err(refuse)?;
    cg.module
        .run_passes("default<O2>", &tm, PassBuilderOptions::create())
        .map_err(|s| refuse(format!("optimisation failed: {s}")))?;
    Ok(cg.module.print_to_string().to_string())
}
