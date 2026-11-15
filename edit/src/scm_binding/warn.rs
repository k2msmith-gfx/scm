//! **Warnings before you run** (docs/editor-crate-plan.md, E8): in a Scheme
//! buffer, a name that nothing binds, and a call with a count of arguments
//! its procedure does not take — underlined as you type, in the warning
//! ink, before anything is sent.
//!
//! What is bound is the IMAGE's to say — the one the buffer's sends go to,
//! which knows more than any reading of the text — plus what this file and
//! the indexed files define. The shell tells this watcher what an image
//! binds (`%warn-facts`, after each send) and which image a buffer is
//! judged against (`%warn-buffer`); a buffer with none is not judged, which
//! is the right answer for a ray file before its ray has started.
//!
//! Which names are FREE is `scm-text`'s scope pass, which binds generously:
//! a warning it would get wrong is a missed one, not a false one. A use
//! inside a form whose head is a macro the pass does not know is not judged
//! either, since a macro decides what its arguments are.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::language::{Diagnostic, Severity};

/// What a procedure accepts, as an arity check needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arity {
    pub required: usize,
    pub optional: usize,
    pub rest: bool,
    pub keys: bool,
}

/// A call shape as `signature` prints it, or as a file writes it:
/// `(f a [b] :at at . rest)`, `(f a &opt b)`, `(f a (b 1))`. None for one
/// that says nothing (`(car …)`, a primitive) or is not a plain lambda list.
pub(crate) fn arity_of(sig: &str) -> Option<Arity> {
    use scm_text::text::Kind;
    let doc = scm_text::Doc::new(sig);
    let toks = doc.tokens();
    let open = toks.first().filter(|t| t.kind == Kind::Open)?;
    let kids: Vec<_> = toks.iter().filter(|t| t.depth == open.depth + 1 && t.kind != Kind::Comment).collect();
    // A curried `((f a) b)` is not one lambda list.
    if kids.first().is_none_or(|t| t.kind != Kind::Atom) {
        return None;
    }
    let mut a = Arity { required: 0, optional: 0, rest: false, keys: false };
    let mut optional = false;
    let mut skip_next = false;
    let mut iter = kids.iter().skip(1).peekable();
    while let Some(t) = iter.next() {
        if skip_next {
            skip_next = false;
            continue;
        }
        match t.kind {
            Kind::Open => {
                // `[b]` / `(b default)`: an optional (positional) parameter —
                // unless keywords began, when it is a keyword's.
                if !a.keys {
                    a.optional += 1;
                }
                // Skip to the group's own closer.
                let depth = t.depth;
                for u in iter.by_ref() {
                    if u.kind == Kind::Close && u.depth == depth {
                        break;
                    }
                }
            }
            Kind::Atom => {
                let text = doc.slice(t.start, t.end);
                match text.as_str() {
                    "…" | "..." => return None,
                    "." | "&rest" | "#!rest" => {
                        a.rest = true;
                        skip_next = true;
                    }
                    "&opt" | "#!optional" => optional = true,
                    "&named" | "&keys" | "#!key" => a.keys = true,
                    _ if text.starts_with(':') || text.starts_with("#:") => {
                        a.keys = true;
                        skip_next = true;
                    }
                    _ if a.keys => {}
                    _ if optional => a.optional += 1,
                    _ => a.required += 1,
                }
            }
            _ => {}
        }
    }
    Some(a)
}

/// Why CALL does not fit A, or None if it does. To a procedure without
/// keyword parameters a keyword is just a value — ray names things with
/// them, `(plate :main-plate …)` — so every argument counts. To one with
/// them, the arguments before the first keyword are its positional ones,
/// and only too many is sure: a keyword VALUE among them would make too
/// few look true.
fn misfit(name: &str, a: Arity, call: scm_text::scope::Call) -> Option<String> {
    let positional = if a.keys { call.positional } else { call.total };
    let plural = |n: usize| if n == 1 { "argument" } else { "arguments" };
    let max = a.required + a.optional;
    if positional < a.required && !(a.keys && call.keywords) {
        let wants = if a.rest || a.optional > 0 {
            format!("at least {}", a.required)
        } else {
            a.required.to_string()
        };
        return Some(format!("{name} takes {wants} {}, given {positional}", plural(a.required)));
    }
    if !a.rest && positional > max {
        let wants = if a.optional > 0 { format!("{} to {max}", a.required) } else { max.to_string() };
        return Some(format!("{name} takes {wants} {}, given {positional}", plural(max)));
    }
    None
}

