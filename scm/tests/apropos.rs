//! `apropos`: names first, then docstrings.
//!
//! The docstring half exists because a reader searches with the word they
//! know, and that is often not the name: ray's agents asked for "lathe" and
//! "vase" while the verb was `revolve` (ray's agent-surface-queue.md §1).

use scm::{Vm, write_str};

#[track_caller]
fn eval(src: &str) -> String {
    let mut vm = Vm::new();
    let v = vm
        .eval_str(src)
        .unwrap_or_else(|e| panic!("eval {src:?}: {e}"));
    write_str(&vm.engine, v)
}

const DEFS: &str = r#"
(define (revolve p) "Spin a profile about an axis -- a Vase, a dome." p)
(define (vase-count) "How many there are." 3)
(define (bowl) 1)
"#;

#[test]
fn a_name_hit_comes_before_a_docstring_hit() {
    assert_eq!(eval(&format!("{DEFS} (apropos \"vase\")")), "(vase-count revolve)");
}

#[test]
fn the_docstring_is_searched_without_regard_to_case() {
    assert_eq!(eval(&format!("{DEFS} (apropos \"DOME\")")), "(revolve)");
}

#[test]
fn the_name_match_is_still_case_sensitive_and_exact_text() {
    // Names are matched as written, as before: only the docstring half
    // folds case, because prose capitalises and identifiers do not.
    assert_eq!(eval(&format!("{DEFS} (apropos \"Vase-\")")), "()");
}

#[test]
fn a_name_found_both_ways_is_listed_once() {
    assert_eq!(eval(&format!("{DEFS} (apropos \"revolve\")")), "(revolve)");
}

#[test]
fn a_procedure_without_a_docstring_is_found_by_name_only() {
    assert_eq!(eval(&format!("{DEFS} (apropos \"bowl\")")), "(bowl)");
}

#[test]
fn no_argument_is_every_name() {
    let all = eval(&format!("{DEFS} (length (apropos))"));
    let named = eval(&format!("{DEFS} (length (apropos \"\"))"));
    assert_eq!(all, named);
    assert!(all.parse::<usize>().unwrap() > 300, "{all} globals");
}
