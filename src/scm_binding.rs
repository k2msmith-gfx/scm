//! **The editor's prims, for the Scheme VM** (docs/editor-crate-plan.md,
//! E2) — and the one place in `edit` that names the VM, as `script/` is in
//! the modeler (`layering.rs` holds both). A host installs them into its
//! session with [`install`]; the modeler does, beside its own.
//!
//! The bodies came from the modeler's `script/editor.rs`, `script/text.rs`
//! and `script/xref.rs` unchanged, declared through a [`prim!`] that is the
//! modeler's own, cut to what they use: arguments that are numbers, text or
//! a list of text, and answers that are nothing, a number, text, or a list.
//!
//! **No parked host.** The modeler's wrapper parks the interpreter for the
//! length of a prim, because some of its prims call back into the prelude
//! and a second borrow of the session would panic. These never do: they
//! reach the widget, and the widget reaches its host only for rects, dirty
//! flags and colours.

/// What a prim answers.
pub enum Out {
    Nil,
    Num(f64),
    Text(String),
    /// A tuple of numbers — a buffer list row.
    Nums(Vec<f64>),
    /// A tuple of answers.
    Many(Vec<Out>),
    /// A mutable sequence, from the Janet years; Scheme gets a list either
    /// way, as `Many` does.
    Arr(Vec<Out>),
}

impl Out {
    pub fn nums<I: IntoIterator<Item = f64>>(xs: I) -> Out {
        Out::Nums(xs.into_iter().collect())
    }

    pub fn rows<I: IntoIterator<Item = Out>>(xs: I) -> Out {
        Out::Many(xs.into_iter().collect())
    }

    /// **`Nil` is `#f`**: scm has no nil, and `unspecified` is TRUE in
    /// Scheme, so a prim that answers "nothing" must not answer something
    /// truthy (ray's binding settled this first).
    pub fn to_val(self) -> scm::Val {
        match self {
            Out::Nil => scm::Val::Bool(false),
            Out::Num(x) => number_val(x),
            Out::Text(s) => scm::Val::Str(s),
            Out::Nums(v) => scm::Val::List(v.into_iter().map(number_val).collect()),
            Out::Many(parts) | Out::Arr(parts) => scm::Val::List(parts.into_iter().map(Out::to_val).collect()),
        }
    }
}

/// A whole number is EXACT: `12`, not `12.0` — ray's rule
/// (`ray::api::scheme::number_val`), kept here rather than borrowed so that
/// `edit` does not link the raytracer for one comparison.
fn number_val(x: f64) -> scm::Val {
    if x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0 && !(x == 0.0 && x.is_sign_negative()) {
        scm::Val::Int(x as i64)
    } else {
        scm::Val::Float(x)
    }
}

/// Absent or wrong-typed is zero, as it is in the modeler's prims.
pub fn val_num(args: &[scm::Val], i: usize) -> f64 {
    match args.get(i) {
        Some(scm::Val::Int(n)) => *n as f64,
        Some(scm::Val::Float(x)) => *x,
        _ => 0.0,
    }
}

/// Text, from a string, a symbol or a keyword.
pub fn val_text(args: &[scm::Val], i: usize) -> Option<String> {
    match args.get(i) {
        Some(scm::Val::Str(s)) | Some(scm::Val::Sym(s)) | Some(scm::Val::Keyword(s)) => Some(s.clone()),
        _ => None,
    }
}

/// ONE argument that is a list of strings.
pub fn val_textv(args: &[scm::Val], i: usize) -> Vec<String> {
    fn one(v: &scm::Val) -> String {
        match v {
            scm::Val::Str(s) | scm::Val::Sym(s) | scm::Val::Keyword(s) => s.clone(),
            scm::Val::Int(n) => n.to_string(),
            scm::Val::Float(x) => x.to_string(),
            _ => String::new(),
        }
    }
    match args.get(i) {
        Some(scm::Val::List(items)) | Some(scm::Val::Vector(items)) => items.iter().map(one).collect(),
        Some(v) => vec![one(v)],
        None => Vec::new(),
    }
}

/// The Rust type an argument kind reads as. `name` takes a keyword as well
/// as a string; so does `text` here, which is the same reading.
macro_rules! prim_ty {
    (num) => { f64 };
    (text) => { String };
    (opt_text) => { Option<String> };
    (name) => { Option<String> };
    (textv) => { Vec<String> };
}

macro_rules! prim_from_val {
    (num, $args:ident, $i:expr) => { $crate::scm_binding::val_num($args, $i) };
    (text, $args:ident, $i:expr) => { $crate::scm_binding::val_text($args, $i).unwrap_or_default() };
    (opt_text, $args:ident, $i:expr) => { $crate::scm_binding::val_text($args, $i) };
    (name, $args:ident, $i:expr) => { $crate::scm_binding::val_text($args, $i) };
    (textv, $args:ident, $i:expr) => { $crate::scm_binding::val_textv($args, $i) };
}

