//! **A buffer read by its language's own reader, in that language's own
//! process** (Kevin, 2026-11-17: "add the guile reader check"). scm's
//! Scheme is read by scm's reader, which is in the editor
//! (`scm_binding::read_check`); Guile's is in Guile — and scm's would mark
//! every `#:keyword` a Guile file has. So a language registered here is not
//! read as the text changes but ASKED about: this watcher keeps each file
//! buffer's newest text, the shell (15-guile) puts it to the running
//! process when that is idle, and what the process cannot read comes back
//! as a mark. Nothing waits on the process: until it answers, and whenever
//! it is not running, the buffer shows only what the editor can tell for
//! itself — brackets (`language/guile.rs`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use scm::Val;

use super::{val_num, val_text};
use crate::language::{Diagnostic, Severity};

/// A buffer bigger than this is not sent to be read on every pause.
const TOO_BIG: usize = 400_000;

#[derive(Default)]
struct Entry {
    lang: String,
    text: String,
    /// Counts the text's changes.
    version: u64,
    /// The version last handed to the shell to ask about.
    asked: u64,
    /// What the reader said of version `read`.
    read: u64,
    found: Vec<Diagnostic>,
}

#[derive(Default)]
struct Reads {
    languages: HashSet<String>,
    files: HashMap<String, Entry>,
}

thread_local! {
    static READS: RefCell<Reads> = RefCell::new(Reads::default());
}

struct ReadWatcher;

static WATCHER: ReadWatcher = ReadWatcher;

impl crate::editorpane::Watcher for ReadWatcher {
    fn text_changed(&self, path: &str, lang: &str, lines: &[&str]) {
        let watched = READS.with(|r| {
            let mut r = r.borrow_mut();
            if !r.languages.contains(lang) {
                r.files.remove(path);
                return false;
            }
            let e = r.files.entry(path.to_string()).or_default();
            e.lang = lang.to_string();
            e.text = lines.join("\n");
            e.version += 1;
            true
        });
        // The shell asks on the host's next turn round, not from inside a
        // draw.
        if watched {
            crate::wake::wake();
        }
    }

    /// The marks of the text as it is NOW: one the reader made of an older
    /// text points at where something was, not where it is.
    fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        READS.with(|r| r.borrow().files.get(path).filter(|e| e.read == e.version).map(|e| e.found.clone()).unwrap_or_default())
    }

    fn closed(&self, path: &str) {
        READS.with(|r| r.borrow_mut().files.remove(path));
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%read-watch language)` — have the buffers in LANGUAGE read by its
    // own process: their text is kept for %read-pending from now on.
    interp.define_fn("%read-watch", |args: &[Val]| {
        READS.with(|r| r.borrow_mut().languages.insert(val_text(args, 0).unwrap_or_default()));
        crate::editorpane::set_watcher(&WATCHER);
        Ok(Val::Bool(true))
    })?;
    // `(%read-pending language)` — a file buffer in LANGUAGE whose text has
    // changed since it was last asked about, as (path version text), or #f
    // when there is none. Handing it out counts as asking: call this only
    // when the question can be put.
    interp.define_fn("%read-pending", |args: &[Val]| {
        let lang = val_text(args, 0).unwrap_or_default();
        Ok(READS.with(|r| {
            let mut r = r.borrow_mut();
            let next = r.files.iter_mut().find(|(_, e)| e.lang == lang && e.asked != e.version && e.text.len() <= TOO_BIG);
            match next {
                Some((path, e)) => {
                    e.asked = e.version;
                    Val::List(vec![Val::Str(path.clone()), Val::Int(e.version as i64), Val::Str(e.text.clone())])
                }
                None => Val::Bool(false),
            }
        }))
    })?;
    // `(%read-result path version line column message)` — what the reader
    // said of VERSION of PATH's text: it stopped at LINE and COLUMN (from 0,
    // the column in characters, just PAST what it could not read) with
    // MESSAGE; or, LINE #f, it read it all. An answer about a text that has
    // since changed is dropped.
    interp.define_fn("%read-result", |args: &[Val]| {
        let path = val_text(args, 0).unwrap_or_default();
        let version = val_num(args, 1) as u64;
        let at = match args.get(2) {
            Some(Val::Int(_) | Val::Float(_)) => Some((val_num(args, 2) as usize, val_num(args, 3) as usize)),
            _ => None,
        };
        let message = val_text(args, 4).unwrap_or_default();
        READS.with(|r| {
            let mut r = r.borrow_mut();
            let Some(e) = r.files.get_mut(&path).filter(|e| e.version == version) else { return };
            e.read = version;
            let lines: Vec<&str> = e.text.split('\n').collect();
            // The text ran out inside something. Inside a bracket, the
            // editor has already marked its opener (`Language::diagnose`);
            // inside a string or a block comment, only the reader knows:
            // marked quietly at the end, as an open form is.
            let ran_out = ["end of file", "end of input", "EOF"].iter().any(|w| message.contains(w));
            e.found = match at {
                None => Vec::new(),
                Some(_) if ran_out => {
                    let language = crate::language::by_name(&e.lang);
                    let marked = language.is_some_and(|l| !l.diagnose(&lines, &l.scan(&lines)).is_empty());
                    let last = lines.len().saturating_sub(1);
                    let end = lines.get(last).map_or(0, |l| l.len());
                    if marked {
                        Vec::new()
                    } else {
                        let from = lines.get(last).and_then(|l| l.char_indices().last()).map_or(0, |(b, _)| b);
                        vec![Diagnostic { start: (last, from), end: (last, end.max(from + 1)), message: "a string or `#|` comment is never closed".into(), severity: Severity::Open }]
                    }
                }
                Some((line, column)) => {
                    let line = line.min(lines.len().saturating_sub(1));
                    let text = lines.get(line).copied().unwrap_or("");
                    // Characters to bytes; and back one, to what it choked on.
                    let byte = |col: usize| text.char_indices().nth(col).map_or(text.len(), |(b, _)| b);
                    let (from, to) = (byte(column.saturating_sub(1)), byte(column).max(byte(column.saturating_sub(1)) + 1).min(text.len().max(1)));
                    vec![Diagnostic { start: (line, from), end: (line, to), message, severity: Severity::Error }]
                }
            };
        });
        crate::host::mark_dirty_ui();
        Ok(Val::Bool(true))
    })?;
    Ok(vec!["%read-watch", "%read-pending", "%read-result"])
}
