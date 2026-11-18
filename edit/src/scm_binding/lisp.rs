//! The prims of what a send reads from a Common Lisp buffer (docs/sbcl-
//! plan.md, L3): the top-level form at the point, the expression before
//! it, the package the buffer's text is read in there, and whether a REPL
//! line is whole — each off Common Lisp's own tokens (`language::lisp`),
//! so `|a (b|` is one symbol here as in the highlighting. And M-.'s index
//! (L5): where a project's Lisp files define a name, loaded or not. And
//! the compiler's warnings marked at the forms they were said of (L6).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use scm::Val;

use super::{val_num, val_text};
use crate::language::{Diagnostic, Severity, lisp};

/// What SBCL's compiler said of a Lisp file's forms when they were sent: by
/// file, each warned form's [`lisp::form_key`] and what was said — kept by
/// KEY, not place, so a mark follows its `defun` as the text around it
/// moves, and goes when the form is sent again and nothing is said.
#[derive(Default)]
struct Warned {
    marks: HashMap<String, Vec<(String, Vec<String>)>>,
    /// Each Lisp file's newest text, and the marks placed in it.
    texts: HashMap<String, String>,
    placed: HashMap<String, Vec<Diagnostic>>,
}

thread_local! {
    static WARNED: RefCell<Warned> = RefCell::new(Warned::default());
}

impl Warned {
    /// Place PATH's marks in its newest text: each at the head and name of
    /// the top-level form with its key.
    fn place(&mut self, path: &str) {
        let (Some(marks), Some(text)) = (self.marks.get(path), self.texts.get(path)) else {
            self.placed.remove(path);
            return;
        };
        let chars: Vec<char> = text.chars().collect();
        let line_col = |p: usize| {
            let before: String = chars[..p.min(chars.len())].iter().collect();
            let line = before.matches('\n').count();
            let col_byte = before.rsplit('\n').next().map_or(0, str::len);
            (line, col_byte)
        };
        let mut out = Vec::new();
        for (a, b) in lisp::toplevel_forms(text) {
            let form: String = chars[a..b].iter().collect();
            let key = lisp::form_key(&form);
            if let Some((_, said)) = marks.iter().find(|(k, _)| *k == key) {
                // To the end of the form's first line: its head and name.
                let first = form.split('\n').next().unwrap_or("");
                let start = line_col(a);
                let end = (start.0, start.1 + first.len());
                let message = said.iter().map(|w| w.trim_start_matches("; ")).collect::<Vec<_>>().join(" · ");
                out.push(Diagnostic { start, end, message, severity: Severity::Warning });
            }
        }
        self.placed.insert(path.to_string(), out);
    }
}

struct WarnWatcher;

static WATCHER: WarnWatcher = WarnWatcher;

impl crate::editorpane::Watcher for WarnWatcher {
    fn text_changed(&self, path: &str, lang: &str, lines: &[&str]) {
        if lang != "lisp" {
            return;
        }
        WARNED.with(|w| {
            let mut w = w.borrow_mut();
            w.texts.insert(path.to_string(), lines.join("\n"));
            if w.marks.contains_key(path) {
                w.place(path);
            }
        });
    }

    fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        WARNED.with(|w| w.borrow().placed.get(path).cloned().unwrap_or_default())
    }

    fn closed(&self, path: &str) {
        WARNED.with(|w| {
            let mut w = w.borrow_mut();
            w.texts.remove(path);
            w.placed.remove(path);
        });
    }
}

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
    // From the start: the warnings are placed in a file's newest text,
    // which the watcher hears of only as it changes.
    crate::editorpane::set_watcher(&WATCHER);
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
    // `(%lisp-warned path run starts warnings)` — RUN, text of the file at
    // PATH, was sent, and SBCL's compiler said WARNINGS of it, each of the
    // form beginning at the character offset in STARTS beside it. Every
    // form RUN holds loses its old marks; each warned one is marked.
    interp.define_fn("%lisp-warned", |args: &[Val]| {
        let path = val_text(args, 0).unwrap_or_default();
        let run = val_text(args, 1).unwrap_or_default();
        let starts: Vec<usize> = match args.get(2) {
            Some(Val::List(v)) => v.iter().map(|x| if let Val::Int(n) = x { (*n).max(0) as usize } else { usize::MAX }).collect(),
            _ => Vec::new(),
        };
        let said: Vec<String> = match args.get(3) {
            Some(Val::List(v)) => v.iter().map(|x| if let Val::Str(t) = x { t.clone() } else { String::new() }).collect(),
            _ => Vec::new(),
        };
        let chars: Vec<char> = run.chars().collect();
        let forms: Vec<(usize, usize, String)> =
            lisp::toplevel_forms(&run).into_iter().map(|(a, b)| (a, b, lisp::form_key(&chars[a..b].iter().collect::<String>()))).collect();
        WARNED.with(|w| {
            let mut w = w.borrow_mut();
            let marks = w.marks.entry(path.clone()).or_default();
            marks.retain(|(k, _)| !forms.iter().any(|(_, _, f)| f == k));
            for (start, text) in starts.iter().zip(&said) {
                let Some((_, _, key)) = forms.iter().find(|(a, b, _)| a <= start && start < b) else { continue };
                match marks.iter_mut().find(|(k, _)| k == key) {
                    Some((_, list)) => list.push(text.clone()),
                    None => marks.push((key.clone(), vec![text.clone()])),
                }
            }
            if marks.is_empty() {
                w.marks.remove(&path);
            }
            w.place(&path);
        });
        crate::host::mark_dirty_ui();
        Ok(Val::Bool(true))
    })?;
    Ok(vec!["%lisp-form-at", "%lisp-form-before", "%lisp-package", "%lisp-complete?", "%lisp-definitions", "%lisp-warned"])
}
