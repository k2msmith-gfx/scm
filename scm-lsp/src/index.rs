//! Every definition in the workspace, so a jump can leave the file.
//!
//! Go-to-definition without this answers only for the buffer you are in,
//! which for a project of any size is the case that matters least: the
//! whole point of the jump is to go somewhere you are not looking.
//!
//! **Positions are resolved at index time, not kept as text.** A name maps
//! straight to a line and a UTF-16 column, so the index holds no file
//! contents — otherwise it would be a second copy of the project in memory
//! that goes stale the moment anything is edited.
//!
//! This is the *textual* half of the answer, and it is honest about being
//! that: it finds what a file says, so a name defined at the REPL has no
//! entry, and neither has a prelude name. `sb-introspect`'s
//! `find-definition-sources-by-name` is the shape of the better answer --
//! the running session saying where a binding came from -- and it needs a
//! location recorded at definition time, which scm does not keep today.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::text::Doc;

/// Where a name is defined, already in the editor's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub uri: String,
    pub line: u32,
    pub character: u32,
    pub end_character: u32,
    /// Which `define…` bound it, for a workspace-symbol listing.
    pub form: String,
    pub detail: Option<String>,
}

#[derive(Default)]
pub struct Index {
    /// One entry per file, so a file can be re-scanned without a sweep.
    by_file: HashMap<String, Vec<(String, Site)>>,
}

/// Directories never worth walking. `target` alone is most of the win in a
/// Rust workspace -- it can hold thousands of `.scm` fixtures under a build
/// directory, and none of them is a definition anyone wants to jump to.
const SKIP: [&str; 7] = ["target", ".git", "node_modules", ".direnv", "result", ".cache", "dist"];
/// A guard against walking something enormous by accident.
const MAX_FILES: usize = 4000;
/// A file bigger than this is not hand-written Scheme; it is generated or
/// it is data.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

impl Index {
    /// Walk a root once, scanning every `.scm` and `.sld`.
    pub fn scan_root(&mut self, root: &Path) -> usize {
        let mut found = 0usize;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    // A leading dot covers .git, .direnv and anything else
                    // a tool keeps to itself.
                    if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
                        continue;
                    }
                    stack.push(path);
                } else if kind.is_file() && is_scheme(&path) && found < MAX_FILES {
                    if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
                        continue;
                    }
                    if self.scan_file(&path) {
                        found += 1;
                    }
                }
            }
        }
        found
    }

    /// Re-scan one file from disk. Returns whether it could be read.
    pub fn scan_file(&mut self, path: &Path) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        self.update(&uri_of(path), &text);
        true
    }

    /// Replace a file's entries from text that may not be on disk yet --
    /// an open buffer is the truth about itself, and a jump to a name just
    /// typed should land where it is now rather than where it was saved.
    pub fn update(&mut self, uri: &str, text: &str) {
        let doc = Doc::new(text);
        let sites: Vec<(String, Site)> = doc
            .definitions()
            .into_iter()
            .map(|d| {
                let (line, character) = doc.position(d.name_at);
                let (_, end_character) = doc.position(d.name_at + d.name.chars().count());
                (
                    d.name,
                    Site {
                        uri: uri.to_string(),
                        line,
                        character,
                        end_character,
                        form: d.form,
                        detail: d.detail,
                    },
                )
            })
            .collect();
        self.by_file.insert(uri.to_string(), sites);
    }

    pub fn forget(&mut self, uri: &str) {
        self.by_file.remove(uri);
    }

    /// Every place a name is defined. More than one is normal and not an
    /// error -- a name can be defined per library, or shadowed in a test --
    /// so all of them are returned and the editor offers the choice.
    pub fn sites(&self, name: &str) -> Vec<Site> {
        let mut out: Vec<Site> = self
            .by_file
            .values()
            .flatten()
            .filter(|(n, _)| n == name)
            .map(|(_, s)| s.clone())
            .collect();
        // Stable, so the same jump goes to the same place twice.
        out.sort_by(|a, b| (&a.uri, a.line).cmp(&(&b.uri, b.line)));
        out
    }

    /// For `workspace/symbol`: every name containing `query`, caselessly.
    /// An empty query means everything, which is what a picker opens with.
    pub fn matching(&self, query: &str, limit: usize) -> Vec<(String, Site)> {
        let needle = query.to_lowercase();
        let mut out: Vec<(String, Site)> = self
            .by_file
            .values()
            .flatten()
            .filter(|(n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
            .map(|(n, s)| (n.clone(), s.clone()))
            .collect();
        out.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }

    pub fn files(&self) -> usize {
        self.by_file.len()
    }

    pub fn names(&self) -> usize {
        self.by_file.values().map(Vec::len).sum()
    }
}

fn is_scheme(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("scm") | Some("sld")
    )
}

/// A `file:` URI. Hand-rolled rather than taking a URL crate for one job,
/// and percent-encoding only what actually breaks a URI -- a space in a
/// path is the case that turns up.
pub fn uri_of(path: &Path) -> String {
    let mut out = String::from("file://");
    for c in path.to_string_lossy().chars() {
        match c {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// The inverse, for the paths a client sends back.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut out = String::with_capacity(rest.len());
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            match u8::from_str_radix(&hex, 16) {
                Ok(b) => out.push(b as char),
                Err(_) => {
                    out.push('%');
                    out.push_str(&hex);
                }
            }
        } else {
            out.push(c);
        }
    }
    Some(PathBuf::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_round_trips_through_a_path_with_a_space() {
        let p = PathBuf::from("/tmp/a dir/b.scm");
        assert_eq!(uri_of(&p), "file:///tmp/a%20dir/b.scm");
        assert_eq!(path_of(&uri_of(&p)).unwrap(), p);
    }

    #[test]
    fn a_name_defined_twice_yields_both_sites_in_a_stable_order() {
        let mut ix = Index::default();
        ix.update("file:///b.scm", "(define (shared) 2)\n");
        ix.update("file:///a.scm", "\n(define (shared) 1)\n");
        let sites = ix.sites("shared");
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].uri, "file:///a.scm");
        assert_eq!(sites[0].line, 1);
        assert_eq!(sites[1].uri, "file:///b.scm");
    }

    #[test]
    fn re_scanning_a_file_replaces_its_entries_rather_than_adding() {
        let mut ix = Index::default();
        ix.update("file:///a.scm", "(define (was-here) 1)\n");
        assert_eq!(ix.sites("was-here").len(), 1);
        ix.update("file:///a.scm", "(define (renamed) 1)\n");
        assert!(ix.sites("was-here").is_empty(), "a stale entry survived");
        assert_eq!(ix.sites("renamed").len(), 1);
        assert_eq!(ix.names(), 1);
    }

    #[test]
    fn record_members_are_indexed_so_a_jump_can_find_them() {
        let mut ix = Index::default();
        ix.update("file:///r.scm", "(define-record-type point (make-point x y) point? (x point-x))\n");
        for name in ["point", "make-point", "point?", "point-x"] {
            assert_eq!(ix.sites(name).len(), 1, "{name} is not indexed");
        }
    }

    #[test]
    fn a_closed_file_is_forgotten() {
        let mut ix = Index::default();
        ix.update("file:///a.scm", "(define (x) 1)\n");
        ix.forget("file:///a.scm");
        assert!(ix.sites("x").is_empty());
        assert_eq!(ix.files(), 0);
    }
}