/// What one image binds.
#[derive(Default)]
struct Facts {
    names: HashSet<String>,
    syntax: HashSet<String>,
    arities: HashMap<String, Arity>,
}

/// A buffer this big is not judged: it is data (a rock's point cloud, 2–5
/// MB), not code anyone is writing, and judging it would cost about 100 ms
/// an edit for nothing (Kevin, 2026-11-15: "skip it for 1 MB files and
/// over").
const TOO_BIG: usize = 1024 * 1024;

#[derive(Default)]
struct Warner {
    on: bool,
    /// Image key → what it binds.
    facts: HashMap<String, Facts>,
    /// Buffer path → the image it is judged against.
    buffers: HashMap<String, String>,
    /// Buffer path → its text as last told, so a change of facts re-judges
    /// it without waiting for an edit.
    texts: HashMap<String, String>,
    /// Buffer path → its warnings. A buffer missing here with a text is
    /// judged when next asked — what new facts do, so a send re-judges the
    /// buffer drawn, not every buffer open.
    found: HashMap<String, Vec<Diagnostic>>,
}

thread_local! {
    static WARNER: RefCell<Warner> = RefCell::new(Warner { on: true, ..Warner::default() });
}

impl Warner {
    fn judge(&mut self, path: &str) {
        let warnings = match (self.on, self.buffers.get(path).and_then(|k| self.facts.get(k)), self.texts.get(path)) {
            (true, Some(facts), Some(text)) if text.len() < TOO_BIG => judge(path, text, facts),
            _ => Vec::new(),
        };
        self.found.insert(path.to_string(), warnings);
    }
}

/// What the `.scm` files beside a file define — its folder and the folders
/// directly in it — with their call shapes. A scene's parts define what
/// each other uses (`fd`, `concrete`), and a part opened without its scene
/// loaded must not call those undefined; the index holds them only when the
/// scene is loaded (the modeler) or the checkout is (the standalone).
#[derive(Default)]
struct Neighbours {
    /// Each file's modification time, to know when to read them again.
    stamp: Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>,
    names: HashSet<String>,
    sigs: HashMap<String, String>,
}

thread_local! {
    static NEIGHBOURS: RefCell<HashMap<std::path::PathBuf, std::rc::Rc<Neighbours>>> = RefCell::new(HashMap::new());
}

fn neighbours(file: &std::path::Path) -> std::rc::Rc<Neighbours> {
    let Some(dir) = file.parent() else { return Default::default() };
    let scheme = |p: &std::path::Path| p.extension().is_some_and(|x| x == "scm");
    let mut paths = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            paths.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).filter(|q| scheme(q)));
        } else if scheme(&p) {
            // The file itself too (its saved text): one entry per FOLDER,
            // shared by every part in it. What the buffer defines now is
            // `here`, which wins.
            paths.push(p);
        }
    }
    paths.sort();
    // Not a home directory's worth: a folder of a scene's parts.
    paths.truncate(200);
    let stamp: Vec<_> = paths.iter().map(|p| (p.clone(), std::fs::metadata(p).and_then(|m| m.modified()).ok())).collect();
    let key = dir.to_path_buf();
    if let Some(n) = NEIGHBOURS.with(|c| c.borrow().get(&key).cloned())
        && n.stamp == stamp
    {
        return n;
    }
    let mut n = Neighbours { stamp, ..Default::default() };
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let doc = scm_text::Doc::new(&text);
        n.names.extend(scm_text::scope::toplevel_names(&doc));
        for d in doc.definitions() {
            if let Some(sig) = d.detail {
                n.sigs.entry(d.name).or_insert(sig);
            }
        }
    }
    let n = std::rc::Rc::new(n);
    NEIGHBOURS.with(|c| c.borrow_mut().insert(key, n.clone()));
    n
}

