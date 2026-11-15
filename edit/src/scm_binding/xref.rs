//! Where a name is defined — the editor pane's cross-file index
//! (docs/modeler-scene-buffers-plan.md, B3).
//!
//! `scm-text` is `scm-lsp`'s token scan and definition index, split into a
//! library so this pane could link it rather than talk to a language
//! server: the pane is INSIDE the session already, and LSP's whole job is
//! reaching a session in another process. The scan survives a half-typed
//! buffer, which is why an open buffer's unsaved text can be indexed as it
//! stands (`%xref-update`).
//!
//! The index holds one set of files at a time. Which set is Scheme policy —
//! the loaded scene's files and the preludes (`xref-reindex`, unify.scm) —
//! because in a tree of two hundred example scenes a name like `slab` is
//! defined many times, and the right one is the one this scene includes.
//! Where the preludes ARE is the host's to say: the modeler's
//! `%xref-prelude-dirs` stayed in the modeler (`script/xref_dirs.rs`).

use std::cell::RefCell;
use std::path::Path;

use scm_text::Index;
use scm_text::index::{path_of, uri_of};

use super::Out;

thread_local! {
    /// The main thread's index: the prims run there, as every prim does.
    static INDEX: RefCell<Index> = RefCell::new(Index::default());
}

/// Whether any indexed file defines NAME (E8's warnings).
pub(super) fn defines(name: &str) -> bool {
    INDEX.with(|ix| ix.borrow().defines(name))
}

/// The call shape an indexed file gives NAME, `(f a b)`, if one does.
pub(super) fn signature(name: &str) -> Option<String> {
    INDEX.with(|ix| ix.borrow().signature(name))
}

prim! {
    /// `(%xref-index paths)` — replace the index with PATHS: each a `.scm`
    /// file, or a directory walked for them. Answers how many names it
    /// holds. An unreadable path is skipped; the rest still index.
    "%xref-index", fn xref_index(paths: textv) {
        INDEX.with(|ix| {
            let mut ix = ix.borrow_mut();
            *ix = Index::default();
            for p in &paths {
                let p = Path::new(p);
                if p.is_dir() {
                    ix.scan_root(p);
                } else {
                    ix.scan_file(p);
                }
            }
            Out::Num(ix.names() as f64)
        })
    }
}

prim! {
    /// `(%xref-update path text)` — re-index PATH from TEXT, which need not
    /// be on disk: an open buffer is the truth about itself.
    "%xref-update", fn xref_update(path: text, src: text) {
        INDEX.with(|ix| ix.borrow_mut().update(&uri_of(Path::new(&path)), &src));
        Out::Nil
    }
}

/// A site as Scheme gets it: `(path line column form)`, 0-based, the column
/// in characters.
fn site_out(site: &scm_text::Site) -> Out {
    let path = path_of(&site.uri).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| site.uri.clone());
    Out::Many(vec![
        Out::Text(path),
        Out::Num(f64::from(site.line)),
        Out::Num(f64::from(site.column)),
        Out::Text(site.form.clone()),
    ])
}

prim! {
    /// `(%xref-sites name)` — every place NAME is defined, as
    /// `((path line column form) …)`, in a stable order.
    "%xref-sites", fn xref_sites(name: text) {
        INDEX.with(|ix| Out::Arr(ix.borrow().sites(&name).iter().map(site_out).collect()))
    }
}

prim! {
    /// `(%xref-references name)` — every place NAME is USED, as
    /// `((path line column "use") …)`, by file and position (E8): a use is
    /// a free occurrence, so a local binding of the same name and quoted
    /// data are not among them.
    "%xref-references", fn xref_references(name: text) {
        INDEX.with(|ix| Out::Arr(ix.borrow().references(&name).iter().map(site_out).collect()))
    }
}

prim! {
    /// `(%xref-matching text limit)` — names containing TEXT, caselessly,
    /// shortest first, at most LIMIT: `((name path line column form) …)`.
    "%xref-matching", fn xref_matching(query: text, limit: num) {
        INDEX.with(|ix| {
            Out::Arr(
                ix.borrow()
                    .matching(&query, limit.max(0.0) as usize)
                    .iter()
                    .map(|(name, site)| match site_out(site) {
                        Out::Many(mut row) => {
                            row.insert(0, Out::Text(name.clone()));
                            Out::Many(row)
                        }
                        other => other,
                    })
                    .collect(),
            )
        })
    }
}

prim! {
    /// `(%text-definitions text)` — what TEXT defines, in order, for the
    /// document-symbols popup (E3f): `((name form detail line column) …)`,
    /// line from 0 and column in characters at the name, detail the call
    /// shape `(f a b)` for a procedure or #f. The same scan as the index,
    /// so it answers for a half-typed buffer.
    "%text-definitions", fn text_definitions(text: text) {
        let doc = scm_text::Doc::new(&text);
        Out::rows(doc.definitions().into_iter().map(|d| {
            Out::Many(vec![
                Out::Text(d.name),
                Out::Text(d.form),
                d.detail.map_or(Out::Nil, Out::Text),
                Out::Num(doc.position(d.name_at).0 as f64),
                Out::Num(doc.column(d.name_at) as f64),
            ])
        }))
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        xref_index::install(interp)?,
        xref_update::install(interp)?,
        xref_sites::install(interp)?,
        xref_references::install(interp)?,
        xref_matching::install(interp)?,
        text_definitions::install(interp)?,
    ])
}
