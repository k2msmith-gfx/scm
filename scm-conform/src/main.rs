//! Run an external R7RS conformance suite and report a number.
//!
//! Plan §10: the suite is the primary oracle precisely because it was written
//! by other people against the spec with no knowledge of this
//! implementation. So the rule is that **nothing here edits the suite** —
//! `harness.scm` adapts to it, and whatever is still missing shows up in the
//! number rather than being smoothed over.
//!
//! The file is split into top-level forms *textually*, using the same
//! [`scm::parse_status`] the REPL uses for multi-line entry. That matters:
//! the suite contains datums this reader cannot read — `1/3` rationals among
//! them, which plan §2.3 deliberately does not support — and reading the file
//! in one go would make the first of them the end of the run. Split by form,
//! an unreadable form is one recorded line in the report.
//!
//! The counterpart rule from §10: a failing test is either fixed or
//! individually explained. So every failure is written out, not just counted.

use std::process::ExitCode;

use scm::{ParseStatus, Vm};

const HARNESS: &str = include_str!("../harness.scm");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let suite_path = args
        .first()
        .cloned()
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/suites/r7rs-tests.scm").into());
    let out_path = args.get(1).cloned();

    let source = match std::fs::read_to_string(&suite_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("scm-conform: {suite_path}: {e}");
            return ExitCode::from(2);
        }
    };

    let mut vm = Vm::new();
    #[cfg(feature = "t2")]
    if matches!(std::env::var("SCM_TIER").as_deref(), Ok("t2") | Ok("T2")) {
        scm_llvm::install_t2(&mut vm);
    }
    // A runaway test must stop the runner rather than hang it. Generous
    // enough that no honest test hits it.
    vm.set_fuel(Some(200_000_000));
    if let Err(e) = vm.eval_str(HARNESS) {
        eprintln!("scm-conform: the harness itself failed: {e}");
        return ExitCode::from(2);
    }

    let forms = split_forms(&source);
    let mut unreadable: Vec<(usize, String)> = Vec::new();
    let mut aborted: Vec<(usize, String)> = Vec::new();

    for (line, text) in &forms {
        if let ParseStatus::Invalid(msg) = scm::parse_status(text) {
            unreadable.push((*line, msg));
            continue;
        }
        if let Err(e) = vm.eval_str(text) {
            // A form that raises *outside* a test — a `define` using something
            // absent, usually — takes its tests with it, so it is reported
            // separately from a failing assertion.
            aborted.push((*line, format!("{e}")));
        }
    }

    // Ask the harness for its tally, one failure per line.
    let report = match vm.eval_str("(failure-lines)") {
        Ok(v) => lines_of(&vm, v),
        Err(e) => {
            eprintln!("scm-conform: could not read the report: {e}");
            return ExitCode::from(2);
        }
    };
    let (passed, failed) = counts(&mut vm);

    let total = passed + failed;
    println!("suite: {suite_path}");
    println!("  forms:          {}", forms.len());
    println!("  unreadable:     {}", unreadable.len());
    println!("  forms aborted:  {}", aborted.len());
    println!();
    println!("  tests passed:   {passed}");
    println!("  tests failed:   {failed}");
    if total > 0 {
        println!(
            "  conformance:    {:.1}% of {total} assertions reached",
            100.0 * passed as f64 / total as f64
        );
    }

    if let Some(path) = out_path {
        let mut text = String::new();
        text.push_str(&format!("# {suite_path}\n\n"));
        text.push_str(&format!("passed {passed}, failed {failed}\n\n"));
        text.push_str("## unreadable forms\n\n");
        for (line, msg) in &unreadable {
            text.push_str(&format!("- line {line}: {msg}\n"));
        }
        text.push_str("\n## forms that raised outside a test\n\n");
        for (line, msg) in &aborted {
            text.push_str(&format!("- line {line}: {msg}\n"));
        }
        text.push_str("\n## failing assertions\n\n");
        for line in &report {
            text.push_str("- ");
            text.push_str(line);
            text.push('\n');
        }
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("scm-conform: could not write {path}: {e}");
            return ExitCode::from(2);
        }
        println!();
        println!("  written to:     {path}");
    }

    ExitCode::SUCCESS
}

/// A Scheme list of strings, as Rust strings.
fn lines_of(vm: &Vm, v: scm::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = v;
    while let scm::Value::Obj(r) = cur {
        let scm::Obj::Pair(car, cdr) = vm.engine.obj(r) else {
            break;
        };
        out.push(scm::display_str(&vm.engine, *car));
        cur = *cdr;
    }
    out
}

fn counts(vm: &mut Vm) -> (i64, i64) {
    let p = vm
        .eval_str("*passed*")
        .ok()
        .and_then(as_int)
        .unwrap_or(0);
    let f = vm
        .eval_str("*failed*")
        .ok()
        .and_then(as_int)
        .unwrap_or(0);
    (p, f)
}

fn as_int(v: scm::Value) -> Option<i64> {
    match v {
        scm::Value::Fixnum(n) => Some(n),
        _ => None,
    }
}

/// Split source into top-level forms, by asking the reader when a chunk has
/// become complete. The same question the REPL asks, for the same reason.
fn split_forms(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut pending = String::new();
    let mut start_line = 1usize;
    for (n, line) in src.lines().enumerate() {
        if pending.is_empty() {
            if line.trim().is_empty() || line.trim_start().starts_with(";;") {
                continue;
            }
            start_line = n + 1;
        }
        pending.push_str(line);
        pending.push('\n');
        match scm::parse_status(&pending) {
            ParseStatus::Incomplete => continue,
            // Hand an invalid chunk on as-is; the caller records it and moves
            // to the next form rather than trying to resynchronise.
            ParseStatus::Complete | ParseStatus::Invalid(_) => {
                out.push((start_line, std::mem::take(&mut pending)));
            }
        }
    }
    if !pending.trim().is_empty() {
        out.push((start_line, pending));
    }
    out
}