/// Declare a prim once: a body against no VM, in a module named after the
/// function, and the Scheme wrapper beside it.
macro_rules! prim {
    (
        $(#[$doc:meta])*
        $name:literal, fn $f:ident($($a:ident : $k:ident),* $(,)?) $body:block
    ) => {
        #[allow(non_snake_case)]
        pub(super) mod $f {
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_imports)]
            use $crate::scm_binding::Out;

            pub const NAME: &str = $name;

            $(#[$doc])*
            #[allow(unused_variables)]
            pub fn core($($a: prim_ty!($k)),*) -> Out $body

            pub fn install(interp: &mut scm::Interp) -> Result<&'static str, scm::EmbedError> {
                #[allow(unused_variables)]
                interp.define_fn(NAME, |args: &[scm::Val]| {
                    #[allow(unused_mut, unused_variables)]
                    let mut i = usize::MAX;
                    $(
                        i = i.wrapping_add(1);
                        let $a = prim_from_val!($k, args, i);
                    )*
                    Ok(core($($a),*).to_val())
                })?;
                Ok(NAME)
            }
        }
    };
}

mod editor;
mod menu;
mod text;
mod xref;

/// **The editor's shell, in Scheme** (E3): `edit/scm/`, in load order. A host
/// evaluates these after installing the prims and before its own prelude,
/// so its files can bind the shell's hooks and add to its chains — never the
/// other way round.
pub const SHELL: &[&str] = &["00-base", "01-handlers", "02-keymap", "03-menus"];

/// Where [`SHELL`]'s files are: read at boot from the tree this binary was
/// built from, as the modeler's own prelude is.
pub fn shell_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scm")
}

