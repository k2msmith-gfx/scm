// S0: prove the plan's backend bet on this machine.
//
//   A. inkwell builds and verifies IR.
//   B. inkwell's own ExecutionEngine (legacy MCJIT) runs it.
//   C. ORCv2/LLJIT via llvm-sys::orc2 runs it -- the path the plan recommends.
//   D. a SECOND module is added to the same JIT afterwards and resolved,
//      which is the incremental story the REPL needs.
//
// Throwaway. Not part of the scm crate.

use std::ffi::CString;
use std::mem;
use std::time::Instant;

use inkwell::context::{Context, ContextRef};
use inkwell::targets::{InitializationConfig, Target};
use inkwell::OptimizationLevel;

use llvm_sys::error::{LLVMConsumeError, LLVMErrorRef, LLVMGetErrorMessage};
use llvm_sys::orc2::lljit::{
    LLVMOrcCreateLLJIT, LLVMOrcCreateLLJITBuilder, LLVMOrcLLJITAddLLVMIRModule,
    LLVMOrcLLJITGetMainJITDylib, LLVMOrcLLJITLookup, LLVMOrcLLJITRef,
};
use llvm_sys::orc2::{
    LLVMOrcCreateNewThreadSafeContextFromLLVMContext, LLVMOrcCreateNewThreadSafeModule,
};

/// `fn(i64) -> i64`, computing `x * mul + 1`, under the given name.
fn build_ir<'c>(ctx: &'c ContextRef<'c>, name: &str, mul: u64) -> inkwell::module::Module<'c> {
    let module = ctx.create_module("s0");
    let i64t = ctx.i64_type();
    let f = module.add_function(name, i64t.fn_type(&[i64t.into()], false), None);
    let bb = ctx.append_basic_block(f, "entry");
    let b = ctx.create_builder();
    b.position_at_end(bb);
    let x = f.get_nth_param(0).unwrap().into_int_value();
    let m = b.build_int_mul(x, i64t.const_int(mul, false), "m").unwrap();
    let a = b.build_int_add(m, i64t.const_int(1, false), "a").unwrap();
    b.build_return(Some(&a)).unwrap();
    module
}

unsafe fn check(err: LLVMErrorRef, what: &str) {
    if !err.is_null() {
        let msg = LLVMGetErrorMessage(err);
        let s = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
        LLVMConsumeError(err);
        panic!("{what}: {s}");
    }
}

/// Hand an inkwell module to LLJIT. Takes ownership: LLVM frees it, so inkwell
/// must not.
unsafe fn add_module(jit: LLVMOrcLLJITRef, tsctx: llvm_sys::orc2::LLVMOrcThreadSafeContextRef, module: inkwell::module::Module) {
    let raw = module.as_mut_ptr();
    mem::forget(module);
    let tsm = LLVMOrcCreateNewThreadSafeModule(raw, tsctx);
    let jd = LLVMOrcLLJITGetMainJITDylib(jit);
    check(LLVMOrcLLJITAddLLVMIRModule(jit, jd, tsm), "AddLLVMIRModule");
}

unsafe fn lookup(jit: LLVMOrcLLJITRef, name: &str) -> unsafe extern "C" fn(i64) -> i64 {
    let cname = CString::new(name).unwrap();
    let mut addr: u64 = 0;
    check(LLVMOrcLLJITLookup(jit, &mut addr, cname.as_ptr()), "Lookup");
    assert!(addr != 0, "{name} resolved to a null address");
    mem::transmute(addr as usize)
}

fn main() {
    println!(
        "host: {} / {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );

    // ---- A: IR construction, in a context we own.
    {
        let owned = Context::create();
        let ctx = unsafe { ContextRef::new(owned.raw()) };
        let module = build_ir(&ctx, "triple_plus_one", 3);
        module.verify().expect("IR failed verification");
        println!("\n--- A: IR built and verified\n{}", module.print_to_string().to_string());

        // ---- B: legacy MCJIT, for comparison only.
        match module.create_jit_execution_engine(OptimizationLevel::Aggressive) {
            Ok(ee) => unsafe {
                let f = ee
                    .get_function::<unsafe extern "C" fn(i64) -> i64>("triple_plus_one")
                    .expect("MCJIT symbol lookup failed");
                println!("B: MCJIT   triple_plus_one(14) = {}", f.call(14));
            },
            Err(e) => println!("B: MCJIT   UNAVAILABLE: {e}"),
        };
    }

    // ---- C: ORCv2 / LLJIT.
    Target::initialize_native(&InitializationConfig::default()).expect("native target init");

    unsafe {
        // We create the context; ORC adopts it. (LLVM 21 removed
        // LLVMOrcThreadSafeContextGetContext, so this is now the only direction.)
        let owned = Context::create();
        let raw_ctx = owned.raw();
        mem::forget(owned); // ORC owns it from here; inkwell must not dispose it.
        let tsctx = LLVMOrcCreateNewThreadSafeContextFromLLVMContext(raw_ctx);
        let ctx = ContextRef::new(raw_ctx);

        let t0 = Instant::now();
        let mut jit: LLVMOrcLLJITRef = std::ptr::null_mut();
        check(LLVMOrcCreateLLJIT(&mut jit, LLVMOrcCreateLLJITBuilder()), "CreateLLJIT");
        let t_create = t0.elapsed();

        let m1 = build_ir(&ctx, "triple_plus_one", 3);
        m1.verify().expect("m1 verify");
        let t1 = Instant::now();
        add_module(jit, tsctx, m1);
        let f = lookup(jit, "triple_plus_one");
        let t_compile = t1.elapsed();
        println!("C: LLJIT   triple_plus_one(14) = {}", f(14));

        // ---- D: a second module, added after the first already ran.
        let m2 = build_ir(&ctx, "times_ten_plus_one", 10);
        m2.verify().expect("m2 verify");
        let t2 = Instant::now();
        add_module(jit, tsctx, m2);
        let g = lookup(jit, "times_ten_plus_one");
        let t_compile2 = t2.elapsed();
        println!("D: LLJIT   times_ten_plus_one(14) = {}", g(14));
        println!("   and the first function still works: {}", f(14));

        println!("\nlatency: LLJIT create {:?}, 1st module add+lookup {:?}, 2nd {:?}",
                 t_create, t_compile, t_compile2);
    }

    println!("\nS0 OK");
}
