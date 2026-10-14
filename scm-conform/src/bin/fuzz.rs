//! `scm-fuzz [--seed S] [--count N] [--stress] [--t2] [--show]`
//!
//! Differential fuzzing of the tiers. Exit status 1 on any mismatch, with
//! each failing program written to `target/fuzz/<seed>-<index>.scm` for
//! replay: `scm target/fuzz/S-I.scm` runs it on T1, `SCM_TIER=t0` on T0.

use std::process::ExitCode;

use scm_conform::fuzz::{Gen, Verdict, compare};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut count = 200u64;
    let mut stress = false;
    let mut want_t2 = false;
    let mut show = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => { i += 1; seed = args[i].parse().expect("--seed N"); }
            "--count" => { i += 1; count = args[i].parse().expect("--count N"); }
            "--stress" => stress = true,
            "--t2" => want_t2 = true,
            "--show" => show = true,
            other => { eprintln!("scm-fuzz: unknown argument {other}"); return ExitCode::from(2); }
        }
        i += 1;
    }
    if show {
        for index in 0..count {
            let mut g = Gen::new(seed.wrapping_mul(1_000_003).wrapping_add(index));
            println!(";; {seed}-{index}\n{}", g.program());
        }
        return ExitCode::SUCCESS;
    }
    #[cfg(feature = "t2")]
    let t2: Option<&dyn Fn(&mut scm::Vm)> = if want_t2 {
        Some(&|vm: &mut scm::Vm| {
            scm_llvm::install_t2(vm);
            vm.set_native_threshold(1);
        })
    } else {
        None
    };
    #[cfg(not(feature = "t2"))]
    let t2: Option<&dyn Fn(&mut scm::Vm)> = {
        if want_t2 {
            eprintln!("scm-fuzz: --t2 needs `--features t2`");
            return ExitCode::from(2);
        }
        None
    };
    let (mismatches, skipped) = compare(seed, count, stress, t2);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/fuzz");
    for m in &mismatches {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}-{}.scm", m.seed, m.index));
        let _ = std::fs::write(&path, &m.program);
        println!("MISMATCH {}-{}  ({})", m.seed, m.index, path.display());
        for (tier, v) in &m.tiers {
            match v {
                Verdict::Value { printed, output } => println!("  {tier}: value {printed}  output {output:?}"),
                Verdict::Error { message, output } => println!("  {tier}: error {message}  output {output:?}"),
                Verdict::Stopped => println!("  {tier}: stopped"),
            }
        }
    }
    println!(
        "{count} programs, seed {seed}: {} mismatch{}, {skipped} skipped (fuel){}{}",
        mismatches.len(),
        if mismatches.len() == 1 { "" } else { "es" },
        if stress { ", gc stress on T1" } else { "" },
        if want_t2 { ", T2 included" } else { "" }
    );
    if mismatches.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
