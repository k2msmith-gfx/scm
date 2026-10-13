//! The R7RS benchmark suite as a language gate (plan §10).
//!
//! ecraven/r7rs-benchmarks, at reduced sizes: `bench/inputs-small/` holds one
//! input per program with the iteration count at 1 and the problem size cut
//! until T0 finishes it in seconds, plus the answer it must return. The
//! published inputs stay the performance instrument; this is the check that
//! every program *runs* and is *right*, and from S12 on that every tier
//! agrees. The five programs outside the declared subset are listed in
//! `bench/README.md` with their reasons.
//!
//! The suite is cloned, not vendored (6 MB of inputs, mixed copyright
//! headers). It is found at `$R7RS_BENCHMARKS` or `target/r7rs-benchmarks`;
//! `bench/run.py --fetch` puts it there. Without it the test says so and
//! passes, which is weaker than a failure -- but a gate that cannot run in a
//! fresh checkout without a network is a gate that gets disabled.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use scm::Interp;

const BENCH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/bench");

fn suite() -> Option<PathBuf> {
    let candidates = [
        std::env::var("R7RS_BENCHMARKS").ok().map(PathBuf::from),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/r7rs-benchmarks")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join("src/common.scm").is_file())
}

/// `inputs/` as the programs expect it, next to an `outputs/` for the ones
/// that write -- but with `bib` cut to 400 KB, because `wc` reading 4.4 MB
/// character by character is a speed measurement, not a language one.
fn run_dir(suite: &Path) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/bench-small");
    let inputs = dir.join("inputs");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&inputs).unwrap();
    fs::create_dir_all(dir.join("outputs")).unwrap();
    for entry in fs::read_dir(suite.join("inputs")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "bib" {
            let bytes = fs::read(entry.path()).unwrap();
            fs::write(inputs.join(&name), &bytes[..bytes.len().min(400_000)]).unwrap();
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(entry.path(), inputs.join(&name)).unwrap();
            #[cfg(not(unix))]
            fs::copy(entry.path(), inputs.join(&name)).unwrap();
        }
    }
    dir
}

fn scheme_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One program, in-process: the same four pieces the suite's own `bench`
/// script concatenates, with stdin standing in as a string port.
fn run_one(suite: &Path, name: &str) -> Result<f64, String> {
    let read = |p: PathBuf| fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()));
    let program = [
        read(Path::new(BENCH).join("scm-prelude.scm"))?,
        read(suite.join("src").join(format!("{name}.scm")))?,
        read(suite.join("src/common.scm"))?,
        read(suite.join("src/common-postlude.scm"))?,
    ]
    .concat();
    let input = read(Path::new(BENCH).join("inputs-small").join(format!("{name}.input")))?;
    let mut interp = Interp::new().map_err(|e| format!("{e}"))?;
    interp
        .eval(&format!(
            "(define current-input-port (make-parameter (open-input-string {})))",
            scheme_string(&input)
        ))
        .map_err(|e| format!("stdin: {e}"))?;
    let t0 = Instant::now();
    let (result, output) = interp.eval_capturing_named(&program, Some(&format!("{name}.scm")));
    let secs = t0.elapsed().as_secs_f64();
    if let Err(e) = result {
        return Err(format!("raised: {e}\n{output}"));
    }
    let csv = output
        .lines()
        .find(|l| l.starts_with("+!CSVLINE!+"))
        .ok_or_else(|| format!("no result line:\n{output}"))?;
    if csv.ends_with("INCORRECT") {
        let why = output.lines().find(|l| l.contains("incorrect")).unwrap_or("");
        return Err(format!("wrong answer: {why}"));
    }
    Ok(secs)
}

#[test]
fn every_program_in_the_subset_runs_and_is_right() {
    let Some(suite) = suite() else {
        eprintln!(
            "r7rs-benchmarks not found: set R7RS_BENCHMARKS or run `scm-conform/bench/run.py --fetch`; skipping"
        );
        return;
    };
    let dir = run_dir(&suite);
    std::env::set_current_dir(&dir).unwrap();
    let mut names: Vec<String> = fs::read_dir(Path::new(BENCH).join("inputs-small"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".input").to_string())
        .collect();
    names.sort();
    assert!(names.len() >= 50, "only {} small inputs", names.len());

    // Threads, each with its own interpreter: the slowest program bounds the
    // wall time instead of the sum.
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let queue = std::sync::Mutex::new(names.clone().into_iter());
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let Some(name) = queue.lock().unwrap().next() else { break };
                    let r = run_one(&suite, &name);
                    results.lock().unwrap().push((name, r));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let mut failed = Vec::new();
    for (name, r) in &results {
        match r {
            Ok(secs) => eprintln!("{name:12} ok  {secs:6.2}s"),
            Err(e) => {
                eprintln!("{name:12} FAILED  {e}");
                failed.push(name.clone());
            }
        }
    }
    assert!(failed.is_empty(), "{} of {} programs failed: {failed:?}", failed.len(), results.len());
}