/// Install every editor prim into `interp`, telling `installed` each name —
/// the modeler records them, so its prelude can unwrap the array records
/// it passes (`99-scheme-only.scm`, `%prim-names`).
pub fn install(interp: &mut scm::Interp, installed: &mut dyn FnMut(&str)) -> Result<(), scm::EmbedError> {
    for name in editor::install(interp)?
        .into_iter()
        .chain(text::install(interp)?)
        .chain(xref::install(interp)?)
        .chain(menu::install(interp)?)
    {
        installed(name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scm::Val;

    fn eval(interp: &mut scm::Interp, src: &str) -> Val {
        let r = interp.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        interp.value(&r).expect("value")
    }

    /// The binding on a bare interpreter — no modeler, no prelude: what the
    /// standalone editor (E4) will boot.
    #[test]
    fn the_prims_answer_through_a_bare_interpreter() {
        let mut interp = scm::Interp::new().expect("interp");
        let mut names = Vec::new();
        install(&mut interp, &mut |n| names.push(n.to_string())).expect("install");
        for n in ["%editor-text", "%pane-append", "%set-text-rendering", "%xref-sites"] {
            assert!(names.iter().any(|m| m == n), "{n} is installed and named");
        }
        assert!(!names.iter().any(|m| m == "%xref-prelude-dirs"), "the modeler's own");

        // The editor: text in and out, and a whole number answers EXACT.
        eval(&mut interp, r#"(%editor-set-text 970 "(add-sphere :at")"#);
        assert_eq!(eval(&mut interp, "(%editor-text 970)"), Val::Str("(add-sphere :at".into()));
        assert_eq!(eval(&mut interp, "(%editor-dirty? 970)"), Val::Int(0));
        assert_eq!(eval(&mut interp, "(%editor-cursor 970)"), Val::List(vec![Val::Int(0), Val::Int(0)]));

        // A text pane: rows, and nothing is `#f`, never a truthy unspecified.
        eval(&mut interp, r#"(%pane-append 971 "normal" "one\ntwo")"#);
        assert_eq!(eval(&mut interp, "(%pane-row-count 971)"), Val::Int(2));
        assert_eq!(eval(&mut interp, "(%pane-row-text 971 1)"), Val::Str("two".into()));
        assert_eq!(eval(&mut interp, "(%pane-row-text 971 9)"), Val::Bool(false));

        // The index: a buffer's unsaved text answers where a name is.
        eval(&mut interp, r#"(%xref-update "/tmp/edit-binding-test.scm" "(define (foo x) x)")"#);
        let Val::List(sites) = eval(&mut interp, r#"(%xref-sites "foo")"#) else { panic!("a list") };
        let Some(Val::List(site)) = sites.first() else { panic!("one site: {sites:?}") };
        assert_eq!(site.get(1), Some(&Val::Int(0)), "line 0");
    }

    /// The shell on a bare interpreter: loads with only what `00-base.scm`
    /// says a host supplies, and runs a key through the chains — a prefix
    /// sequence, the typing-pane rule, an error landing in the echo — and a
    /// menu, by keys and by a right-press through the role registry.
    #[test]
    fn the_shell_runs_keys_and_menus_on_a_bare_interpreter() {
        let mut interp = scm::Interp::new().expect("interp");
        install(&mut interp, &mut |_| {}).expect("install");
        // What a host supplies (00-base.scm's list): ray's list and string
        // helpers, the focus prims, the pane under a point and a clock.
        eval(&mut interp, r#"
            (import (srfi 1))
            (define (list-sort less? xs)
              (let loop ((xs xs) (out '()))
                (if (null? xs) out
                    (loop (cdr xs)
                          (let ins ((ys out))
                            (cond ((null? ys) (list (car xs)))
                                  ((less? (car xs) (car ys)) (cons (car xs) ys))
                                  (else (cons (car ys) (ins (cdr ys))))))))))
            (define (string-split s d)
              (let loop ((i 0) (start 0) (acc '()))
                (cond ((= i (string-length s)) (reverse (cons (substring s start i) acc)))
                      ((char=? (string-ref s i) (string-ref d 0))
                       (loop (+ i 1) (+ i 1) (cons (substring s start i) acc)))
                      (else (loop (+ i 1) start acc)))))
            (define (string-prefix? p s)
              (and (<= (string-length p) (string-length s))
                   (string=? p (substring s 0 (string-length p)))))
            (define *typing* #f)
            (define (%focused-pane) 0)
            (define (%typing-pane) *typing*)
            (define (%pane-at x y) (list 0 "text"))
            (define (%os-clock) 0)"#);
        for name in SHELL {
            let path = shell_dir().join(format!("{name}.scm"));
            let src = std::fs::read_to_string(&path).expect("read");
            interp.eval_named(&src, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        eval(&mut interp, r#"
            (define ran '())
            (define echoed #f)
            (set! *echo* (lambda (m) (set! echoed m)))
            (bind-key "C-x C-f" (lambda () (set! ran (cons 'find-file ran))))
            (bind-key "q" (lambda () (set! ran (cons 'q ran))))
            (bind-key "C-c" (lambda () (raise "nothing to cancel")))"#);
        // A prefix arms on C-x and runs on C-f.
        eval(&mut interp, r#"(on-key "C-x") (on-key "C-f")"#);
        assert_eq!(eval(&mut interp, "ran"), Val::List(vec![Val::Sym("find-file".into())]));
        // While a pane takes typing, a bare key is text, not a command...
        eval(&mut interp, r#"(set! *typing* 0) (on-key "q")"#);
        assert_eq!(eval(&mut interp, "(length ran)"), Val::Int(1));
        // ...and a chorded one still reaches the map.
        eval(&mut interp, r#"(set! *typing* #f) (on-key "q")"#);
        assert_eq!(eval(&mut interp, "(car ran)"), Val::Sym("q".into()));
        // A command that raises reports through the echo, not the stderr log.
        eval(&mut interp, r#"(on-key "C-c")"#);
        assert_eq!(eval(&mut interp, "echoed"), Val::Str("nothing to cancel".into()));
        // A menu is data; the popup runs the row the keys move to.
        eval(&mut interp, r#"
            (define-menu :m (list (list "One" (lambda () (set! ran (cons 'one ran))))
                                  (list "Two" (lambda () (set! ran (cons 'two ran))))))
            (show-menu :m 10 10)"#);
        assert_eq!(eval(&mut interp, "(menu-open?)"), Val::Bool(true));
        eval(&mut interp, r#"(on-key "down") (on-key "return")"#);
        assert_eq!(eval(&mut interp, "(car ran)"), Val::Sym("two".into()));
        assert_eq!(eval(&mut interp, "(menu-open?)"), Val::Bool(false));
        // A right-press asks the host the pane's role, and the role's entry
        // names the menu; a quick click (no drag, no hold) posts it linear.
        eval(&mut interp, r#"
            (set! *pane-role* (lambda (id kind) :text))
            (set-pane-menu! :text (lambda (x y id shift) :m))
            (on-menu-press 5 5) (on-menu-release 5 5)"#);
        assert_eq!(eval(&mut interp, "(menu-open?)"), Val::Bool(true));
        eval(&mut interp, r#"(on-key "C-g")"#);
        // A role the host never named opens nothing.
        eval(&mut interp, r#"(set! *pane-role* (lambda (id kind) :elsewhere))"#);
        assert_eq!(eval(&mut interp, "(on-menu-press 5 5)"), Val::Bool(false));
        let keys = eval(&mut interp, "(describe-keys)");
        let Val::Str(keys) = keys else { panic!("{keys:?}") };
        assert!(keys.contains("C-x C-f — "), "{keys}");
    }
}