/// The warnings for TEXT, the buffer visiting PATH, judged against what
/// FACTS' image binds.
fn judge(path: &str, text: &str, facts: &Facts) -> Vec<Diagnostic> {
    use scm_text::scope;
    let doc = scm_text::Doc::new(text);
    let here = scope::toplevel_names(&doc);
    let near = neighbours(std::path::Path::new(path));
    // Macros this file defines are macros too.
    let local_syntax: HashSet<String> =
        doc.definitions().into_iter().filter(|d| d.form == "define-syntax").map(|d| d.name).collect();
    let is_macro = |h: &str| facts.syntax.contains(h) || local_syntax.contains(h);
    // Positions as the widget counts them: (line, byte).
    let lines: Vec<&str> = text.split('\n').collect();
    let at = |offset: usize| {
        let line = doc.position(offset).0 as usize;
        let l = lines.get(line).copied().unwrap_or("");
        (line, l.char_indices().nth(doc.column(offset) as usize).map_or(l.len(), |(b, _)| b))
    };
    let mut out = Vec::new();
    for r in scope::references(&doc) {
        if r.under.iter().any(|h| is_macro(h)) {
            continue;
        }
        let elsewhere = |n: &str| super::xref::defines(n) || near.names.contains(n);
        let known = facts.names.contains(&r.name) || facts.syntax.contains(&r.name) || here.contains(&r.name) || elsewhere(&r.name);
        if !known {
            out.push(Diagnostic {
                start: at(r.start),
                end: at(r.end),
                message: format!("{} is not defined — not in this image, this file, the files beside it or the indexed files", r.name),
                severity: Severity::Warning,
            });
            continue;
        }
        let Some(call) = r.call else { continue };
        // The file is the truth about what it defines; the image about the
        // rest. A file's own definition not yet sent is judged by its text.
        let arity = if here.contains(&r.name) || (!facts.names.contains(&r.name) && elsewhere(&r.name)) {
            doc.definitions()
                .into_iter()
                .find(|d| d.name == r.name)
                .and_then(|d| d.detail)
                .or_else(|| super::xref::signature(&r.name))
                .or_else(|| near.sigs.get(&r.name).cloned())
                .and_then(|s| arity_of(&s))
        } else {
            facts.arities.get(&r.name).copied()
        };
        if let Some(why) = arity.and_then(|a| misfit(&r.name, a, call)) {
            out.push(Diagnostic { start: at(r.start), end: at(r.end), message: why, severity: Severity::Warning });
        }
    }
    out
}

struct WarnWatcher;

static WATCHER: WarnWatcher = WarnWatcher;

impl crate::editorpane::Watcher for WarnWatcher {
    fn text_changed(&self, path: &str, lang: &str, lines: &[&str]) {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            if lang != "scheme" {
                w.texts.remove(path);
                w.found.remove(path);
                return;
            }
            w.texts.insert(path.to_string(), lines.join("\n"));
            w.judge(path);
        });
    }

    fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            if !w.found.contains_key(path) && w.texts.contains_key(path) {
                w.judge(path);
            }
            w.found.get(path).cloned().unwrap_or_default()
        })
    }

    fn closed(&self, path: &str) {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            w.texts.remove(path);
            w.found.remove(path);
            w.buffers.remove(path);
        });
    }
}

prim! {
    /// `(%warn-facts key names syntax signatures)` — what the image called
    /// KEY binds: its variables (NAMES, `apropos`), its syntax (SYNTAX,
    /// `%syntax-names`) and its procedures' call shapes (SIGNATURES, a flat
    /// list name sig name sig …). Every buffer judged against KEY is judged
    /// again.
    "%warn-facts", fn warn_facts(key: text, names: textv, syntax: textv, sigs: textv) {
        let arities = sigs
            .chunks(2)
            .filter_map(|p| match p {
                [n, s] => arity_of(s).map(|a| (n.clone(), a)),
                _ => None,
            })
            .collect();
        let facts = Facts { names: names.into_iter().collect(), syntax: syntax.into_iter().collect(), arities };
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            w.facts.insert(key.clone(), facts);
            let stale: Vec<String> = w.buffers.iter().filter(|(_, k)| **k == key).map(|(p, _)| p.clone()).collect();
            for p in stale {
                w.found.remove(&p);
            }
        });
        crate::editorpane::set_watcher(&WATCHER);
        crate::host::mark_dirty_ui();
        Out::Nil
    }
}

prim! {
    /// `(%warn-buffer path key)` — judge the buffer visiting PATH against the
    /// image KEY (`%warn-facts`); #f judges it against none, so it shows no
    /// warnings.
    "%warn-buffer", fn warn_buffer(path: text, key: opt_text) {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            match key {
                Some(k) => w.buffers.insert(path.clone(), k),
                None => w.buffers.remove(&path),
            };
            w.judge(&path);
        });
        crate::editorpane::set_watcher(&WATCHER);
        crate::host::mark_dirty_ui();
        Out::Nil
    }
}

