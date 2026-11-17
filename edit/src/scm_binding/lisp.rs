//! The prims of what a send reads from a Common Lisp buffer (docs/sbcl-
//! plan.md, L3): the top-level form at the point, the expression before
//! it, the package the buffer's text is read in there, and whether a REPL
//! line is whole — each off Common Lisp's own tokens (`language::lisp`),
//! so `|a (b|` is one symbol here as in the highlighting.

use scm::Val;

use super::{val_num, val_text};
use crate::language::lisp;

fn span(found: Option<(usize, usize)>) -> Val {
    match found {
        Some((a, b)) => Val::List(vec![Val::Int(a as i64), Val::Int(b as i64)]),
        None => Val::Bool(false),
    }
}

/// TEXT and the point (LINE, BYTE) as a character offset into it.
fn text_and_point(args: &[Val]) -> (String, usize) {
    let text = val_text(args, 0).unwrap_or_default();
    let p = lisp::char_offset(&text, val_num(args, 1) as usize, val_num(args, 2) as usize);
    (text, p)
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%lisp-form-at text line byte)` — the top-level form of Lisp TEXT the
    // point is in, else the nearest before it, as (start end) characters;
    // #f when there is none. `(%lisp-form-before text line byte)` — the
    // expression that ends just before the point, likewise.
    interp.define_fn("%lisp-form-at", |args: &[Val]| {
        let (text, p) = text_and_point(args);
        Ok(span(lisp::form_at(&text, p)))
    })?;
    interp.define_fn("%lisp-form-before", |args: &[Val]| {
        let (text, p) = text_and_point(args);
        Ok(span(lisp::form_before(&text, p)))
    })?;
    // `(%lisp-package text line byte)` — the package a form at the point is
    // read in, from the last `(in-package …)` above it, as `find-package`
    // wants the name; #f when there is none.
    interp.define_fn("%lisp-package", |args: &[Val]| {
        let (text, p) = text_and_point(args);
        Ok(lisp::package_at(&text, p).map_or(Val::Bool(false), Val::Str))
    })?;
    // `(%lisp-complete? text)` — is Lisp TEXT whole: every `(` closed, no
    // string, `|…|` or `#|` left open?
    interp.define_fn("%lisp-complete?", |args: &[Val]| Ok(Val::Bool(lisp::complete(&val_text(args, 0).unwrap_or_default()))))?;
    Ok(vec!["%lisp-form-at", "%lisp-form-before", "%lisp-package", "%lisp-complete?"])
}
