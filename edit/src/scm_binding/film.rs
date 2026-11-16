//! The film prims (`crate::film`): what a film script asks of the program,
//! queued for it to apply after the form — input injected through the same
//! code a real event takes, a frame saved, a caption set. Installed only in
//! film mode (`Session::film`): the modeler has its own prims by these names.

use scm::Val;

use super::{val_num, val_text};
use crate::film::{Op, push};

fn queued(op: Op) -> Result<Val, String> {
    push(op);
    Ok(Val::Bool(false))
}

pub fn install(interp: &mut scm::Interp) -> Result<(), scm::EmbedError> {
    // `(%inject-key "C-x")` — a key press, as the window would deliver it.
    interp.define_fn("%inject-key", |args| queued(Op::Key(val_text(args, 0).ok_or("expected a key")?)))?;
    // `(%inject-mouse what [a b])` — the pointer (see `Op::Mouse`).
    interp.define_fn("%inject-mouse", |args| {
        queued(Op::Mouse(val_text(args, 0).ok_or("expected what")?, val_num(args, 1), val_num(args, 2)))
    })?;
    // `(%screenshot-raw "f.png")` — the frame as it stands, caption and all.
    interp.define_fn("%screenshot-raw", |args| queued(Op::Shot(val_text(args, 0).ok_or("expected a path")?)))?;
    // `(%film-icon "files"|"repl"|"terminal")` — the centre of that strip
    // icon as last drawn, (x y), for the pointer to go to.
    interp.define_fn("%film-icon", |args| {
        let want = val_text(args, 0).unwrap_or_default();
        let found = crate::app::icon_rects().into_iter().find(|(i, _)| {
            matches!((i, want.as_str()), (crate::app::Icon::Files, "files") | (crate::app::Icon::Repl, "repl") | (crate::app::Icon::Terminal, "terminal"))
        });
        Ok(match found {
            Some((_, r)) => Val::List(vec![Val::Float(r.x as f64 + r.w as f64 / 2.0), Val::Float(r.y as f64 + r.h as f64 / 2.0)]),
            None => Val::Bool(false),
        })
    })?;
    // `(caption "text")` — the caption under the editor; `(caption)` clears it.
    interp.define_fn("caption", |args| queued(Op::Caption(val_text(args, 0))))?;
    Ok(())
}
