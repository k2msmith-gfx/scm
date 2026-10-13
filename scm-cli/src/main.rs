//! The `scm` binary: a REPL and a script runner.
//!
//! This is deliberately the *thin* version. Plan §9's S8 is the REPL as a
//! protocol -- an eval API with several clients, live-frame backtraces,
//! introspection, redefinition semantics -- and none of that is here. What is
//! here is enough to type at the language, which is worth having long before
//! the protocol is.
//!
//! The one piece that is not a shortcut is multi-line input: it asks
//! [`scm::parse_status`] whether the text so far is complete, so an unclosed
//! paren continues the entry instead of reporting a syntax error. That
//! distinction is why S1 made *incomplete* and *invalid* different answers.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use scm::{ParseStatus, Vm, VmError, write_str};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fuel: Option<u64> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fuel" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<u64>().ok()) {
                    Some(n) => fuel = Some(n),
                    None => {
                        eprintln!("scm: --fuel wants a number");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!("scm {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }

    let mut vm = Vm::new();
    vm.set_fuel(fuel);

    match rest.first().map(String::as_str) {
        Some("-e") => match rest.get(1) {
            Some(src) => run_once(&mut vm, src),
            None => {
                eprintln!("scm: -e wants an expression");
                ExitCode::from(2)
            }
        },
        Some(path) => match std::fs::read_to_string(path) {
            Ok(src) => run_once(&mut vm, &src),
            Err(e) => {
                eprintln!("scm: {path}: {e}");
                ExitCode::from(2)
            }
        },
        None => repl(&mut vm),
    }
}

fn usage() {
    println!("usage: scm [--fuel N] [FILE | -e EXPR]");
    println!();
    println!("  no arguments   start the REPL");
    println!("  FILE           run a file");
    println!("  -e EXPR        evaluate one expression and print it");
    println!("  --fuel N       stop after N safepoints; a runaway is an error,");
    println!("                 not a hang (plan requirement 8)");
}

fn run_once(vm: &mut Vm, src: &str) -> ExitCode {
    match vm.eval_str(src) {
        Ok(v) => {
            // A definition or a `for` returns nothing worth printing.
            let text = write_str(&vm.engine, v);
            if text != "#<unspecified>" {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", describe(&e));
            ExitCode::FAILURE
        }
    }
}

fn repl(vm: &mut Vm) -> ExitCode {
    println!("scm {} — Ctrl-D to leave", env!("CARGO_PKG_VERSION"));
    println!(
        "{} globals, {} objects after boot",
        vm.global_count(),
        vm.engine.obj_count()
    );
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut pending = String::new();

    loop {
        let prompt = if pending.is_empty() { "scm> " } else { "...  " };
        print!("{prompt}");
        let _ = io::stdout().flush();

        let Some(line) = lines.next() else {
            println!();
            return ExitCode::SUCCESS;
        };
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("scm: {e}");
                return ExitCode::FAILURE;
            }
        };

        if pending.is_empty() && line.trim().is_empty() {
            continue;
        }
        pending.push_str(&line);
        pending.push('\n');

        match scm::parse_status(&pending) {
            // An unclosed paren is "keep typing", not an error.
            ParseStatus::Incomplete => continue,
            ParseStatus::Invalid(msg) => {
                eprintln!("read error: {msg}");
                pending.clear();
                continue;
            }
            ParseStatus::Complete => {}
        }

        let src = std::mem::take(&mut pending);
        match vm.eval_str(&src) {
            Ok(v) => {
                let text = write_str(&vm.engine, v);
                if text != "#<unspecified>" {
                    println!("{text}");
                }
            }
            Err(e) => eprintln!("{}", describe(&e)),
        }
    }
}

/// The REPL has to stay usable after a refusal, so an error prints and the
/// loop continues -- which is also the cheapest demonstration that a failed
/// evaluation leaves the VM intact (requirement 8).
fn describe(e: &VmError) -> String {
    match e {
        VmError::Interrupted => "interrupted".into(),
        VmError::OutOfFuel => "out of fuel (raise or drop --fuel)".into(),
        VmError::OutOfMemory => "heap cap exceeded".into(),
        VmError::Error(msg) => format!("error: {msg}"),
    }
}
