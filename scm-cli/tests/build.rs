//! S14's exit criterion (plan §9): a hello-world binary runs with no
//! toolchain present. The build invokes nothing but the running `scm`,
//! and the test runs both the build and the result with an empty PATH.

use std::process::Command;

fn scm() -> &'static str {
    env!("CARGO_BIN_EXE_scm")
}

#[test]
fn scm_build_makes_a_binary_that_runs_without_a_toolchain() {
    let dir = std::env::temp_dir().join(format!("scm-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("hello.scm");
    std::fs::write(
        &src,
        r#"(define (greet who) (string-append "hello, " who))
(display (greet (if (> (length (command-line)) 1) (cadr (command-line)) "world")))
(newline)
(let ([line (read-line)])
  (unless (eof-object? line) (display (string-append "stdin said " line)) (newline)))
(exit (if (member "fail" (command-line)) 3 0))
"#,
    )
    .unwrap();
    let out = dir.join("hello");
    let built = Command::new(scm())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let note = String::from_utf8_lossy(&built.stdout);
    assert!(note.contains("bytes of program"), "{note}");

    let run = Command::new(&out).arg("kevin").env("PATH", "").output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "hello, kevin\n");

    // stdin reaches the program, and the exit code is the program's.
    let mut child = Command::new(&out)
        .args(["x", "fail"])
        .env("PATH", "")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"ping\n").unwrap();
    let run = child.wait_with_output().unwrap();
    assert_eq!(run.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "hello, x\nstdin said ping\n");

    // An error in the program is reported and the exit status says so.
    std::fs::write(&src, "(display \"before\")\n(car 5)\n").unwrap();
    let out2 = dir.join("broken");
    assert!(Command::new(scm()).args(["build", src.to_str().unwrap(), "-o", out2.to_str().unwrap()]).output().unwrap().status.success());
    let run = Command::new(&out2).output().unwrap();
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("not a pair"));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before");

    // A bundle is its program, not a builder: `build` is just an argument.
    let run = Command::new(&out).args(["build", "x"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "hello, build\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_plain_scm_is_not_mistaken_for_a_bundle() {
    let run = Command::new(scm()).args(["-e", "(+ 1 2)"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "3");
}
