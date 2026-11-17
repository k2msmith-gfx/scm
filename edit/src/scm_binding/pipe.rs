//! The prims of a REPL over pipes (`crate::pipe`) — evcxr, Rust's — and of
//! what a send to one reads from a Rust buffer: the statement at the point
//! and the expression before it (`language::rust`).

use scm::Val;

use super::{val_num, val_text, val_textv};
use crate::language::rust;

/// The (line, byte) of a place as the CHARACTER offset the shell's sends
/// count in — into the buffer's text, its lines joined by one newline.
fn offset(lines: &[&str], (line, byte): (usize, usize)) -> i64 {
    let before: usize = lines.iter().take(line).map(|l| l.chars().count() + 1).sum();
    (before + lines.get(line).map_or(0, |l| l[..byte.min(l.len())].chars().count())) as i64
}

fn span(lines: &[&str], found: Option<rust::Span>) -> Val {
    match found {
        Some((from, to)) => Val::List(vec![Val::Int(offset(lines, from)), Val::Int(offset(lines, to))]),
        None => Val::Bool(false),
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%pipe-which program)` — where PROGRAM is (itself, the PATH's, or
    // cargo's bin), or #f.
    interp.define_fn("%pipe-which", |args: &[Val]| {
        Ok(crate::pipe::which(&val_text(args, 0).unwrap_or_default()).map_or(Val::Bool(false), |p| Val::Str(p.to_string_lossy().into_owned())))
    })?;
    // `(%rust-toolchain cargo)` — (cargo's path, rustc's version) as far as
    // each is here, else #f in its place: what the "no evcxr" message reads
    // to say how to get one on THIS machine.
    interp.define_fn("%rust-toolchain", |args: &[Val]| {
        let cargo = crate::pipe::which(&val_text(args, 0).unwrap_or_else(|| "cargo".into()));
        // rustc beside that cargo, else the PATH's.
        let rustc = cargo.as_ref().and_then(|c| Some(c.parent()?.join("rustc")).filter(|p| p.is_file())).or_else(|| crate::pipe::which("rustc"));
        let version = rustc.and_then(|r| std::process::Command::new(r).arg("--version").output().ok()).and_then(|o| {
            // "rustc 1.94.0 (… 2026-…)" -> "1.94.0"
            String::from_utf8_lossy(&o.stdout).split_whitespace().nth(1).map(str::to_string)
        });
        let or_false = |v: Option<String>| v.map_or(Val::Bool(false), Val::Str);
        Ok(Val::List(vec![or_false(cargo.map(|c| c.to_string_lossy().into_owned())), or_false(version)]))
    })?;
    // `(%pipe-start program args cwd ok failed prompt)` — start PROGRAM, a
    // REPL read a line at a time whose answers end in the text OK or
    // FAILED and which writes PROMPT before it reads: its id. Raises with
    // the reason when it cannot start.
    interp.define_fn("%pipe-start", |args: &[Val]| {
        let program = val_text(args, 0).ok_or("expected a program")?;
        let cwd = val_text(args, 2);
        let framing = crate::pipe::Framing {
            ok: val_text(args, 3).unwrap_or_default(),
            failed: val_text(args, 4).unwrap_or_default(),
            prompt: val_text(args, 5).unwrap_or_default(),
        };
        let id = crate::pipe::start(&program, &val_textv(args, 1), cwd.as_deref().map(std::path::Path::new), framing)?;
        Ok(Val::Int(id as i64))
    })?;
    // `(%pipe-send id line)` — put one LINE to REPL ID: the request's
    // number, whose answer %pipe-poll brings. Raises when it has exited.
    interp.define_fn("%pipe-send", |args: &[Val]| {
        Ok(Val::Int(crate::pipe::send(val_num(args, 0) as u32, &val_text(args, 1).unwrap_or_default())? as i64))
    })?;
    // `(%pipe-poll)` — every answer since the last call, oldest first, each
    // (id request ok? output).
    interp.define_fn("%pipe-poll", |_args: &[Val]| {
        Ok(Val::List(
            crate::pipe::poll()
                .into_iter()
                .map(|a| Val::List(vec![Val::Int(a.repl as i64), Val::Int(a.request as i64), Val::Bool(a.ok), Val::Str(a.output)]))
                .collect(),
        ))
    })?;
    // `(%pipe-pending id)` — how many requests REPL ID has yet to answer.
    interp.define_fn("%pipe-pending", |args: &[Val]| Ok(Val::Int(crate::pipe::pending(val_num(args, 0) as u32) as i64)))?;
    // `(%pipe-alive? id)` — is REPL ID running?
    interp.define_fn("%pipe-alive?", |args: &[Val]| Ok(Val::Bool(crate::pipe::alive(val_num(args, 0) as u32))))?;
    // `(%pipe-interrupt id)` — Ctrl-C to REPL ID; #f with nothing to stop.
    interp.define_fn("%pipe-interrupt", |args: &[Val]| Ok(Val::Bool(crate::pipe::interrupt(val_num(args, 0) as u32))))?;
    // `(%pipe-stop id)` — stop REPL ID.
    interp.define_fn("%pipe-stop", |args: &[Val]| {
        crate::pipe::stop(val_num(args, 0) as u32);
        Ok(Val::Bool(false))
    })?;
    // `(%rust-statement-at text line byte)` — the top-level statement or
    // item of Rust TEXT the point (LINE, BYTE in it) is in, else the
    // nearest before it, as (start end) character offsets into TEXT; #f
    // when there is none. `(%rust-expression-before text line byte)` — the
    // expression that ends just before the point, likewise.
    interp.define_fn("%rust-statement-at", |args: &[Val]| {
        let text = val_text(args, 0).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').collect();
        Ok(span(&lines, rust::statement_at(&lines, val_num(args, 1) as usize, val_num(args, 2) as usize)))
    })?;
    interp.define_fn("%rust-expression-before", |args: &[Val]| {
        let text = val_text(args, 0).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').collect();
        Ok(span(&lines, rust::expression_before(&lines, val_num(args, 1) as usize, val_num(args, 2) as usize)))
    })?;
    // `(%rust-statements text)` — every top-level statement and item of
    // Rust TEXT, in order, each (start end).
    interp.define_fn("%rust-statements", |args: &[Val]| {
        let text = val_text(args, 0).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').collect();
        Ok(Val::List(rust::statements(&lines).into_iter().map(|s| span(&lines, Some(s))).collect()))
    })?;
    // `(%rust-complete? text)` — is Rust TEXT whole: every bracket, string
    // and comment closed?
    interp.define_fn("%rust-complete?", |args: &[Val]| {
        let text = val_text(args, 0).unwrap_or_default();
        Ok(Val::Bool(rust::complete(&text.split('\n').collect::<Vec<_>>())))
    })?;
    // `(%rust-defined-name text)` — "fn area", "struct P", "let x": what
    // Rust TEXT defines when it is one item, else #f.
    interp.define_fn("%rust-defined-name", |args: &[Val]| {
        let text = val_text(args, 0).unwrap_or_default();
        Ok(rust::defined_name(&text.split('\n').collect::<Vec<_>>()).map_or(Val::Bool(false), Val::Str))
    })?;
    Ok(vec![
        "%pipe-which",
        "%rust-toolchain",
        "%pipe-start",
        "%pipe-send",
        "%pipe-poll",
        "%pipe-pending",
        "%pipe-alive?",
        "%pipe-interrupt",
        "%pipe-stop",
        "%rust-statement-at",
        "%rust-expression-before",
        "%rust-statements",
        "%rust-complete?",
        "%rust-defined-name",
    ])
}
