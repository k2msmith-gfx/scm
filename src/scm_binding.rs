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
mod text;
mod xref;

/// Install every editor prim into `interp`, telling `installed` each name —
/// the modeler records them, so its prelude can unwrap the array records
/// it passes (`99-scheme-only.scm`, `%prim-names`).
pub fn install(interp: &mut scm::Interp, installed: &mut dyn FnMut(&str)) -> Result<(), scm::EmbedError> {
    for name in editor::install(interp)?.into_iter().chain(text::install(interp)?).chain(xref::install(interp)?) {
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
}
