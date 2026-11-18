//! The prims of what a send reads from a Common Lisp buffer (docs/sbcl-
//! plan.md, L3): the top-level form at the point, the expression before
//! it, the package the buffer's text is read in there, and whether a REPL
//! line is whole — each off Common Lisp's own tokens (`language::lisp`),
//! so `|a (b|` is one symbol here as in the highlighting. And M-.'s index
//! (L5): where a project's Lisp files define a name, loaded or not.

use std::path::{Path, PathBuf};

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

/// The files a Lisp project's M-. searches: its root is the nearest
/// directory above FILE holding a `.asd` or `.git`, else FILE's own; every
/// `.lisp`, `.lsp`, `.cl` and `.asd` under it, past hidden directories and
/// build output, at most [`MAX_FILES`].
const MAX_FILES: usize = 3000;

fn project_root(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    let marked = |d: &Path| {
        d.join(".git").exists()
            || std::fs::read_dir(d).is_ok_and(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "asd")))
    };
    dir.ancestors().find(|d| marked(d)).unwrap_or(dir).to_path_buf()
}

fn lisp_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if !name.starts_with('.') && !matches!(name, "target" | "node_modules" | "build") {
                    dirs.push(p);
                }
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x, "lisp" | "lsp" | "cl" | "asd")) {
                out.push(p);
                if out.len() >= MAX_FILES {
                    return out;
                }
            }
        }
    }
    out
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
    // `(%lisp-definitions file name buffers)` — where the Lisp files of
    // FILE's project define NAME: ((path line column form) …), 0-based, the
    // column in characters at the name. BUFFERS, ((path text) …), are open
    // buffers whose text counts as it stands, saved or not.
    interp.define_fn("%lisp-definitions", |args: &[Val]| {
        let file = PathBuf::from(val_text(args, 0).unwrap_or_default());
        let name = lisp::bare_name(&val_text(args, 1).unwrap_or_default());
        let mut open: Vec<(PathBuf, String)> = Vec::new();
        if let Some(Val::List(bufs)) = args.get(2) {
            for b in bufs {
                if let Val::List(pt) = b
                    && let [Val::Str(p), Val::Str(t)] = pt.as_slice()
                {
                    open.push((PathBuf::from(p), t.clone()));
                }
            }
        }
        let mut files = lisp_files(&project_root(&file));
        // An open buffer outside the project still counts.
        files.extend(open.iter().map(|(p, _)| p.clone()).filter(|p| !files.contains(p)).collect::<Vec<_>>());
        let mut out = Vec::new();
        for path in files {
            let text = match open.iter().find(|(p, _)| *p == path) {
                Some((_, t)) => t.clone(),
                None => match std::fs::metadata(&path) {
                    Ok(m) if m.len() <= 2_000_000 => std::fs::read_to_string(&path).unwrap_or_default(),
                    _ => continue,
                },
            };
            // Cheap first: a file that never spells the name defines nothing.
            if !text.to_ascii_lowercase().contains(&name.to_ascii_lowercase()) {
                continue;
            }
            for (line, column, form) in lisp::definitions(&text, &name) {
                out.push(Val::List(vec![
                    Val::Str(path.to_string_lossy().into_owned()),
                    Val::Int(line as i64),
                    Val::Int(column as i64),
                    Val::Str(form),
                ]));
            }
        }
        Ok(Val::List(out))
    })?;
    Ok(vec!["%lisp-form-at", "%lisp-form-before", "%lisp-package", "%lisp-complete?", "%lisp-definitions"])
}