prim! {
    /// `(%warn-facts? key)` — whether the image KEY has been described.
    "%warn-facts?", fn warn_has_facts(key: text) {
        Out::Num(if WARNER.with(|w| w.borrow().facts.contains_key(&key)) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%warn-enable on)` — draw the warnings (1) or not (0).
    "%warn-enable", fn warn_enable(on: num) {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            w.on = on != 0.0;
            w.found.clear();
        });
        crate::host::mark_dirty_ui();
        Out::Nil
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        warn_facts::install(interp)?,
        warn_buffer::install(interp)?,
        warn_has_facts::install(interp)?,
        warn_enable::install(interp)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_shape_reads_as_an_arity() {
        let a = |s| arity_of(s).unwrap();
        assert_eq!(a("(f a b)"), Arity { required: 2, optional: 0, rest: false, keys: false });
        assert_eq!(a("(f a [b] :at at . rest)"), Arity { required: 1, optional: 1, rest: true, keys: true });
        assert_eq!(a("(g a &opt b c)"), Arity { required: 1, optional: 2, rest: false, keys: false });
        assert_eq!(a("(h a (b 1))"), Arity { required: 1, optional: 1, rest: false, keys: false });
        assert_eq!(a("(v . xs)"), Arity { required: 0, optional: 0, rest: true, keys: false });
        assert_eq!(arity_of("(car …)"), None, "a primitive says nothing");
        assert_eq!(arity_of("((f a) b)"), None, "curried");
    }

    #[test]
    fn a_misfit_says_what_it_takes() {
        let one = Arity { required: 1, optional: 0, rest: false, keys: false };
        use scm_text::scope::Call;
        let plain = |n| Call { total: n, positional: n, keywords: false };
        assert_eq!(misfit("cube", one, plain(2)).as_deref(), Some("cube takes 1 argument, given 2"));
        assert_eq!(misfit("cube", one, plain(1)), None);
        let range = Arity { required: 1, optional: 1, rest: false, keys: false };
        assert_eq!(misfit("f", range, plain(3)).as_deref(), Some("f takes 1 to 2 arguments, given 3"));
        let rest = Arity { required: 2, optional: 0, rest: true, keys: false };
        assert_eq!(misfit("g", rest, plain(1)).as_deref(), Some("g takes at least 2 arguments, given 1"));
        assert_eq!(misfit("g", rest, plain(9)), None);
        // A keyword passed as a NAME counts when the procedure takes none.
        let named = Call { total: 4, positional: 0, keywords: true };
        let four = Arity { required: 4, optional: 0, rest: false, keys: false };
        assert_eq!(misfit("plate", four, named), None);
        // To one that takes keywords, only too many is sure.
        let keyed = Arity { required: 1, optional: 0, rest: false, keys: true };
        assert_eq!(misfit("add-sphere", keyed, Call { total: 2, positional: 0, keywords: true }), None);
        assert!(misfit("add-sphere", keyed, Call { total: 4, positional: 2, keywords: true }).is_some());
    }

    #[test]
    fn a_megabyte_buffer_is_not_judged() {
        WARNER.with(|w| {
            let mut w = w.borrow_mut();
            w.facts.insert("k".into(), Facts::default());
            w.buffers.insert("/big.scm".into(), "k".into());
            w.texts.insert("/big.scm".into(), format!("(undefined-thing)\n{}", " ".repeat(TOO_BIG)));
            w.judge("/big.scm");
            assert!(w.found["/big.scm"].is_empty(), "data, not code");
            w.texts.insert("/big.scm".into(), "(undefined-thing)\n".into());
            w.judge("/big.scm");
            assert_eq!(w.found["/big.scm"].len(), 1, "the same text, small, is judged");
        });
    }

    #[test]
    fn a_buffer_is_judged_against_its_image() {
        let facts = Facts {
            names: ["display", "+", "add-sphere", "cube"].iter().map(|s| s.to_string()).collect(),
            syntax: ["define", "let", "with-scene"].iter().map(|s| s.to_string()).collect(),
            arities: [("cube".to_string(), arity_of("(cube x)").unwrap()), ("add-sphere".to_string(), arity_of("(add-sphere name :radius r)").unwrap())]
                .into_iter()
                .collect(),
        };
        let text = "(define (twice x) (+ x x))\n(add-shpere \"s\" :radius 1)\n(cube 1 2)\n(twice 1 2)\n(with-scene s (anything at all))\n(add-sphere \"s\" :radius 1)\n";
        let got: Vec<(usize, String)> = judge("/nowhere/x.scm", text, &facts).into_iter().map(|d| (d.start.0, d.message)).collect();
        assert_eq!(
            got,
            vec![
                (1, "add-shpere is not defined — not in this image, this file, the files beside it or the indexed files".to_string()),
                (2, "cube takes 1 argument, given 2".to_string()),
                (3, "twice takes 1 argument, given 2".to_string()),
            ]
        );
    }
}
