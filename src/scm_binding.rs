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
        // '() is an empty list, not one empty string.
        Some(scm::Val::Null) => Vec::new(),
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

mod breaks;
mod clipboard;
mod editor;
mod menu;
pub mod remote;
pub mod standalone;
mod term;
mod text;
mod xref;

/// **The editor's shell, in Scheme** (E3): `edit/scm/`, in load order. A host
/// evaluates these after installing the prims and before its own prelude,
/// so its files can bind the shell's hooks and add to its chains — never the
/// other way round.
pub const SHELL: &[&str] = &["00-base", "01-handlers", "02-keymap", "03-menus", "04-minibuffer", "05-buffers", "06-help", "07-backend", "08-sends", "09-complete", "10-xref", "11-diagnostics", "12-error-pane", "13-term", "14-remote", "15-guile", "16-repl-commands", "17-files", "18-search", "19-ex"];

/// Where [`SHELL`]'s files are: read at boot from the tree this binary was
/// built from, as the modeler's own prelude is.
pub fn shell_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scm")
}

/// Install every editor prim into `interp`, telling `installed` each name —
/// the modeler records them, so its prelude can unwrap the array records
/// it passes (`99-scheme-only.scm`, `%prim-names`).
pub fn install(interp: &mut scm::Interp, installed: &mut dyn FnMut(&str)) -> Result<(), scm::EmbedError> {
    crate::language::scheme::set_reader(read_check);
    // `(%edit-path rel)` — a file of the edit crate's tree, as an absolute
    // path: Guile's serve.scm (E6), read where this binary was built, as
    // the shell's own files are.
    interp.define_fn("%edit-path", |args| {
        let rel = val_text(args, 0).unwrap_or_default();
        Ok(scm::Val::Str(shell_dir().join("..").join(rel).to_string_lossy().into_owned()))
    })?;
    installed("%edit-path");
    for name in editor::install(interp)?
        .into_iter()
        .chain(text::install(interp)?)
        .chain(xref::install(interp)?)
        .chain(menu::install(interp)?)
        .chain(clipboard::install(interp)?)
        .chain(breaks::install(interp)?)
        .chain(term::install(interp)?)
        .chain(remote::install(interp)?)
    {
        installed(name);
    }
    Ok(())
}

