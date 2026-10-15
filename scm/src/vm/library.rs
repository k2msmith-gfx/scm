//! R7RS libraries: `define-library`, `import`, and where libraries come from.
//!
//! **A library is an environment frame.** Its body runs with that frame as
//! the environment, so its `define`s land in the frame and never touch the
//! global namespace; a `define-syntax` inside it binds the macro as a value
//! in the frame, which is what `let-syntax` already does, so library macros
//! are scoped the same way. Importing copies bindings: into the importing
//! library's frame, or into the globals for a program -- the REPL, a file,
//! a bundle -- so user code stays on the fast global path. `(scheme base)`
//! and the other standard names are the prelude, so importing them is a
//! no-op.
//!
//! The one simplification: a binding is copied at import, so a library that
//! `set!`s its own exported variable *after* it was imported does not reach
//! the importer. R7RS makes `set!` on an imported binding an error and no
//! library in the wild does the other thing; it can become a shared cell
//! later without changing a program.
//!
//! A library named `(srfi 1)` is looked for as `srfi/1.sld`, then `.scm`,
//! in the current directory and on `SCM_LIBRARY_PATH`, and failing those in
//! the set embedded in the binary -- which is how a `scm build` bundle has
//! them too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{Step, VResult, Vm, VmError};
use crate::heap::Obj;
use crate::syntax::SyntaxRules;
use crate::value::{ObjRef, SymId, Value};

/// A loaded library.
pub struct Library {
    /// Its top-level frame. A collector root for as long as the library is
    /// registered, since importers may hold closures over it.
    pub frame: ObjRef,
    /// `(external, internal)` names, in export order.
    pub exports: Vec<(SymId, SymId)>,
}

/// Libraries this crate ships, embedded: `(name, source)`. Embedded rather
/// than installed, so `scm` needs no library directory beside it and a
/// `scm build` bundle carries them too. A file of the same name found on
/// the library path wins, which is how one is overridden or developed.
pub const EMBEDDED: &[(&str, &str)] = &[
    ("(scm test)", include_str!("../../lib/scm/test.sld")),
    ("(scm hash-table)", include_str!("../../lib/scm/hash-table.sld")),
    ("(scm numvector)", include_str!("../../lib/scm/numvector.sld")),
    ("(scm linalg)", include_str!("../../lib/scm/linalg.sld")),
    ("(scm noise)", include_str!("../../lib/scm/noise.sld")),
    ("(srfi 160 base)", include_str!("../../lib/srfi/160/base.sld")),
    ("(srfi 160 s8)", include_str!("../../lib/srfi/160/s8.sld")),
    ("(srfi 160 u8)", include_str!("../../lib/srfi/160/u8.sld")),
    ("(srfi 160 s16)", include_str!("../../lib/srfi/160/s16.sld")),
    ("(srfi 160 u16)", include_str!("../../lib/srfi/160/u16.sld")),
    ("(srfi 160 s32)", include_str!("../../lib/srfi/160/s32.sld")),
    ("(srfi 160 u32)", include_str!("../../lib/srfi/160/u32.sld")),
    ("(srfi 160 s64)", include_str!("../../lib/srfi/160/s64.sld")),
    ("(srfi 160 u64)", include_str!("../../lib/srfi/160/u64.sld")),
    ("(srfi 160 f32)", include_str!("../../lib/srfi/160/f32.sld")),
    ("(srfi 160 f64)", include_str!("../../lib/srfi/160/f64.sld")),
    ("(srfi 1)", include_str!("../../lib/srfi/1.sld")),
    ("(srfi 2)", include_str!("../../lib/srfi/2.sld")),
    ("(srfi 4)", include_str!("../../lib/srfi/4.sld")),
    ("(srfi 8)", include_str!("../../lib/srfi/8.sld")),
    ("(srfi 63)", include_str!("../../lib/srfi/63.sld")),
    ("(srfi 27)", include_str!("../../lib/srfi/27.sld")),
    ("(srfi 42)", include_str!("../../lib/srfi/42.sld")),
    ("(srfi 158)", include_str!("../../lib/srfi/158.sld")),
    ("(srfi 194)", include_str!("../../lib/srfi/194.sld")),
    ("(srfi 69)", include_str!("../../lib/srfi/69.sld")),
    ("(srfi 125)", include_str!("../../lib/srfi/125.sld")),
    ("(srfi 128)", include_str!("../../lib/srfi/128.sld")),
    ("(srfi 151)", include_str!("../../lib/srfi/151.sld")),
    ("(srfi 26)", include_str!("../../lib/srfi/26.sld")),
    ("(srfi 111)", include_str!("../../lib/srfi/111.sld")),
    ("(srfi 132)", include_str!("../../lib/srfi/132.sld")),
    ("(srfi 133)", include_str!("../../lib/srfi/133.sld")),
    ("(srfi 14)", include_str!("../../lib/srfi/14.sld")),
    ("(srfi 13)", include_str!("../../lib/srfi/13.sld")),
    ("(srfi 130)", include_str!("../../lib/srfi/130.sld")),
];

