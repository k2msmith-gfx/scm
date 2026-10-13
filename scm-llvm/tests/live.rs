//! S11a's exit criterion (plan §9): redefining a leaf function mid-render
//! does not crash, the running pass finishes with the old code, and the
//! next pass uses the new.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use scm::Vm;
use scm_llvm::{Live, Shape, Vec3};

const SRC: &str = r#"
(define (mix a b t) (+ a (* t (- b a))))
(define tint 0.1)
(define palette (vector 0.8 0.5 0.2))
(define (texel u v)
  (let ([n (* u v)])
    (map (lambda (c) (mix tint c n)) (list (vector-ref palette 0) (vector-ref palette 1) (vector-ref palette 2)))))
"#;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str(SRC).unwrap();
    vm
}

fn pass(code: &scm_llvm::Compiled) -> Vec<Vec3> {
    (0..64 * 64).map(|i| code.call3(&[(i % 64) as f64 / 64.0, (i / 64) as f64 / 64.0]).unwrap()).collect()
}

#[test]
fn a_pass_in_flight_keeps_the_old_code_and_the_next_pass_gets_the_new() {
    let mut vm = vm();
    let live = Live::compile(&mut vm, "texel", Shape::Vec3).unwrap();
    assert_eq!(live.generation(), 0);
    let (old, g0) = live.snapshot();
    let before = pass(&old);

    // The redefinition arrives mid-pass: the REPL redefines, the host refreshes.
    vm.eval_str("(define (texel u v) (list u v 1.0))").unwrap();
    assert!(live.stale(&vm));
    let g1 = live.refresh(&mut vm).unwrap();
    assert_eq!(g1, Some(1));
    assert_eq!(live.generation(), 1);

    // The in-flight pass still holds the old code, and it still runs.
    assert_eq!(pass(&old), before);
    // The next pass picks up the new.
    let (new, g) = live.snapshot();
    assert_eq!((g, g0), (1, 0));
    assert_eq!(new.call3(&[0.25, 0.5]), Some(Vec3 { x: 0.25, y: 0.5, z: 1.0 }));
    assert_ne!(pass(&new), before);
    drop(old); // the old JIT goes here, after its last caller
    assert_eq!(new.call3(&[0.25, 0.5]), Some(Vec3 { x: 0.25, y: 0.5, z: 1.0 }));
}

#[test]
fn staleness_tracks_helpers_and_tables_but_not_unrelated_globals() {
    let mut vm = vm();
    let live = Live::compile(&mut vm, "texel", Shape::Vec3).unwrap();
    assert!(!live.stale(&vm));
    assert_eq!(live.refresh(&mut vm).unwrap(), None);
    vm.eval_str("(define unrelated 42)").unwrap();
    assert!(!live.stale(&vm), "an unrelated define is not a change");
    // A helper it reaches.
    vm.eval_str("(define (mix a b t) (* 0.0 a b t))").unwrap();
    assert!(live.stale(&vm));
    assert_eq!(live.refresh(&mut vm).unwrap(), Some(1));
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]), Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 }));
    // A table it captured, rebound.
    vm.eval_str("(define (mix a b t) b) (define palette (vector 1.0 2.0 3.0))").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), Some(2));
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]), Some(Vec3 { x: 1.0, y: 2.0, z: 3.0 }));
    // A constant it folded.
    vm.eval_str("(define (mix a b t) a) (define tint 0.75)").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), Some(3));
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]), Some(Vec3 { x: 0.75, y: 0.75, z: 0.75 }));
    vm.eval_str("(define (mix a b t) b)").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), Some(4));
    // Mutating the table in place moves no binding: that is what `recompile` is for.
    vm.eval_str("(vector-set! palette 0 9.0)").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), None);
    assert!(!live.stale(&vm));
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]).unwrap().x, 1.0, "the snapshot is a snapshot");
    live.recompile(&mut vm).unwrap();
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]).unwrap().x, 9.0);
}

#[test]
fn a_refused_redefinition_keeps_the_old_code_running() {
    let mut vm = vm();
    let live = Live::compile(&mut vm, "texel", Shape::Vec3).unwrap();
    let before = live.snapshot().0.call3(&[0.5, 0.5]);
    vm.eval_str("(define (texel u v) (display u) (list u v 1.0))").unwrap();
    let r = live.refresh(&mut vm).unwrap_err();
    assert!(r.message.contains("`display` is not a pure primitive"), "{r}");
    assert_eq!(live.generation(), 0);
    assert_eq!(live.snapshot().0.call3(&[0.5, 0.5]), before);
    // Still stale, so a fix is picked up by the next refresh.
    assert!(live.stale(&vm));
    vm.eval_str("(define (texel u v) (list u v 1.0))").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), Some(1));
}

#[test]
fn redefining_under_a_running_render_does_not_crash() {
    let mut vm = vm();
    let live = Arc::new(Live::compile(&mut vm, "texel", Shape::Vec3).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    // A render thread: passes back to back, each on a snapshot, each
    // checked for self-consistency against the snapshot it started with.
    let worker = {
        let live = Arc::clone(&live);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut passes = 0u64;
            let mut last = None;
            while !stop.load(Ordering::Relaxed) {
                let (code, generation) = live.snapshot();
                let grid = pass(&code);
                // Every texel of this pass came from this code.
                assert_eq!(grid[777], code.call3(&[(777 % 64) as f64 / 64.0, (777 / 64) as f64 / 64.0]).unwrap());
                passes += 1;
                last = Some((generation, grid[0]));
            }
            (passes, last.unwrap())
        })
    };
    // The REPL: twenty redefinitions while that runs.
    let mut gens = Vec::new();
    for k in 1..=20 {
        vm.eval_str(&format!("(define (texel u v) (list {k}.0 u v))")).unwrap();
        gens.push(live.refresh(&mut vm).unwrap().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    assert_eq!(gens, (1..=20).collect::<Vec<u64>>());
    stop.store(true, Ordering::Relaxed);
    let (passes, (generation, first)) = worker.join().expect("the render thread did not crash");
    assert!(passes > 1, "the worker ran {passes} passes");
    // The last pass used the definition of its generation.
    assert_eq!(first.x, generation as f64);
    // And the final pass after the last refresh sees the twentieth.
    let (code, g) = live.snapshot();
    assert_eq!((g, code.call3(&[0.0, 0.0]).unwrap().x), (20, 20.0));
}