/// **Diagnostics as you type** (E3f): the editor's Scheme asks scm's own
/// reader whether a buffer reads, as `scm-lsp` does, on a fresh engine so
/// that reading allocates nothing in the session. The reader is the one a
/// send will meet, so what it marks is exactly what would fail to send.
fn read_check(text: &str) -> crate::language::scheme::ReadCheck {
    use crate::language::scheme::ReadCheck;
    let mut engine = scm::Engine::new();
    match scm::Reader::read_all(text, &mut engine) {
        Ok(_) => ReadCheck::Fine,
        Err(scm::ReadError::Incomplete) => ReadCheck::Incomplete,
        Err(scm::ReadError::Invalid { span, msg }) => ReadCheck::Invalid { start: span.start, end: span.end, message: msg },
    }
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
    /// sequence, the typing-pane rule, an error landing in the echo — a
    /// menu, by keys and by a right-press through the role registry, the
    /// minibuffer from a prompt to its callback, buffers from a scratch to a
    /// saved file and back, and the help screens.
    #[test]
    fn the_shell_runs_on_a_bare_interpreter() {
        let mut interp = scm::Interp::new().expect("interp");
        install(&mut interp, &mut |_| {}).expect("install");
        // What a host supplies (00-base.scm's list): ray's list and string
        // helpers and the file I/O — host/prelude.scm, as the standalone
        // program loads them — then the focus prims, the pane under a point,
        // a clock, the kernel's typing hold and a path resolver.
        let host = shell_dir().join("host/prelude.scm");
        interp.eval_named(&std::fs::read_to_string(&host).expect("host prelude"), &host.to_string_lossy()).expect("host prelude loads");
        eval(&mut interp, r#"
            (define *typing* #f)
            (define (%focused-pane) 0)
            (define (%typing-pane) *typing*)
            (define (%pane-at x y) (list 0 "text"))
            (define (%os-clock) 0)
            (define (%hold-typing id) (set! *typing* id))
            (define (%release-typing) (set! *typing* #f))
            (define (%os-realpath p) p)"#);
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
        // The minibuffer: a prompt holds typing, text arrives through
        // on-text, backspace edits, return hands it to the callback, C-g
        // aborts a prompt, and echo lands in the minibuffer's pane.
        eval(&mut interp, r#"
            (set! *pane-id* (lambda (key) (and (eq? key :minibuffer) 7)))
            (bind-editor-keys :minibuffer)
            (define answer #f)
            (read-minibuffer "Name: " (lambda (s) (set! answer s)))"#);
        assert_eq!(eval(&mut interp, "*typing*"), Val::Int(7), "the prompt holds typing");
        eval(&mut interp, r#"(set! %focused-pane (lambda () 7))
                             (on-text "x") (on-key "backspace")
                             (on-text "a") (on-text "b") (on-key "return")"#);
        assert_eq!(eval(&mut interp, "answer"), Val::Str("ab".into()));
        assert_eq!(eval(&mut interp, "*typing*"), Val::Bool(false), "submitting released it");
        eval(&mut interp, r#"(set! answer #f)
                             (read-minibuffer "Name: " (lambda (s) (set! answer s)))
                             (on-text "z") (on-key "C-g")"#);
        assert_eq!(eval(&mut interp, "*typing*"), Val::Bool(false), "C-g aborted the prompt");
        assert_eq!(eval(&mut interp, "answer"), Val::Bool(false), "and ran nothing");
        eval(&mut interp, r#"(echo "saved " 3 " files")"#);
        assert_eq!(eval(&mut interp, "(%pane-row-text 7 (- (%pane-row-count 7) 1))"), Val::Str("saved 3 files".into()));
        // Buffers: a scratch is named, saved as a file, switched away from
        // and back by name, and killing the last one leaves a fresh scratch.
        let path = std::env::temp_dir().join(format!("edit-shell-test-{}.scm", std::process::id()));
        let path = path.to_string_lossy().replace('\\', "/");
        eval(&mut interp, &format!(r#"
            (set! *pane-id* (lambda (key) (case key ((:minibuffer) 7) ((:editor) 3) (else #f))))
            (register-buffer (%editor-active-buffer 3) #f)
            (%editor-set-text 3 "(define x 1)")
            (save-buffer "{path}")"#));
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), Some("(define x 1)"));
        let base = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(eval(&mut interp, "(buffer-names)"), Val::List(vec![Val::Str(base.clone())]));
        eval(&mut interp, r#"(let ((b (%editor-buffer-new 3)))
                               (register-buffer b #f) (visit-buffer b))"#);
        assert_eq!(eval(&mut interp, "(buffer-name (%editor-active-buffer 3))"), Val::Str("*scratch*".into()));
        eval(&mut interp, &format!(r#"(visit-buffer (buffer-by-name "{base}"))"#));
        assert_eq!(eval(&mut interp, "(%editor-text 3)"), Val::Str("(define x 1)".into()));
        eval(&mut interp, "(kill-buffer) (kill-buffer)");
        assert_eq!(eval(&mut interp, "(buffer-names)"), Val::List(vec![Val::Str("*scratch*".into())]));
        let _ = std::fs::remove_file(&path);
        // Help: show-keys lists the bindings and the host's extra sections;
        // C-h k then a sequence opens the command's page in a view-only
        // buffer; C-h b is the whole keymap as a document.
        eval(&mut interp, r#"
            (define helped '())
            (set! *transcript-append* (lambda (text (style #f)) (set! helped (cons text helped))))
            (set! *show-keys-extra* (lambda () (list (cons "── mouse" '(("drag" "select"))))))
            (show-keys)"#);
        let helped = eval(&mut interp, "(apply string-append helped)");
        let Val::Str(helped) = helped else { panic!("{helped:?}") };
        assert!(helped.contains("C-x C-f") && helped.contains("── mouse"), "{helped}");
        eval(&mut interp, r#"(describe-key) (on-key "C-x") (on-key "C-f")"#);
        assert_eq!(eval(&mut interp, "(%editor-markdown? 3)"), Val::Int(1), "the page is a view-only buffer");
        assert!(eval(&mut interp, r#"(buffer-by-name "*describe-key*")"#) != Val::Bool(false));
        assert_eq!(eval(&mut interp, "(length ran)"), Val::Int(3), "described, not run");
        let doc = eval(&mut interp, "(keybindings-markdown)");
        let Val::Str(doc) = doc else { panic!("{doc:?}") };
        assert!(doc.starts_with("# Key bindings") && doc.contains("`C-x C-f`"), "{doc}");
        // A send: C-M-x on the second form evaluates it through the
        // buffer's backend, and its value lands in the transcript.
        eval(&mut interp, r#"
            (set! helped '())
            (visit-buffer (buffer-by-name "*scratch*"))
            (%editor-set-text 3 "(define (sq x) (* x x))\n(sq 7)")
            (%editor-goto-line 3 0) (eval-defun)
            (%editor-goto-line 3 1) (eval-defun)"#);
        let sent = eval(&mut interp, "(apply string-append helped)");
        let Val::Str(sent) = sent else { panic!("{sent:?}") };
        assert!(sent.contains(";; defun: (sq 7)") && sent.contains("=> 49"), "{sent}");
        assert!(sent.contains("=> sq") && !sent.contains("#<unspecified>"), "a definition answers its name: {sent}");
        // Describe: on an argument, the form's head is described, from the
        // buffer's backend.
        eval(&mut interp, r#"
            (define (twice x) "(twice x)\n\nDoubles X." (* 2 x))
            (set! helped '())
            (%editor-set-text 3 "(twice 4")
            (%editor-act 3 "buffer-end")
            (editor-describe)"#);
        let described = eval(&mut interp, "(apply string-append helped)");
        let Val::Str(described) = described else { panic!("{described:?}") };
        assert!(described.contains("── (doc twice)") && described.contains("Doubles X."), "{described}");
        // M-.: a name defined in an indexed file opens that file at the
        // definition; M-, comes back.
        let lib = std::env::temp_dir().join(format!("edit-xref-test-{}.scm", std::process::id()));
        std::fs::write(&lib, "; a library\n(define (helper) 1)\n").unwrap();
        let lib = lib.to_string_lossy().replace('\\', "/");
        eval(&mut interp, &format!(r#"
            (%xref-index (list "{lib}"))
            (define from (%editor-active-buffer 3))
            (jump-to-definition "helper")"#));
        assert_eq!(eval(&mut interp, "(buffer-path (%editor-active-buffer 3))"), Val::Str(lib.clone()));
        assert_eq!(eval(&mut interp, "(%editor-cursor 3)"), Val::List(vec![Val::Int(1), Val::Int(9)]), "on the name");
        eval(&mut interp, "(go-back-from-definition)");
        assert_eq!(eval(&mut interp, "(equal? (%editor-active-buffer 3) from)"), Val::Bool(true));
        let _ = std::fs::remove_file(&lib);
        // The in-process backend answers a RESULT: what the forms printed,
        // the value, or an error as data rather than an unwind.
        eval(&mut interp, r#"(define r (in-process-eval "(display 1) (+ 2 3)"))
                             (define e (in-process-eval "(car 5)"))"#);
        assert_eq!(eval(&mut interp, "(%alist-ref r :status #f)"), Val::Sym("ok".into()));
        assert_eq!(eval(&mut interp, "(%alist-ref r :value #f)"), Val::Int(5));
        assert_eq!(eval(&mut interp, "(%alist-ref r :output #f)"), Val::Str("1".into()));
        assert_eq!(eval(&mut interp, "(%alist-ref e :status #f)"), Val::Sym("error".into()));
        assert!(eval(&mut interp, "(backend-op (in-process-backend) :definition-sites)") != Val::Bool(false), "all five offered");
        // An error's message is what a guard around the send would have
        // printed, whichever way it was raised.
        for (src, message) in [("(car 5)", "car: not a pair: 5"), ("(raise 'sym)", "sym"), (r#"(error \"boom\" 1)"#, "boom 1")] {
            let m = eval(&mut interp, &format!(r#"(%alist-ref (in-process-eval "{src}") :message #f)"#));
            assert_eq!(m, Val::Str(message.into()), "{src}");
        }
        // An error the sent code catches is its own business: no break.
        eval(&mut interp, r#"(define c (in-process-eval "(guard (e (#t 'caught)) (car 1))"))"#);
        assert_eq!(eval(&mut interp, "(%alist-ref c :value #f)"), Val::Sym("caught".into()));

        // E3f, the reader: text that will not read is marked where the
        // reader stops; a form left open, quietly, at its outermost opener;
        // M-g n goes there and says why.
        eval(&mut interp, r#"(let ((b (%editor-buffer-new 3))) (register-buffer b #f) (visit-buffer b))
                             (%editor-set-text 3 "(define (f x)\n  (car x))\n(f 1 #<)")"#);
        let d = eval(&mut interp, "(car (%editor-diagnostics 3))");
        let Val::List(d) = d else { panic!("{d:?}") };
        assert_eq!((&d[0], &d[3]), (&Val::Int(2), &Val::Int(1)), "an error, on line 3: {d:?}");
        eval(&mut interp, r#"(%editor-set-text 3 "(ok)\n(open (more)\n  (x)")"#);
        assert_eq!(eval(&mut interp, "(map (lambda (d) (list (car d) (cadr d) (cadddr d))) (%editor-diagnostics 3))"),
                   Val::List(vec![Val::List(vec![Val::Int(1), Val::Int(0), Val::Int(0)])]), "open at (open");
        eval(&mut interp, "(goto-diagnostic)");
        assert_eq!(eval(&mut interp, "(%editor-cursor 3)"), Val::List(vec![Val::Int(1), Val::Int(0)]));
        eval(&mut interp, r#"(%editor-set-text 3 "(fine)")"#);
        assert_eq!(eval(&mut interp, "(null? (%editor-diagnostics 3))"), Val::Bool(true));
        // A file that is not Scheme is not read as Scheme.
        eval(&mut interp, r#"(%editor-set-text 3 "{ \"json\": [1, 2 }")
                             (%buffer-set-path 3 (%editor-active-buffer 3) "/tmp/x.json")"#);
        assert_eq!(eval(&mut interp, "(null? (%editor-diagnostics 3))"), Val::Bool(true));
        eval(&mut interp, "(%buffer-set-path 3 (%editor-active-buffer 3) #f)");

        // Document symbols: what the buffer defines, name:LINE in order,
        // as the minibuffer's list; RET on one goes there.
        eval(&mut interp, r#"(%editor-set-text 3 "(define (f x) x)\n\n(define y 2)")"#);
        assert_eq!(eval(&mut interp, "(map definition-label (%text-definitions (%editor-text 3)))"),
                   Val::List(vec![Val::Str("f:1".into()), Val::Str("y:3".into())]));
        // (RET takes the highlighted candidate, as in M-x; RET again goes.)
        eval(&mut interp, r#"(document-symbols) (on-text "y") (on-key "return") (on-key "return")"#);
        assert_eq!(eval(&mut interp, "(%editor-cursor 3)"), Val::List(vec![Val::Int(2), Val::Int(8)]), "on y");

        // The error pane: a send that raises keeps the calls it raised in,
        // innermost first, each with its locals — none of them the send's
        // own machinery — and C-h e shows them.
        eval(&mut interp, r#"
            (in-process-eval "(define (inner v) (vector-ref v 9)) (define (outer n) (inner (make-vector n 0)))")
            (set! helped '())
            (%editor-set-text 3 "(outer 3)")
            (%editor-goto-line 3 0) (eval-defun)"#);
        let frames = eval(&mut interp, "(list-ref *last-error* 3)");
        let Val::List(frames) = frames else { panic!("{frames:?}") };
        let Val::List(first) = &frames[0] else { panic!("{frames:?}") };
        assert_eq!(first[0], Val::Str("(vector-ref v 9)".into()), "{frames:?}");
        let locals = format!("{:?}", first[2]);
        assert!(locals.contains("\"v\"") && locals.contains("#(0 0 0)"), "{locals}");
        assert!(!format!("{frames:?}").contains("07-backend"), "{frames:?}");
        let sent = eval(&mut interp, "(apply string-append helped)");
        let Val::Str(sent) = sent else { panic!("{sent:?}") };
        assert!(sent.contains("C-h e"), "{sent}");
        eval(&mut interp, "(view-last-error)");
        assert!(eval(&mut interp, r#"(buffer-by-name "*error*")"#) != Val::Bool(false));
        let page = eval(&mut interp, "(error-markdown *last-error*)");
        let Val::Str(page) = page else { panic!("{page:?}") };
        assert!(page.starts_with("# vector-ref") && page.contains("## 1 · `(vector-ref v 9)`"), "{page}");

        let keys = eval(&mut interp, "(describe-keys)");
        let Val::Str(keys) = keys else { panic!("{keys:?}") };
        assert!(keys.contains("C-x C-f — "), "{keys}");
    }
}