/// Every embedded library's name, for `(scm libraries)` and the gate.
pub fn embedded_names() -> Vec<&'static str> {
    EMBEDDED.iter().map(|(n, _)| *n).collect()
}

/// A binding as it crosses a library boundary.
#[derive(Clone)]
enum Binding {
    Value(Value),
    Macro(Rc<SyntaxRules>),
}

impl Vm {
    /// The canonical spelling of a library name: `(srfi 1)` -> `"(srfi 1)"`.
    fn library_name(&mut self, form: Value) -> VResult<String> {
        let parts = self.list_to_vec(form)?;
        if parts.is_empty() {
            return Err(self.err("a library name is a non-empty list"));
        }
        let mut out = String::from("(");
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            match p {
                Value::Symbol(s) => out.push_str(self.engine.interner.sym_name(self.engine.resolve(*s))),
                Value::Fixnum(n) if *n >= 0 => out.push_str(&n.to_string()),
                other => {
                    return Err(self.err(format!(
                        "a library name part must be a symbol or a non-negative integer, not {}",
                        crate::writer::write_str(&self.engine, *other)
                    )));
                }
            }
        }
        out.push(')');
        Ok(out)
    }

    /// The standard libraries are the prelude: importing one is a no-op.
    fn is_builtin_library(name: &str) -> bool {
        name.starts_with("(scheme ") || name == "(scm)"
    }

    /// `(define-library NAME decl …)`, at top level.
    pub(super) fn define_library(&mut self, args: &[Value], source_dir: Option<PathBuf>) -> VResult<Step> {
        let Some((name_form, decls)) = args.split_first() else {
            return Err(self.err("define-library: expected a name and declarations"));
        };
        let name = self.library_name(*name_form)?;
        if self.loading.contains(&name) {
            return Err(self.err(format!("library {name} imports itself, through {}", self.loading.join(" -> "))));
        }
        let frame = self.engine.env(None, Vec::new());
        // Registered before the body runs, so a collection during it sees
        // the frame, and so a cycle is reported rather than looped.
        self.libraries.insert(name.clone(), Library { frame, exports: Vec::new() });
        self.loading.push(name.clone());
        let saved_dir = std::mem::replace(&mut self.source_dir, source_dir);
        let result = self.library_body(frame, decls);
        self.source_dir = saved_dir;
        self.loading.pop();
        match result {
            Ok(exports) => {
                if let Some(lib) = self.libraries.get_mut(&name) {
                    lib.exports = exports;
                }
                Ok(Step::Return(Value::Unspecified))
            }
            Err(e) => {
                self.libraries.remove(&name);
                Err(e)
            }
        }
    }

    /// Process the declarations into `frame`; returns the export table.
    fn library_body(&mut self, frame: ObjRef, decls: &[Value]) -> VResult<Vec<(SymId, SymId)>> {
        let mut export_specs: Vec<Value> = Vec::new();
        let mut work: Vec<Value> = decls.to_vec();
        let mut i = 0;
        while i < work.len() {
            let decl = work[i];
            i += 1;
            let parts = self.list_to_vec(decl)?;
            let Some(Value::Symbol(head)) = parts.first() else {
                return Err(self.err("define-library: each declaration is a list headed by a keyword"));
            };
            let head = self.engine.interner.sym_name(self.engine.resolve(*head)).to_string();
            match head.as_str() {
                "export" => export_specs.extend_from_slice(&parts[1..]),
                "import" => {
                    for set in &parts[1..] {
                        let bindings = self.import_set(*set)?;
                        for (name, b) in bindings {
                            self.bind_in_frame(frame, name, b);
                        }
                    }
                }
                "begin" => {
                    for form in &parts[1..] {
                        let base = self.stack.len();
                        self.eval_inner(Step::Eval(*form, Some(frame)), base)?;
                    }
                }
                "include" | "include-ci" => {
                    for f in &parts[1..] {
                        let forms = self.read_include(*f)?;
                        for form in forms {
                            let base = self.stack.len();
                            self.eval_inner(Step::Eval(form, Some(frame)), base)?;
                        }
                    }
                }
                "include-library-declarations" => {
                    for f in &parts[1..] {
                        let forms = self.read_include(*f)?;
                        // Splice them in where we are.
                        let rest: Vec<Value> = work.drain(i..).collect();
                        work.extend(forms);
                        work.extend(rest);
                    }
                }
                "cond-expand" => {
                    let chosen = self.cond_expand(&parts[1..])?;
                    let rest: Vec<Value> = work.drain(i..).collect();
                    work.extend(chosen);
                    work.extend(rest);
                }
                other => return Err(self.err(format!("define-library: unknown declaration `{other}`"))),
            }
        }
        let mut exports = Vec::new();
        for spec in export_specs {
            let (external, internal) = match spec {
                Value::Symbol(s) => (s, s),
                other => {
                    let parts = self.list_to_vec(other)?;
                    match parts.as_slice() {
                        [Value::Symbol(r), Value::Symbol(from), Value::Symbol(to)]
                            if self.engine.interner.sym_name(self.engine.resolve(*r)) == "rename" =>
                        {
                            (*to, *from)
                        }
                        _ => return Err(self.err("export: expected a name or (rename internal external)")),
                    }
                }
            };
            if self.frame_binding(frame, internal).is_none() {
                return Err(self.err(format!(
                    "export: `{}` is not defined in the library",
                    self.engine.interner.sym_name(self.engine.resolve(internal))
                )));
            }
            exports.push((self.engine.resolve(external), self.engine.resolve(internal)));
        }
        Ok(exports)
    }

    /// Read every form in an included file, relative to the library's own.
    fn read_include(&mut self, f: Value) -> VResult<Vec<Value>> {
        let Value::Obj(r) = f else {
            return Err(self.err("include: expected a file name"));
        };
        let Obj::Str(chars) = self.engine.obj(r) else {
            return Err(self.err("include: expected a file name"));
        };
        let name: String = chars.iter().collect();
        let path = match &self.source_dir {
            Some(dir) if !Path::new(&name).is_absolute() => dir.join(&name),
            _ => PathBuf::from(&name),
        };
        let text = std::fs::read_to_string(&path).map_err(|e| self.err(format!("include: {}: {e}", path.display())))?;
        crate::reader::Reader::read_all_named(&text, &path.display().to_string(), &mut self.engine)
            .map_err(|e| VmError::Error(format!("include: {}: {e}", path.display())))
    }

    /// `cond-expand` clauses: the body of the first whose requirement holds.
    pub(super) fn cond_expand(&mut self, clauses: &[Value]) -> VResult<Vec<Value>> {
        for clause in clauses {
            let parts = self.list_to_vec(*clause)?;
            let Some((req, body)) = parts.split_first() else {
                return Err(self.err("cond-expand: each clause is (requirement body …)"));
            };
            if self.feature_holds(*req)? {
                return Ok(body.to_vec());
            }
        }
        Ok(Vec::new())
    }

    fn feature_holds(&mut self, req: Value) -> VResult<bool> {
        match req {
            Value::Symbol(s) => {
                let name = self.engine.interner.sym_name(self.engine.resolve(s)).to_string();
                if name == "else" {
                    return Ok(true);
                }
                let features = self.eval_str("(features)")?;
                let list = self.list_to_vec(features)?;
                Ok(list.iter().any(|f| matches!(f, Value::Symbol(x) if self.engine.interner.sym_name(*x) == name)))
            }
            Value::Obj(_) => {
                let parts = self.list_to_vec(req)?;
                let Some(Value::Symbol(h)) = parts.first() else {
                    return Err(self.err("cond-expand: bad requirement"));
                };
                let h = self.engine.interner.sym_name(self.engine.resolve(*h)).to_string();
                match h.as_str() {
                    "and" => {
                        for p in &parts[1..] {
                            if !self.feature_holds(*p)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    "or" => {
                        for p in &parts[1..] {
                            if self.feature_holds(*p)? {
                                return Ok(true);
                            }
                        }
                        Ok(false)
                    }
                    "not" => Ok(!self.feature_holds(parts.get(1).copied().unwrap_or(Value::Bool(false)))?),
                    "library" => {
                        let name = self.library_name(parts.get(1).copied().unwrap_or(Value::Null))?;
                        Ok(Self::is_builtin_library(&name) || self.libraries.contains_key(&name) || self.locate_library(&name).is_some())
                    }
                    _ => Err(self.err(format!("cond-expand: unknown requirement `{h}`"))),
                }
            }
            _ => Err(self.err("cond-expand: bad requirement")),
        }
    }

    /// The bindings an import set denotes: `(external name, binding)`.
    fn import_set(&mut self, set: Value) -> VResult<Vec<(SymId, Binding)>> {
        let parts = self.list_to_vec(set)?;
        let Some(Value::Symbol(h)) = parts.first() else {
            return Err(self.err("import: expected a library name or import set"));
        };
        let head = self.engine.interner.sym_name(self.engine.resolve(*h)).to_string();
        let modifier = matches!(head.as_str(), "only" | "except" | "prefix" | "rename") && parts.len() >= 2;
        if !modifier {
            let name = self.library_name(set)?;
            return self.library_bindings(&name);
        }
        let inner = self.import_set(parts[1])?;
        let rest = &parts[2..];
        match head.as_str() {
            "only" => {
                let mut out = Vec::new();
                for id in rest {
                    let Value::Symbol(id) = id else { return Err(self.err("only: expected identifiers")) };
                    let id = self.engine.resolve(*id);
                    match inner.iter().find(|(n, _)| *n == id) {
                        Some(b) => out.push(b.clone()),
                        None => return Err(self.err(format!("only: `{}` is not exported", self.engine.interner.sym_name(id)))),
                    }
                }
                Ok(out)
            }
            "except" => {
                let mut drop = Vec::new();
                for id in rest {
                    let Value::Symbol(id) = id else { return Err(self.err("except: expected identifiers")) };
                    let id = self.engine.resolve(*id);
                    if !inner.iter().any(|(n, _)| *n == id) {
                        return Err(self.err(format!("except: `{}` is not exported", self.engine.interner.sym_name(id))));
                    }
                    drop.push(id);
                }
                Ok(inner.into_iter().filter(|(n, _)| !drop.contains(n)).collect())
            }
            "prefix" => {
                let Some(Value::Symbol(p)) = rest.first() else { return Err(self.err("prefix: expected an identifier")) };
                let p = self.engine.interner.sym_name(self.engine.resolve(*p)).to_string();
                Ok(inner
                    .into_iter()
                    .map(|(n, b)| {
                        let full = format!("{p}{}", self.engine.interner.sym_name(n));
                        (self.engine.interner.symbol(&full), b)
                    })
                    .collect())
            }
            _ => {
                // rename
                let mut out = inner;
                for pair in rest {
                    let pr = self.list_to_vec(*pair)?;
                    let [Value::Symbol(from), Value::Symbol(to)] = pr.as_slice() else {
                        return Err(self.err("rename: expected (from to) pairs"));
                    };
                    let (from, to) = (self.engine.resolve(*from), self.engine.resolve(*to));
                    match out.iter_mut().find(|(n, _)| *n == from) {
                        Some(entry) => entry.0 = to,
                        None => return Err(self.err(format!("rename: `{}` is not exported", self.engine.interner.sym_name(from)))),
                    }
                }
                Ok(out)
            }
        }
    }

    /// Every export of a library, loading it first if needed.
    fn library_bindings(&mut self, name: &str) -> VResult<Vec<(SymId, Binding)>> {
        if Self::is_builtin_library(name) {
            return Ok(Vec::new());
        }
        if self.loading.iter().any(|l| l == name) {
            return Err(self.err(format!("library {name} imports itself, through {}", self.loading.join(" -> "))));
        }
        if !self.libraries.contains_key(name) {
            self.load_library(name)?;
        }
        let (frame, exports) = {
            let lib = self.libraries.get(name).expect("loaded");
            (lib.frame, lib.exports.clone())
        };
        let mut out = Vec::with_capacity(exports.len());
        for (external, internal) in exports {
            match self.frame_binding(frame, internal) {
                Some(b) => out.push((external, b)),
                None => return Err(self.err(format!("{name}: export `{}` has no binding", self.engine.interner.sym_name(internal)))),
            }
        }
        Ok(out)
    }

    /// Find the library's source: on disk, or embedded.
    fn locate_library(&self, name: &str) -> Option<(String, Option<PathBuf>)> {
        let rel: String = name.trim_matches(|c| c == '(' || c == ')').split(' ').collect::<Vec<_>>().join("/");
        let mut dirs: Vec<PathBuf> = vec![PathBuf::from(".")];
        if let Some(dir) = &self.source_dir {
            dirs.push(dir.clone());
        }
        if let Ok(p) = std::env::var("SCM_LIBRARY_PATH") {
            dirs.extend(p.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
        }
        dirs.extend(self.library_path.iter().cloned());
        for dir in dirs {
            for ext in ["sld", "scm"] {
                let path = dir.join(format!("{rel}.{ext}"));
                if let Ok(text) = std::fs::read_to_string(&path) {
                    return Some((text, path.parent().map(Path::to_path_buf)));
                }
            }
        }
        EMBEDDED.iter().find(|(n, _)| *n == name).map(|(_, src)| (src.to_string(), None))
    }

    fn load_library(&mut self, name: &str) -> VResult<()> {
        let Some((text, dir)) = self.locate_library(name) else {
            return Err(self.err(format!("library not found: {name} (looked for {}.sld on . and SCM_LIBRARY_PATH)", name.trim_matches(|c| c == '(' || c == ')').replace(' ', "/"))));
        };
        let forms = crate::reader::Reader::read_all_named(&text, name, &mut self.engine)
            .map_err(|e| VmError::Error(format!("{name}: read error: {e}")))?;
        let handles: Vec<_> = forms.iter().map(|&f| self.engine.root(f)).collect();
        let mut result = Ok(());
        for form in &forms {
            let parts = self.list_to_vec(*form)?;
            let is_def = matches!(parts.first(), Some(Value::Symbol(s)) if self.engine.interner.sym_name(self.engine.resolve(*s)) == "define-library");
            if !is_def {
                result = Err(self.err(format!("{name}: a library file holds define-library forms only")));
                break;
            }
            let base = self.stack.len();
            if let Err(e) = self.define_library(&parts[1..], dir.clone()).and_then(|step| self.eval_inner(step, base)) {
                result = Err(e);
                break;
            }
        }
        for h in handles {
            self.engine.unroot(h);
        }
        result?;
        if !self.libraries.contains_key(name) {
            return Err(self.err(format!("{name}: the file did not define that library")));
        }
        Ok(())
    }

    /// A library's binding for `name`: its own, or -- since re-exporting
    /// an imported binding is legal R7RS and a standard import lands in
    /// the globals -- a global of that name. `(export make-list)` beside
    /// `(import (scheme base))` is the case that needs the fallback.
    fn frame_binding(&self, frame: ObjRef, name: SymId) -> Option<Binding> {
        let resolved = self.engine.resolve(name);
        if let Obj::Env { slots, .. } = self.engine.obj(frame)
            && let Some((_, v)) = slots
                .iter()
                .rev()
                .find(|(s, _)| *s == name || self.engine.resolve(*s) == resolved)
        {
            if let Value::Obj(r) = v
                && let Obj::Macro(m) = self.engine.obj(*r)
            {
                return Some(Binding::Macro(Rc::clone(m)));
            }
            return Some(Binding::Value(*v));
        }
        if let Some(m) = self.macros.get(&resolved) {
            return Some(Binding::Macro(Rc::clone(m)));
        }
        self.global(resolved).map(Binding::Value)
    }

    fn bind_in_frame(&mut self, frame: ObjRef, name: SymId, b: Binding) {
        let v = match b {
            Binding::Value(v) => v,
            Binding::Macro(m) => self.engine.alloc(Obj::Macro(m)),
        };
        if let Obj::Env { slots, .. } = self.engine.obj_mut(frame) {
            match slots.iter_mut().find(|(s, _)| *s == name) {
                Some(slot) => slot.1 = v,
                None => slots.push((name, v)),
            }
        }
    }

    /// `(import set …)` at top level: into the globals.
    pub(super) fn import_toplevel(&mut self, sets: &[Value]) -> VResult<Step> {
        for set in sets {
            for (name, b) in self.import_set(*set)? {
                match b {
                    Binding::Value(v) => self.set_global(name, v),
                    Binding::Macro(m) => {
                        self.macros.insert(name, m);
                        self.macro_gen += 1;
                    }
                }
            }
        }
        Ok(Step::Return(Value::Unspecified))
    }

    /// Import a library by name from the host: `vm.import("(srfi 1)")`.
    pub fn import(&mut self, name: &str) -> VResult<()> {
        let form = crate::reader::Reader::new(&format!("(import {name})")).read(&mut self.engine)
            .map_err(|e| VmError::Error(format!("{e}")))?
            .ok_or_else(|| VmError::Error("import: nothing to import".into()))?;
        let h = self.engine.root(form);
        let r = self.eval(form);
        self.engine.unroot(h);
        r.map(|_| ())
    }

    pub fn add_library_path(&mut self, dir: impl Into<PathBuf>) {
        self.library_path.push(dir.into());
    }

    /// The frames of every loaded library: collector roots.
    pub(super) fn library_roots(&self, out: &mut Vec<Value>) {
        out.extend(self.libraries.values().map(|l| Value::Obj(l.frame)));
    }

    pub fn loaded_libraries(&self) -> Vec<String> {
        let mut v: Vec<String> = self.libraries.keys().cloned().collect();
        v.sort();
        v
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<(), ()>) {}
