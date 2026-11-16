//! The scene-editor pane (docs/modeler-scene-editor-plan.md): a multi-line
//! text-EDITOR surface — where textpane.rs is a transcript (append + one
//! input line), this pane holds a whole file. The editing state machine
//! (cursor, selection, click hit-testing, scroll) is `cosmic_text::Editor`;
//! rendering stays ours — cosmic-text hands back plain rects through a
//! callback and rasterizes glyphs via the same swash the rest of the UI
//! uses, composited with [`crate::text::blend`] into the CPU framebuffer.
//!
//! The Emacs split, applied again: the kernel owns the buffer, the raw edit
//! actions (`%editor-*` prims), and the undo/redo stacks; WHICH file loads,
//! the key bindings, when it saves, and what gets sent to the interpreter
//! are Scheme policy (unify.scm's scene-editor section).
//!
//! Undo granularity: consecutive plain inserts coalesce into one change (a
//! typed word undoes as a word, not a letter at a time); any other action —
//! motion, click, enter, delete, save — closes the run. Buffer-content undo
//! is deliberately separate from the scene `(undo)`: text is text.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cosmic_text::{Action, Attrs, AttrsList, Buffer, Change, Color, Cursor, Edit, Editor,
    Family, FontSystem, Metrics, Motion, Selection, Shaping, Style, SwashCache, Weight};
use cosmic_text::{CacheKeyFlags, PhysicalGlyph, Renderer, SubpixelBin};
use modit::{Event as ViEvent, Key as ViKey, Parser as _, ViMode, ViParser, WordIter};

use crate::framebuffer::{Framebuffer, Rect};
use crate::language::{self, Language, ParenPos, Tok, find_partner};

/// Inner padding between the pane border and the text — textpane's PAD.
const PAD: f32 = 8.0;

/// Line height as a multiple of the font px (cosmic-text lays out with real
/// leading, unlike the transcript's char-grid).
pub const LEADING: f32 = 1.4;

/// Undo depth per pane — plenty for a scene file, bounded for sanity.
const MAX_UNDO: usize = 500;

thread_local! {
    /// Monotonic buffer-id source, shared across all editor panes and
    /// surviving a `set_font` state rebuild (the script layer names buffers
    /// by these, so they must stay stable).
    static NEXT_BUF_ID: Cell<u32> = const { Cell::new(1) };
}

fn mint_buf_id() -> u32 {
    NEXT_BUF_ID.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    })
}

/// One open document — the editing state that must survive a buffer switch
/// (S5). Everything here is per-buffer; the enclosing [`PaneState`] just says
/// which `Doc` is on screen. Closures throughout take `pane: &mut Doc` (the
/// name is historical — pre-S5 there was one doc per pane), so their bodies
/// read unchanged.
/// A decoded raster image, for view-only image buffers (a .png opened in the
/// editor). RGBA8, row-major; blitted scale-to-fit at draw.
struct ImageBuf {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

struct Doc {
    /// Stable buffer id — the handle Scheme uses to name/switch/kill.
    id: u32,
    /// Display name for the modeline (Scheme policy: basename, uniquified).
    name: String,
    /// The file this buffer visits (None = scratch / generated doc). Owned
    /// HERE, not in a script-side table (review item 1c): the registry that
    /// used to mirror this in `*buffers*` could drift from the real buffer
    /// list and needed a janitor (prune-mru); single-ownership removes the
    /// drift class.
    path: Option<String>,
    /// Showing this buffer is what auto-opened the editor dock, so quitting
    /// it ('q' in a reading view) closes the dock again.
    auto_opened: bool,
    /// What the modeline says after the name — the buffer's place in the
    /// loaded scene ("scene", "part of view-classic.scm"), empty for none.
    /// Scheme policy writes it (docs/modeler-scene-buffers-plan.md, B2).
    tag: String,
    /// The dialect of the language the text is written for, which the
    /// modeline gives in brackets: "ray" makes "Scheme (ray)". Empty for
    /// none; Scheme policy writes it (the dialect detectors, 07-backend).
    dialect: String,
    /// Some when this is an IMAGE buffer (view-only): draw blits it instead
    /// of the text editor, and edits are refused.
    image: Option<ImageBuf>,
    /// A movie (crate::video): its frames, and which one shows.
    video: Option<crate::video::Player>,
    /// Some for a MARKDOWN buffer (view-only): the styled spans are baked in
    /// and the lexer is skipped so it can't overwrite them. Edits
    /// refused. Carries the source so the reading-view toggle can re-render.
    md: Option<Markdown>,
    editor: Editor<'static>,
    /// The metrics px last applied to the buffer.
    applied_px: f32,
    /// The line-number column's width in physical pixels, as the last draw
    /// laid it out (0 with numbers off, or in a markdown or picture
    /// buffer): the mouse and the completion popup offset by it.
    gutter: f32,
    /// Modified since the last set_text / mark_saved.
    dirty: bool,
    /// An insert-coalescing change is open (see the module doc).
    insert_run: bool,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Per-line signature of the last-applied syntax spans (0 = unlexed) —
    /// only lines whose spans changed pay the reshape.
    line_sigs: Vec<u64>,
    /// The last scan, and what was last applied from it (see [`LexCache`]).
    lex: Option<LexCache>,
    /// The vi state machine (modit — the same parser COSMIC's ViEditor
    /// uses). Only consulted while the global vi toggle is on. Per-doc so a
    /// buffer remembers its NORMAL/INSERT mode across a switch (evil-style).
    vi: ViParser,
    /// Vi registers: yank/put targets, the unnamed register included.
    registers: HashMap<char, (Selection, String)>,
    /// The last `/`//`?` search, and its direction.
    search: Option<(String, bool)>,
    /// What each evaluated stretch of text built, anchored where it starts
    /// and carried through every edit (see [`Owner`]). Dropped when the
    /// text is replaced wholesale — a reload is a fresh document.
    owners: Vec<Owner>,
    /// How this buffer's text lexes and indents. Scheme for every buffer
    /// until a second language exists (docs/editor-crate-plan.md E6).
    lang: &'static dyn Language,
}

/// The scene keys a form of this buffer made the last time it ran, held at
/// the form's start so they follow the form as text is typed around it.
/// `C-M-x` on a form with no name for its shape makes a fresh key every
/// run; this is how the next run knows which entry to replace
/// (docs/modeler-scene-buffers-plan.md, E1).
#[derive(Clone)]
struct Owner {
    at: Cursor,
    keys: Vec<String>,
}

/// Move `c` through one edit, as the text it points into moves: text
/// inserted at or before it pushes it along, text deleted around it pulls
/// it to the deletion's start.
fn shift_cursor(c: &mut Cursor, item: &cosmic_text::ChangeItem) {
    let before = |a: &Cursor, b: &Cursor| (a.line, a.index) < (b.line, b.index);
    let (s, e) = (item.start, item.end);
    if item.insert {
        if before(c, &s) {
            return;
        }
        if c.line == s.line {
            c.index = e.index + (c.index - s.index);
            c.line = e.line;
        } else {
            c.line += e.line - s.line;
        }
    } else {
        if !before(&s, c) {
            return;
        }
        if before(c, &e) {
            *c = s;
        } else if c.line == e.line {
            c.index = s.index + (c.index - e.index);
            c.line = s.line;
        } else {
            c.line -= e.line - s.line;
        }
    }
}

/// Carry every owner through CHANGE, in its items' order.
fn shift_owners(owners: &mut [Owner], change: &Change) {
    for item in &change.items {
        for o in owners.iter_mut() {
            shift_cursor(&mut o.at, item);
        }
    }
}

/// An editor pane: a stack of open [`Doc`]s and which one is showing. S5's
/// buffer list — switching (`C-x b`) just moves `active`, so each buffer's
/// cursor, scroll, undo, dirty flag, and syntax cache are preserved.
struct PaneState {
    docs: Vec<Doc>,
    active: usize,
    /// Buffer ids, most-recently-visited first (C-x b's default = second
    /// entry). Touched by new/switch, pruned by close — no script-side
    /// janitor (item 1c).
    mru: Vec<u32>,
}

impl PaneState {
    fn new() -> Self {
        let first = mint_buf_id();
        Self { docs: vec![Doc::new(first)], active: 0, mru: vec![first] }
    }

    /// Move `buf` to the MRU front (visit order, most recent first).
    fn mru_touch(&mut self, buf: u32) {
        self.mru.retain(|&b| b != buf);
        self.mru.insert(0, buf);
    }

    fn doc(&self) -> &Doc {
        &self.docs[self.active]
    }

    fn doc_mut(&mut self) -> &mut Doc {
        &mut self.docs[self.active]
    }

    fn index_of(&self, buf: u32) -> Option<usize> {
        self.docs.iter().position(|d| d.id == buf)
    }

    /// Open a fresh empty buffer and make it active; returns its id.
    fn buffer_new(&mut self) -> u32 {
        let id = mint_buf_id();
        self.docs.push(Doc::new(id));
        self.active = self.docs.len() - 1;
        self.mru_touch(id);
        id
    }

    /// Make `buf` the active buffer (closing any open insert-run/completion on
    /// the outgoing doc first). Returns whether the buffer existed.
    fn buffer_switch(&mut self, buf: u32) -> bool {
        match self.index_of(buf) {
            Some(i) => {
                self.doc_mut().close_insert_run();
                self.active = i;
                self.mru_touch(buf);
                true
            }
            None => false,
        }
    }

    /// Drop a buffer. Refuses to remove the last one (leaves it empty, like
    /// Emacs keeping *scratch*). Returns whether it was removed.
    fn buffer_close(&mut self, buf: u32) -> bool {
        let Some(i) = self.index_of(buf) else { return false };
        if self.docs.len() == 1 {
            return false;
        }
        self.docs.remove(i);
        self.mru.retain(|&b| b != buf);
        if self.active >= self.docs.len() {
            self.active = self.docs.len() - 1;
        } else if self.active > i {
            self.active -= 1;
        }
        true
    }
}

impl Doc {
    fn new(id: u32) -> Self {
        // No cosmic auto-indent: Enter computes the LISP indent itself
        // (innermost open paren's column + 2) via the lexer.
        Self {
            id,
            name: "*scratch*".to_string(),
            path: None,
            auto_opened: false,
            tag: String::new(),
            dialect: String::new(),
            image: None,
            video: None,
            md: None,
            editor: Editor::new(Buffer::new_empty(Metrics::new(14.0, 14.0 * LEADING))),
            applied_px: 0.0,
            gutter: 0.0,
            dirty: false,
            insert_run: false,
            undo: Vec::new(),
            redo: Vec::new(),
            line_sigs: Vec::new(),
            lex: None,
            vi: ViParser::new(),
            registers: HashMap::new(),
            search: None,
            owners: Vec::new(),
            lang: &language::SCHEME,
        }
    }

    /// Does this buffer read as its language — so the reader's diagnostics
    /// apply? Text: a scratch buffer, or a file named as Scheme. A `.janet`
    /// session file, a `.txt` or a `.json` is also edited here, and the
    /// Scheme reader's complaints about it would be noise. (One language
    /// for every buffer until E6 picks one per file.)
    fn reads(&self) -> bool {
        self.md.is_none()
            && self.image.is_none()
            && self.video.is_none()
            && self.path.as_deref().is_none_or(|p| {
                let p = p.to_ascii_lowercase();
                [".scm", ".ss", ".sld", ".sls"].iter().any(|ext| p.ends_with(ext))
            })
    }

    /// Close an open insert-coalescing run, banking it as one undo step.
    fn close_insert_run(&mut self) {
        if self.insert_run {
            self.insert_run = false;
            if let Some(change) = self.editor.finish_change()
                && !change.items.is_empty() {
                    self.push_undo(change);
                }
        }
    }

    fn push_undo(&mut self, change: Change) {
        shift_owners(&mut self.owners, &change);
        self.dirty = true;
        self.undo.push(change);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Run `f` inside a discrete change; bank it if it edited anything.
    fn discrete<R>(&mut self, f: impl FnOnce(&mut Editor<'static>) -> R) -> R {
        self.close_insert_run();
        self.editor.start_change();
        let r = f(&mut self.editor);
        if let Some(change) = self.editor.finish_change()
            && !change.items.is_empty() {
                self.push_undo(change);
            }
        r
    }
}

struct State {
    font_system: FontSystem,
    cache: SwashCache,
    /// The mono face's family name inside our private fontdb — the pane
    /// shapes with exactly the font the text panes draw (JetBrains Mono by
    /// default; `set-text-font` swaps both).
    family: String,
    panes: HashMap<u32, PaneState>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Build the state around one font: a fontdb holding ONLY our mono face —
/// no system-font discovery (startup cost, machine-dependent fallback), and
/// glyph flavor identical to the rest of the UI.
fn build_state(font_data: Vec<u8>) -> State {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(font_data);
    let family = db
        .faces()
        .next()
        .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
        .unwrap_or_else(|| "monospace".to_string());
    State {
        font_system: FontSystem::new_with_locale_and_db("en-US".to_string(), db),
        cache: SwashCache::new(),
        family,
        panes: HashMap::new(),
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let state = s.get_or_insert_with(|| build_state(crate::text::mono_font_bytes()));
        f(state)
    })
}

/// Run `f` against a pane's ACTIVE document. The closure param is named
/// `pane` for historical reasons but is a `&mut Doc` — pre-S5 there was one
/// doc per pane, so every existing body reads unchanged.
fn with_pane<R>(id: u32, f: impl FnOnce(&mut FontSystem, &mut Doc, &str) -> R) -> R {
    with_state(|state| {
        let pane = state.panes.entry(id).or_insert_with(PaneState::new);
        f(&mut state.font_system, pane.doc_mut(), &state.family)
    })
}

/// Run `f` against a pane's view (the buffer list) — for the S5 buffer ops.
fn with_view<R>(id: u32, f: impl FnOnce(&mut PaneState) -> R) -> R {
    with_state(|state| f(state.panes.entry(id).or_insert_with(PaneState::new)))
}

fn buffer_text(editor: &Editor<'static>) -> String {
    editor.with_buffer(|buffer| {
        let mut out = String::new();
        for (i, line) in buffer.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line.text());
        }
        out
    })
}

/// Replace a pane's whole buffer (the Scheme side just slurped a file).
/// Cursor, scroll, dirty flag, and both undo stacks reset — a fresh
/// document, not an edit.
pub fn set_text(id: u32, text: &str) {
    with_pane(id, |fs, pane, family| {
        pane.editor.with_buffer_mut(|buffer| {
            buffer.set_text(text, &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
        });
        pane.editor.set_cursor(Cursor::default());
        pane.editor.set_selection(Selection::None);
        pane.editor.shape_as_needed(fs, false);
        pane.image = None; // a text load turns an image buffer back to text
        pane.video = None;
        pane.md = None; // ...and out of markdown view-only mode
        pane.dirty = false;
        pane.insert_run = false;
        pane.undo.clear();
        pane.redo.clear();
        pane.owners.clear();
        // set_text rebuilt every line with DEFAULT attrs — the lex cache
        // must forget its "already colored" sigs or a reopen with the same
        // text skips re-applying the spans (highlighting-vanishes bug,
        // user report 2026-09-08).
        pane.line_sigs.clear();
    });
    crate::host::mark_dirty_ui();
}

/// The buffer's content — what save writes and tests assert on. Lines join
/// with '\n' (the buffer normalizes endings on load).
pub fn text(id: u32) -> String {
    with_pane(id, |_, pane, _| buffer_text(&pane.editor))
}

/// Scroll by text rows (positive = toward the end of the file). Rows scale
/// by the current line height, so a wheel notch means the same amount at
/// every text size.
pub fn scroll(id: u32, rows: f32) {
    with_pane(id, |fs, pane, _| {
        let line_h = pane.editor.with_buffer(|b| b.metrics().line_height);
        pane.editor.action(fs, Action::Scroll { pixels: rows * line_h });
    });
    crate::host::mark_dirty_ui();
}

pub fn dirty(id: u32) -> bool {
    with_pane(id, |_, pane, _| pane.dirty)
}

/// The cursor as (line, byte-index-in-line) — what the Scheme sexp scanner
/// anchors "send expression at point" on.
pub fn cursor(id: u32) -> (usize, usize) {
    with_pane(id, |_, pane, _| {
        let c = pane.editor.cursor();
        (c.line, c.index)
    })
}

/// Place the cursor at the start of LOGICAL line `line` (clamped), clearing any
/// selection — Emacs's goto-line. Unlike the `"down"` motion (which steps by
/// VISUAL row and so miscounts when lines wrap in a narrow pane), this addresses
/// the logical line directly, so scripted positioning is layout-independent.
pub fn goto_line(id: u32, line: usize) {
    with_pane(id, |fs, pane, _| {
        let last = pane.editor.with_buffer(|b| b.lines.len().saturating_sub(1));
        let line = line.min(last);
        pane.editor.set_selection(Selection::None);
        pane.editor.set_cursor(Cursor::new(line, 0));
        pane.editor.shape_as_needed(fs, false);
    });
    crate::host::mark_dirty_ui();
}

/// Select the whole of LOGICAL line `line` (clamped, excluding its newline) and
/// leave the cursor at its end — the keyboard counterpart to a triple-click,
/// for driving region sends deterministically regardless of wrapping.
pub fn select_line(id: u32, line: usize) {
    with_pane(id, |fs, pane, _| {
        let (last, len) = pane.editor.with_buffer(|b| {
            let last = b.lines.len().saturating_sub(1);
            (last, b.lines[line.min(last)].text().len())
        });
        let line = line.min(last);
        pane.editor.set_selection(Selection::Normal(Cursor::new(line, 0)));
        pane.editor.set_cursor(Cursor::new(line, len));
        pane.editor.shape_as_needed(fs, false);
    });
    crate::host::mark_dirty_ui();
}

/// The cursor's WINDOW position and the line height — where the completion
/// popup anchors. None before the first layout.
pub fn cursor_window_pos(id: u32) -> Option<(f32, f32, f32)> {
    let rect = crate::host::rect(id)?;
    let pad = PAD * crate::host::ui_scale();
    with_pane(id, |fs, pane, _| {
        pane.editor.shape_as_needed(fs, false);
        pane.editor.cursor_position().map(|(cx, cy)| {
            let line_h = pane.editor.with_buffer(|b| b.metrics().line_height);
            (rect.x as f32 + pad + pane.gutter + cx as f32, rect.y as f32 + pad + cy as f32, line_h)
        })
    })
}

/// Replace the byte range (l0,i0)..(l1,i1) with `text` as one undo step,
/// cursor landing after the insertion — completion's token swap.
pub fn replace_range(id: u32, l0: usize, i0: usize, l1: usize, i1: usize, text: &str) {
    with_pane(id, |_, pane, _| {
        pane.discrete(|e| {
            e.delete_range(Cursor::new(l0, i0), Cursor::new(l1, i1));
            let after = e.insert_at(Cursor::new(l0, i0), text, None);
            e.set_cursor(after);
            e.set_selection(Selection::None);
        });
    });
    crate::host::mark_dirty_ui();
}

// ── Search and replace (Kevin, 2026-11-14) ──────────────────────────────────
// Plain text within a line; `fold` matches ASCII case-insensitively, which
// keeps every byte offset the same in the folded copy.

fn fold_line(s: &str, fold: bool) -> std::borrow::Cow<'_, str> {
    if fold { std::borrow::Cow::Owned(s.to_ascii_lowercase()) } else { std::borrow::Cow::Borrowed(s) }
}

/// The lines' text, for a search.
fn lines_of(pane: &Doc) -> Vec<String> {
    pane.editor.with_buffer(|b| b.lines.iter().map(|l| l.text().to_string()).collect())
}

/// Every match of `needle` in `lines` at or after (line, byte), in order.
fn matches_from(lines: &[String], needle: &str, fold: bool, from: (usize, usize)) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let needle = fold_line(needle, fold);
    for (l, text) in lines.iter().enumerate().skip(from.0) {
        let hay = fold_line(text, fold);
        let mut at = if l == from.0 { from.1.min(hay.len()) } else { 0 };
        while !hay.is_char_boundary(at) {
            at += 1;
        }
        while let Some(i) = hay[at..].find(needle.as_ref()) {
            let s = at + i;
            out.push((l, s, s + needle.len()));
            at = s + needle.len();
        }
    }
    out
}

/// The next match of `needle` from (line, byte) — forward from there, or
/// backward from before it — wrapping round the buffer once: the match as
/// (line, start, end) and whether it wrapped.
pub fn find(id: u32, needle: &str, from: (usize, usize), forward: bool, fold: bool) -> Option<((usize, usize, usize), bool)> {
    with_pane(id, |_, pane, _| {
        let lines = lines_of(pane);
        let all = matches_from(&lines, needle, fold, (0, 0));
        if forward {
            all.iter().find(|m| (m.0, m.1) >= from).map(|m| (*m, false)).or_else(|| all.first().map(|m| (*m, true)))
        } else {
            all.iter().rev().find(|m| (m.0, m.1) < from).map(|m| (*m, false)).or_else(|| all.last().map(|m| (*m, true)))
        }
    })
}

/// Select (line, i0..i1) with the cursor at its end, scrolled into view —
/// how a search shows its match.
pub fn select_range(id: u32, line: usize, i0: usize, i1: usize) {
    with_pane(id, |fs, pane, _| {
        pane.editor.set_selection(Selection::Normal(Cursor::new(line, i0)));
        pane.editor.set_cursor(Cursor::new(line, i1));
        pane.editor.shape_as_needed(fs, false);
    });
    crate::host::mark_dirty_ui();
}

/// Replace every match of `needle` that starts in [from, to) with `with`, as
/// ONE undo step: how many. A `to` before `from` wraps — from `from` to the
/// end, then the top down to `to` — the rest of a query-replace's run.
pub fn replace_all(id: u32, needle: &str, with: &str, fold: bool, from: (usize, usize), to: (usize, usize)) -> usize {
    let n = with_pane(id, |_, pane, _| {
        let found: Vec<_> = matches_from(&lines_of(pane), needle, fold, (0, 0))
            .into_iter()
            .filter(|m| {
                let at = (m.0, m.1);
                if to > from { at >= from && at < to } else { at >= from || at < to }
            })
            .collect();
        if !found.is_empty() {
            pane.discrete(|e| {
                // Back to front, so each replacement leaves the earlier
                // matches' offsets as they were.
                for &(l, i0, i1) in found.iter().rev() {
                    e.delete_range(Cursor::new(l, i0), Cursor::new(l, i1));
                    e.insert_at(Cursor::new(l, i0), with, None);
                }
                e.set_selection(Selection::None);
            });
        }
        found.len()
    });
    crate::host::mark_dirty_ui();
    n
}

/// The selected text, or `None` when there is none to take — the one way
/// copy, cut and a vi yank read a selection. cosmic-text's own
/// `copy_selection` indexes the buffer's first line, and a buffer that was
/// never given text has NO lines (`Buffer::new_empty`; even `set_text("")`
/// leaves it empty): Copy on a fresh buffer after a click — which leaves a
/// selection anchor — panicked the whole program (Kevin, 2026-11-13, the
/// marking menu's Copy). A selection that is only a point copies nothing.
fn selection_text(editor: &Editor<'static>) -> Option<String> {
    if editor.with_buffer(|b| b.lines.is_empty()) {
        return None;
    }
    editor.copy_selection().filter(|t| !t.is_empty())
}

/// The cursor at character offset `off` of [`buffer_text`] (lines joined
/// by one `\n`), clamped to the end. Scheme counts characters; a cursor's
/// index is a byte offset into its line.
fn cursor_of_char(editor: &Editor<'static>, off: usize) -> Cursor {
    editor.with_buffer(|buffer| {
        let mut left = off;
        let last = buffer.lines.len().saturating_sub(1);
        for (i, line) in buffer.lines.iter().enumerate() {
            let text = line.text();
            let n = text.chars().count();
            if left <= n || i == last {
                let byte = text.char_indices().nth(left).map_or(text.len(), |(b, _)| b);
                return Cursor::new(i, byte);
            }
            left -= n + 1;
        }
        Cursor::new(0, 0)
    })
}

/// The character offset of `c` in [`buffer_text`].
fn char_of_cursor(editor: &Editor<'static>, c: Cursor) -> usize {
    editor.with_buffer(|buffer| {
        let mut off = 0;
        for (i, line) in buffer.lines.iter().enumerate() {
            let text = line.text();
            if i == c.line {
                return off + text[..c.index.min(text.len())].chars().count();
            }
            off += text.chars().count() + 1;
        }
        off.saturating_sub(1)
    })
}

/// Take the keys owned by every anchor at a character offset in
/// `start..end` of the active buffer, removing those anchors: the text
/// there is about to run again, and what it makes replaces what they hold.
pub fn take_owned(id: u32, start: usize, end: usize) -> Vec<String> {
    with_pane(id, |_, pane, _| {
        pane.close_insert_run(); // bank a typed run, so the anchors are current
        let (mut taken, mut kept) = (Vec::new(), Vec::new());
        for o in pane.owners.drain(..) {
            let at = char_of_cursor(&pane.editor, o.at);
            if start <= at && at < end.max(start + 1) {
                taken.extend(o.keys);
            } else {
                kept.push(o);
            }
        }
        pane.owners = kept;
        taken.sort();
        taken.dedup();
        taken
    })
}

/// Anchor `keys` at character offset `at` of the active buffer.
pub fn own(id: u32, at: usize, keys: Vec<String>) {
    with_pane(id, |_, pane, _| {
        pane.close_insert_run();
        let at = cursor_of_char(&pane.editor, at);
        pane.owners.push(Owner { at, keys });
    });
}

/// Forget every anchor in the active buffer (a whole-buffer send is about
/// to set them all again).
pub fn owned_clear(id: u32) {
    with_pane(id, |_, pane, _| pane.owners.clear());
}

/// Every anchor of the active buffer as (character offset, keys), in
/// offset order — for tests and for asking what a form owns.
pub fn owners(id: u32) -> Vec<(usize, Vec<String>)> {
    with_pane(id, |_, pane, _| {
        pane.close_insert_run();
        let mut out: Vec<(usize, Vec<String>)> =
            pane.owners.iter().map(|o| (char_of_cursor(&pane.editor, o.at), o.keys.clone())).collect();
        out.sort();
        out
    })
}

/// The call the cursor is inside — eldoc's anchor: in `(add-sphere :at [1
/// 2|` that's "add-sphere". The buffer's language decides
/// ([`Language::enclosing_head`]).
pub fn enclosing_head(id: u32) -> Option<String> {
    with_pane(id, |_, pane, _| {
        let cursor = pane.editor.cursor();
        pane.editor.with_buffer(|buffer| {
            let lines: Vec<&str> = buffer.lines.iter().map(|l| l.text()).collect();
            pane.lang.enclosing_head(&lines, cursor.line, cursor.index)
        })
    })
}

/// **What watches the buffers from outside** — a language server
/// (`crate::lsp`, E7). The widget tells it each file buffer's text when the
/// text changes, and draws what it says is wrong beside the language's own
/// read marks. The widget does not know what is watching.
pub trait Watcher: Sync {
    /// The buffer visiting PATH, in language LANG, holds LINES now.
    fn text_changed(&self, path: &str, lang: &str, lines: &[&str]);
    /// What is wrong with the buffer visiting PATH, in the widget's units.
    fn diagnostics(&self, path: &str) -> Vec<language::Diagnostic>;
    /// No buffer visits PATH any more: it was closed, or it visits another
    /// file now.
    fn closed(&self, _path: &str) {}
    /// A word or two for the modeline of the buffer visiting PATH in LANG —
    /// how what watches it is ("rust-analyzer indexing (40%)") — or None.
    fn mode(&self, _path: &str, _lang: &str) -> Option<String> {
        None
    }
}

thread_local! {
    static WATCHERS: RefCell<Vec<&'static dyn Watcher>> = const { RefCell::new(Vec::new()) };
}

/// Install a watcher beside the others — the language server's (E7), the
/// warnings (E8). Installing one twice keeps one.
pub fn set_watcher(w: &'static dyn Watcher) {
    WATCHERS.with(|ws| {
        let mut ws = ws.borrow_mut();
        if !ws.iter().any(|x| std::ptr::addr_eq(*x as *const dyn Watcher, w as *const dyn Watcher)) {
            ws.push(w);
        }
    });
}

/// The watchers, copied out so none is borrowed while one runs.
fn watchers() -> Vec<&'static dyn Watcher> {
    WATCHERS.with(|ws| ws.borrow().clone())
}

#[cfg(test)]
fn clear_watchers() {
    WATCHERS.with(|ws| ws.borrow_mut().clear());
}

/// Tell the watchers PATH was let go, unless a buffer in some pane still
/// visits it.
fn release(path: Option<String>) {
    let Some(path) = path else { return };
    let held = with_state(|state| state.panes.values().any(|p| p.docs.iter().any(|d| d.path.as_deref() == Some(path.as_str()))));
    if !held {
        watchers().iter().for_each(|w| w.closed(&path));
    }
}

/// The watchers' modeline word for a buffer that is text with a file: the
/// first that has one.
fn watched_mode(doc: &Doc) -> Option<String> {
    if doc.md.is_some() || doc.image.is_some() || doc.video.is_some() {
        return None;
    }
    let path = doc.path.as_deref()?;
    watchers().iter().find_map(|w| w.mode(path, doc.lang.name()))
}

/// The watchers' diagnostics for a buffer that is text with a file.
fn watched_diagnostics(doc: &Doc) -> Vec<language::Diagnostic> {
    if doc.md.is_some() || doc.image.is_some() || doc.video.is_some() {
        return Vec::new();
    }
    match doc.path.as_deref() {
        Some(path) => watchers().iter().flat_map(|w| w.diagnostics(path)).collect(),
        None => Vec::new(),
    }
}

/// What the active buffer's language cannot read (E3f), from the scan the
/// draw keeps — made now if the text has changed since — and what the
/// [`Watcher`] says.
pub fn diagnostics(id: u32) -> Vec<language::Diagnostic> {
    with_pane(id, |_, pane, _| {
        let reads = pane.reads();
        let (lex, lang, path) = (&mut pane.lex, pane.lang, pane.path.as_deref());
        pane.editor.with_buffer(|buffer| fresh_scan(lex, lang, reads, buffer, path));
        let mut diags = pane.lex.as_ref().map_or_else(Vec::new, |c| c.diags.clone());
        diags.extend(watched_diagnostics(pane));
        diags
    })
}

/// Set the ACTIVE buffer's language by name ("scheme", "guile"): its
/// highlighting, indentation and read marks follow. False for a name no
/// language answers to.
pub fn set_language(id: u32, name: &str) -> bool {
    let Some(lang) = language::by_name(name) else { return false };
    with_pane(id, |_, pane, _| {
        pane.lang = lang;
        pane.lex = None;
        pane.line_sigs.clear();
    });
    crate::host::mark_dirty_ui();
    true
}

/// The language of buffer `buf` in pane `id`, by name.
pub fn buffer_language(id: u32, buf: u32) -> Option<&'static str> {
    with_doc(id, buf, |d| d.lang.name())
}

/// The buffer now matches the file on disk (save just wrote it).
pub fn mark_saved(id: u32) {
    with_pane(id, |_, pane, _| {
        pane.close_insert_run();
        pane.dirty = false;
    });
    crate::host::mark_dirty_ui();
}

// ── S5: multiple buffers per pane (docs/modeler-scene-editor-plan.md) ──────

/// Open a fresh empty buffer in the pane and make it active; returns its id.
pub fn buffer_new(id: u32) -> u32 {
    let buf = with_view(id, |pane| pane.buffer_new());
    crate::host::mark_dirty_ui();
    buf
}

/// Make `buf` the pane's active buffer. Returns whether it existed.
pub fn buffer_switch(id: u32, buf: u32) -> bool {
    let ok = with_view(id, |pane| pane.buffer_switch(buf));
    if ok {
        crate::host::mark_dirty_ui();
    }
    ok
}

/// Drop `buf` from the pane (never the last one). Returns whether it went.
pub fn buffer_close(id: u32, buf: u32) -> bool {
    let path = buffer_path(id, buf);
    let ok = with_view(id, |pane| pane.buffer_close(buf));
    if ok {
        release(path);
        crate::host::mark_dirty_ui();
    }
    ok
}

/// The pane's buffers as `(id, dirty)` pairs, in stored order.
pub fn buffer_list(id: u32) -> Vec<(u32, bool)> {
    with_view(id, |pane| pane.docs.iter().map(|d| (d.id, d.dirty)).collect())
}

// ── The buffer REGISTRY (review item 1c) ────────────────────────────────────
// Path / display name / auto-opened flag / MRU order live on the Doc itself —
// the same store that owns the text — instead of the parallel Janet table
// that once drifted from the real buffer list. Scheme keeps the POLICY (find-file,
// C-x b defaults, uniquified naming); these are the facts it consults.

fn with_doc<R>(id: u32, buf: u32, f: impl FnOnce(&mut Doc) -> R) -> Option<R> {
    with_view(id, |pane| pane.index_of(buf).map(|i| f(&mut pane.docs[i])))
}

/// The file `buf` visits, if any.
pub fn buffer_path(id: u32, buf: u32) -> Option<String> {
    with_doc(id, buf, |d| d.path.clone()).flatten()
}

/// Point `buf` at a file path (None = scratch).
pub fn set_buffer_path(id: u32, buf: u32, path: Option<&str>) -> bool {
    let old = with_doc(id, buf, |d| std::mem::replace(&mut d.path, path.map(str::to_string)));
    match old {
        Some(old) => {
            if old.as_deref() != path {
                release(old);
            }
            true
        }
        None => false,
    }
}

/// `buf`'s modeline display name.
pub fn buffer_name(id: u32, buf: u32) -> Option<String> {
    with_doc(id, buf, |d| d.name.clone())
}

/// Set `buf`'s modeline display name (Scheme policy: basename, uniquified).
pub fn set_buffer_name(id: u32, buf: u32, name: &str) -> bool {
    let ok = with_doc(id, buf, |d| d.name = name.to_string()).is_some();
    if ok {
        crate::host::mark_dirty_ui();
    }
    ok
}

/// The buffer whose display name is exactly `name`.
pub fn buffer_by_name(id: u32, name: &str) -> Option<u32> {
    with_view(id, |pane| pane.docs.iter().find(|d| d.name == name).map(|d| d.id))
}

/// The buffer visiting `path`.
pub fn buffer_by_path(id: u32, path: &str) -> Option<u32> {
    with_view(id, |pane| {
        pane.docs.iter().find(|d| d.path.as_deref() == Some(path)).map(|d| d.id)
    })
}

/// Buffer ids, most-recently-visited first. Always consistent with the real
/// buffer list (close prunes) — the property the old Janet-side registry needed
/// a janitor for.
pub fn buffer_mru(id: u32) -> Vec<u32> {
    with_view(id, |pane| pane.mru.clone())
}

/// Whether showing `buf` is what auto-opened the editor dock.
pub fn buffer_auto_opened(id: u32, buf: u32) -> bool {
    with_doc(id, buf, |d| d.auto_opened).unwrap_or(false)
}

pub fn set_buffer_auto_opened(id: u32, buf: u32, on: bool) -> bool {
    with_doc(id, buf, |d| d.auto_opened = on).is_some()
}

/// `buf`'s modeline tag — its place in the loaded scene, or empty.
pub fn buffer_tag(id: u32, buf: u32) -> Option<String> {
    with_doc(id, buf, |d| d.tag.clone())
}

/// `buf`'s dialect ("ray"), or empty.
pub fn buffer_dialect(id: u32, buf: u32) -> Option<String> {
    with_doc(id, buf, |d| d.dialect.clone())
}

/// Set `buf`'s dialect (Scheme policy): the modeline says "Scheme (ray)".
pub fn set_buffer_dialect(id: u32, buf: u32, dialect: &str) -> bool {
    let ok = with_doc(id, buf, |d| d.dialect = dialect.to_string()).is_some();
    if ok {
        crate::host::mark_dirty_ui();
    }
    ok
}

/// Set `buf`'s modeline tag (Scheme policy).
pub fn set_buffer_tag(id: u32, buf: u32, tag: &str) -> bool {
    let ok = with_doc(id, buf, |d| d.tag = tag.to_string()).is_some();
    if ok {
        crate::host::mark_dirty_ui();
    }
    ok
}

/// The pane's active buffer id.
pub fn active_buffer(id: u32) -> u32 {
    with_view(id, |pane| pane.doc().id)
}

/// Set the active buffer's modeline display name (Scheme policy).
pub fn set_name(id: u32, name: &str) {
    with_pane(id, |_, pane, _| pane.name = name.to_string());
    crate::host::mark_dirty_ui();
}

/// Turn the active buffer into a view-only MOVIE buffer playing `path`
/// (crate::video: ffmpeg decodes it in the background, and it starts
/// playing as the first frame arrives). Always succeeds; a clip that cannot
/// play says why on its pane.
pub fn open_video(id: u32, path: &str) -> bool {
    let player = crate::video::Player::open(path);
    with_pane(id, |fs, pane, family| {
        pane.editor.with_buffer_mut(|buffer| {
            buffer.set_text("", &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
        });
        pane.editor.shape_as_needed(fs, false);
        pane.image = None;
        pane.md = None;
        pane.video = Some(player);
        pane.dirty = false;
        pane.undo.clear();
        pane.redo.clear();
        pane.owners.clear();
        pane.line_sigs.clear();
    });
    crate::host::mark_dirty_ui();
    true
}

/// A movie buffer's controls, by name — the menu's and the keys':
/// "toggle", "play", "pause", "restart", "loop", "step" (+1), "back" (-1),
/// "end". Answers (frame decoded playing looping), or None when the active
/// buffer is not a movie.
pub fn video_control(id: u32, what: &str) -> Option<(usize, usize, bool, bool)> {
    let out = with_pane(id, |_, pane, _| {
        let v = pane.video.as_mut()?;
        match what {
            "toggle" => v.toggle(),
            "play" if !v.playing => v.toggle(),
            "pause" if v.playing => v.toggle(),
            "restart" => {
                v.seek(0);
                v.playing = true;
            }
            "loop" => v.looping = !v.looping,
            "step" => v.step(1),
            "back" => v.step(-1),
            "end" => {
                v.playing = false;
                v.seek(isize::MAX);
            }
            _ => {}
        }
        Some((v.frame, v.decoded(), v.playing, v.looping))
    });
    if out.is_some() && what != "state" {
        crate::host::mark_dirty_ui();
    }
    out
}

/// Run every playing movie's clock: whether a shown frame moved (the UI is
/// marked dirty too), and the earliest time one wants its next frame, for
/// the host's event loop to sleep until.
pub fn video_tick() -> (bool, Option<std::time::Instant>) {
    let now = std::time::Instant::now();
    let (changed, next) = with_state(|state| {
        let mut changed = false;
        let mut next: Option<std::time::Instant> = None;
        for pane in state.panes.values_mut() {
            for doc in pane.docs.iter_mut() {
                if let Some(v) = doc.video.as_mut() {
                    let (c, n) = v.tick(now);
                    changed |= c;
                    next = match (next, n) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
            }
        }
        (changed, next)
    });
    if changed {
        crate::host::mark_dirty_ui();
    }
    (changed, next)
}

/// Turn the active buffer into a view-only IMAGE buffer showing `path` (a
/// PNG or JPEG — `image::open` sniffs the format). Returns whether it decoded.
/// The text editor is emptied; edits are refused while an image is loaded.
pub fn open_image(id: u32, path: &str) -> bool {
    let decoded = match image::open(path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            ImageBuf { w: rgba.width(), h: rgba.height(), rgba: rgba.into_raw() }
        }
        Err(_) => return false,
    };
    with_pane(id, |fs, pane, family| {
        pane.editor.with_buffer_mut(|buffer| {
            buffer.set_text("", &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
        });
        pane.editor.shape_as_needed(fs, false);
        pane.image = Some(decoded);
        pane.video = None;
        pane.dirty = false;
        pane.undo.clear();
        pane.redo.clear();
        pane.owners.clear();
        pane.line_sigs.clear();
    });
    crate::host::mark_dirty_ui();
    true
}

// ── Markdown buffers (view-only) ────────────────────────────────────────────
// Two renderings share one styling primitive (`md_bake`). SOURCE view keeps the
// raw text and highlights each element in place (markup visible, markdown-mode
// style). READING view (C-c C-v) rebuilds a WYSIWYG string — markup stripped,
// `•` bullets / `─` rules / `▏ ` quote bars baked into the text, paragraphs
// reflowed — with larger heading text via cosmic-text's per-span metrics. Both
// are view-only. Deferred: tables, embedded images, code-block syntax colors.

/// A markdown buffer's state beyond its editor text.
struct Markdown {
    /// The raw file source — the toggle re-renders from this.
    src: String,
    /// true = WYSIWYG reading view; false = source highlighting.
    reading: bool,
    /// The base px the heading metrics were last baked at — re-bake on a UI
    /// scale change so headings track the body size.
    applied_px: f32,
}

/// One styling decision for a run of display text.
#[derive(Clone, Default)]
struct MdStyle {
    color: Option<u32>,
    bold: bool,
    italic: bool,
    /// Heading size as a multiple of the base px (None = body size).
    size_scale: Option<f32>,
}

/// The px the editor lays body text at (font size × UI scale) — the same
/// figure `draw` computes, so open/toggle bake headings at the right size.
fn editor_px() -> f32 {
    crate::textpane::font_px() * crate::host::ui_scale()
}

fn md_heading_scale(lvl: pulldown_cmark::HeadingLevel) -> f32 {
    use pulldown_cmark::HeadingLevel::*;
    match lvl {
        H1 => 1.7,
        H2 => 1.45,
        H3 => 1.25,
        H4 => 1.12,
        _ => 1.05,
    }
}

/// The ambient inline style for a run of reading-view text, from the open
/// tags around it. `heading` carries the heading size scale when inside one.
fn md_inline_style(
    theme: &crate::theme::Theme,
    heading: Option<f32>,
    bold: bool,
    italic: bool,
    link: bool,
    code_block: bool,
    quote: bool,
) -> MdStyle {
    let mut st = MdStyle::default();
    if let Some(scale) = heading {
        st.color = Some(theme.text_accent);
        st.bold = true;
        st.size_scale = Some(scale);
    } else if code_block {
        st.color = Some(theme.code_string);
    } else if quote {
        st.color = Some(theme.code_comment);
        st.italic = true;
    }
    if bold {
        st.bold = true;
    }
    if italic {
        st.italic = true;
    }
    if link {
        st.color = Some(theme.text_accent);
    }
    st
}

/// SOURCE view: text unchanged, one span per element's whole range (markup
/// chars stay visible). Overlapping spans land in document order, so an inner
/// element (bold inside a heading) overrides the outer.
fn md_layout_source(theme: &crate::theme::Theme, src: &str) -> (String, Vec<(std::ops::Range<usize>, MdStyle)>) {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut spans = Vec::new();
    for (ev, range) in Parser::new_ext(src, Options::empty()).into_offset_iter() {
        let m = match ev {
            Event::Start(Tag::Heading { .. }) =>
                MdStyle { color: Some(theme.text_accent), bold: true, ..Default::default() },
            Event::Start(Tag::Strong) =>
                MdStyle { color: Some(theme.code_number), bold: true, ..Default::default() },
            Event::Start(Tag::Emphasis) =>
                MdStyle { color: Some(theme.code_keyword), italic: true, ..Default::default() },
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) =>
                MdStyle { color: Some(theme.code_string), ..Default::default() },
            Event::Start(Tag::BlockQuote(_)) =>
                MdStyle { color: Some(theme.code_comment), ..Default::default() },
            Event::Start(Tag::Link { .. }) =>
                MdStyle { color: Some(theme.text_accent), ..Default::default() },
            _ => continue,
        };
        spans.push((range, m));
    }
    (src.to_string(), spans)
}

/// READING view: a WYSIWYG display string with markup stripped and structural
/// glyphs baked in, plus its styled spans. Paragraphs reflow (soft breaks →
/// spaces); the pane wraps them to width.
fn md_layout_reading(theme: &crate::theme::Theme, src: &str) -> (String, Vec<(std::ops::Range<usize>, MdStyle)>) {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    // Newline bookkeeping: `nl` ensures we're at a line start, `blank` ensures
    // a blank line before a new block (never a leading blank at the very top).
    fn nl(out: &mut String) {
        if !(out.is_empty() || out.ends_with('\n')) {
            out.push('\n');
        }
    }
    fn blank(out: &mut String) {
        nl(out);
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
    }
    // Append `s`, recording a span over it (no-op on empty).
    fn push(out: &mut String, spans: &mut Vec<(std::ops::Range<usize>, MdStyle)>, s: &str, st: MdStyle) {
        if s.is_empty() {
            return;
        }
        let start = out.len();
        out.push_str(s);
        spans.push((start..out.len(), st));
    }

    let mut out = String::new();
    let mut spans: Vec<(std::ops::Range<usize>, MdStyle)> = Vec::new();
    let (mut bold, mut italic, mut link) = (0u32, 0u32, 0u32);
    let mut code_block = false;
    let mut quote = 0u32;
    let mut heading: Option<HeadingLevel> = None;
    // One entry per open list: Some(next number) ordered, None a bullet list.
    let mut lists: Vec<Option<u64>> = Vec::new();
    // Just emitted a bullet marker — keep the item's first paragraph on its line.
    let mut at_item = false;
    // A table being read: its rows of cells (each cell its styled runs), set
    // out in aligned columns when it ends — the reading view is in the mono
    // face, so padding lines the columns up (Kevin, 2026-11-14: :help's
    // tables ran together into one paragraph).
    let mut table: Option<MdTable> = None;

    for (ev, _r) in Parser::new_ext(src, Options::ENABLE_TABLES).into_offset_iter() {
        // Inside a table cell, text goes to the cell.
        if let Some(t) = table.as_mut() {
            let st = md_inline_style(theme, None, bold > 0 || t.in_head, italic > 0, link > 0, false, false);
            match &ev {
                Event::Text(s) => {
                    t.push(s, st);
                    continue;
                }
                Event::Code(s) => {
                    t.push(s, MdStyle { color: Some(theme.code_string), ..Default::default() });
                    continue;
                }
                Event::SoftBreak | Event::HardBreak => {
                    t.push(" ", st);
                    continue;
                }
                _ => {}
            }
        }
        match &ev {
            Event::Start(Tag::Table(aligns)) => {
                blank(&mut out);
                table = Some(MdTable { aligns: aligns.clone(), rows: Vec::new(), head_rows: 0, in_head: false });
            }
            Event::Start(Tag::TableHead) => {
                if let Some(t) = table.as_mut() {
                    t.in_head = true;
                    t.rows.push(Vec::new());
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = table.as_mut() {
                    t.in_head = false;
                    t.head_rows = t.rows.len();
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(Vec::new());
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    t.lay_out(theme, &mut out, &mut spans);
                }
            }
            _ => {}
        }
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                blank(&mut out);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                heading = None;
                out.push('\n');
            }
            Event::Start(Tag::Paragraph) => {
                if at_item {
                    at_item = false; // stay on the bullet's line
                } else {
                    blank(&mut out);
                    if quote > 0 {
                        let bar = "▏ ".repeat(quote as usize);
                        push(&mut out, &mut spans, &bar, MdStyle { color: Some(theme.text_accent), ..Default::default() });
                    }
                }
            }
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => link += 1,
            Event::End(TagEnd::Link) => link = link.saturating_sub(1),
            Event::Start(Tag::BlockQuote(_)) => {
                blank(&mut out);
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => {
                blank(&mut out);
                code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block = false;
                nl(&mut out);
            }
            Event::Start(Tag::List(start)) => {
                if lists.is_empty() {
                    blank(&mut out);
                } else {
                    nl(&mut out);
                }
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                nl(&mut out);
                let depth = lists.len().saturating_sub(1);
                push(&mut out, &mut spans, &"  ".repeat(depth), MdStyle::default());
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                push(&mut out, &mut spans, &marker, MdStyle { color: Some(theme.text_accent), ..Default::default() });
                at_item = true;
            }
            Event::End(TagEnd::Item) => {
                out.push('\n');
                at_item = false;
            }
            Event::Text(t) => {
                let st = md_inline_style(
                    theme,
                    heading.map(md_heading_scale),
                    bold > 0,
                    italic > 0,
                    link > 0,
                    code_block,
                    quote > 0,
                );
                push(&mut out, &mut spans, &t, st);
                at_item = false;
            }
            Event::Code(t) => {
                push(&mut out, &mut spans, &t, MdStyle { color: Some(theme.code_string), ..Default::default() });
                at_item = false;
            }
            Event::SoftBreak => out.push(if code_block { '\n' } else { ' ' }),
            Event::HardBreak => out.push('\n'),
            Event::Rule => {
                blank(&mut out);
                push(&mut out, &mut spans, &"─".repeat(40), MdStyle { color: Some(theme.text_dim), ..Default::default() });
                out.push('\n');
            }
            _ => {}
        }
    }
    while out.ends_with('\n') {
        out.pop();
    }
    (out, spans)
}

/// A markdown table as it is read: rows of cells, each cell its styled runs.
struct MdTable {
    aligns: Vec<pulldown_cmark::Alignment>,
    rows: Vec<Vec<Vec<(String, MdStyle)>>>,
    /// How many of `rows` are the header (above the rule).
    head_rows: usize,
    in_head: bool,
}

impl MdTable {
    fn push(&mut self, s: &str, st: MdStyle) {
        if let Some(cell) = self.rows.last_mut().and_then(|r| r.last_mut()) {
            cell.push((s.to_string(), st));
        }
    }

    /// Set the table out as aligned columns: cells padded to their column's
    /// width, a dim │ between columns, a ─┼─ rule under the header.
    fn lay_out(&self, theme: &crate::theme::Theme, out: &mut String, spans: &mut Vec<(std::ops::Range<usize>, MdStyle)>) {
        use pulldown_cmark::Alignment;
        let width = |cell: &Vec<(String, MdStyle)>| cell.iter().map(|(s, _)| s.chars().count()).sum::<usize>();
        let cols = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..cols)
            .map(|c| self.rows.iter().filter_map(|r| r.get(c)).map(width).max().unwrap_or(0))
            .collect();
        let dim = MdStyle { color: Some(theme.text_dim), ..Default::default() };
        let mut push = |out: &mut String, s: &str, st: MdStyle| {
            if !s.is_empty() {
                let start = out.len();
                out.push_str(s);
                spans.push((start..out.len(), st));
            }
        };
        let empty = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            for (c, w) in widths.iter().enumerate() {
                if c > 0 {
                    push(out, " │ ", dim.clone());
                }
                let cell = row.get(c).unwrap_or(&empty);
                let pad = w - width(cell);
                let (before, after) = match self.aligns.get(c) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                out.push_str(&" ".repeat(before));
                for (s, st) in cell {
                    push(out, s, st.clone());
                }
                // No trailing padding on the last column.
                if c + 1 < widths.len() {
                    out.push_str(&" ".repeat(after));
                }
            }
            out.push('\n');
            if i + 1 == self.head_rows {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                push(out, &rule.join("─┼─"), dim.clone());
                out.push('\n');
            }
        }
    }
}

/// Clip whole-text `spans` to per-line [`AttrsList`]s over `text` (split on
/// '\n', matching cosmic-text), applying color / weight / style, and per-span
/// metrics for heading sizes (heading px = `base_px` × scale).
fn md_bake(family: &str, text: &str, spans: &[(std::ops::Range<usize>, MdStyle)], base_px: f32) -> Vec<AttrsList> {
    let ink = |c: u32| Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let base = Attrs::new().family(Family::Name(family));
    let mut line_start = Vec::new();
    let mut pos = 0usize;
    for line in text.split('\n') {
        line_start.push(pos);
        pos += line.len() + 1;
    }
    let n = line_start.len();
    let line_end = |i: usize| if i + 1 < n { line_start[i + 1] - 1 } else { text.len() };
    let mut lists: Vec<AttrsList> = (0..n).map(|_| AttrsList::new(&base)).collect();
    for (range, m) in spans {
        for i in 0..n {
            let (ls, le) = (line_start[i], line_end(i));
            let s = range.start.max(ls);
            let e = range.end.min(le);
            if e <= s {
                continue;
            }
            let mut a = base.clone();
            if let Some(c) = m.color {
                a = a.color(ink(c));
            }
            if m.bold {
                a = a.weight(Weight::BOLD);
            }
            if m.italic {
                a = a.style(Style::Italic);
            }
            if let Some(scale) = m.size_scale {
                let hp = base_px * scale;
                a = a.metrics(Metrics::new(hp, (hp * LEADING).round()));
            }
            lists[i].add_span((s - ls)..(e - ls), &a);
        }
    }
    lists
}

/// Lay out the active markdown buffer in its current mode and push text +
/// styled spans into the editor. `replace_text` true replaces the buffer text
/// and resets the cursor (open / mode toggle); false keeps the text and only
/// re-bakes spans (a metrics-only refresh after a UI-scale change).
fn md_render(pane: &mut Doc, fs: &mut FontSystem, family: &str, base_px: f32, replace_text: bool) {
    let (src, reading) = match &pane.md {
        Some(md) => (md.src.clone(), md.reading),
        None => return,
    };
    let theme = crate::theme::get();
    let (text, spans) =
        if reading { md_layout_reading(&theme, &src) } else { md_layout_source(&theme, &src) };
    if replace_text {
        pane.editor.with_buffer_mut(|buffer| {
            buffer.set_text(&text, &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
        });
        pane.editor.set_cursor(Cursor::default());
        pane.editor.set_selection(Selection::None);
    }
    let lists = md_bake(family, &text, &spans, base_px);
    pane.editor.with_buffer_mut(|buffer| {
        for (i, list) in lists.into_iter().enumerate() {
            if i < buffer.lines.len() {
                buffer.lines[i].set_attrs_list(list);
            }
        }
    });
    pane.editor.shape_as_needed(fs, false);
    if let Some(md) = &mut pane.md {
        md.applied_px = base_px;
    }
}

/// Turn the active buffer into a view-only MARKDOWN buffer from `src` text
/// (source-highlighted; `C-c C-v` toggles the WYSIWYG reading view). Edits are
/// refused and the lexer is skipped. Backs both file opens and generated
/// docs (e.g. the key-binding browser).
pub fn open_markdown_text(id: u32, src: &str) {
    let px = editor_px();
    let src = src.to_string();
    with_pane(id, |fs, pane, family| {
        pane.image = None;
        pane.video = None;
        pane.md = Some(Markdown { src, reading: false, applied_px: px });
        md_render(pane, fs, family, px, true);
        pane.dirty = false;
        pane.insert_run = false;
        pane.undo.clear();
        pane.redo.clear();
        pane.owners.clear();
        pane.line_sigs.clear();
    });
    crate::host::mark_dirty_ui();
}

/// Load `path` (a .md file) into the active buffer as a view-only markdown
/// buffer. Returns whether the file read.
pub fn open_markdown(id: u32, path: &str) -> bool {
    match std::fs::read_to_string(path) {
        Ok(src) => {
            open_markdown_text(id, &src);
            true
        }
        Err(_) => false,
    }
}

/// Toggle the active markdown buffer between source and reading view. Returns
/// 1 (now reading), 0 (now source), or -1 (not a markdown buffer).
pub fn toggle_markdown_reading(id: u32) -> i32 {
    let px = editor_px();
    let mut result = -1;
    with_pane(id, |fs, pane, family| {
        let Some(md) = &mut pane.md else { return };
        md.reading = !md.reading;
        result = md.reading as i32;
        md_render(pane, fs, family, px, true);
    });
    crate::host::mark_dirty_ui();
    result
}

/// Whether the active buffer is a view-only markdown buffer (help/keybindings/
/// a `.md` file) — the Scheme menu router shows the doc-viewer menu on these.
pub fn is_markdown(id: u32) -> bool {
    with_view(id, |pane| pane.doc().md.is_some())
}

/// Window coords → buffer coords for the mouse actions (the buffer's
/// origin sits at the pane rect + padding, right of the line numbers'
/// `gutter`).
fn to_buffer_xy(id: u32, gutter: f32, x: f64, y: f64) -> (i32, i32) {
    let pad = PAD * crate::host::ui_scale();
    let r = crate::host::rect(id).unwrap_or_default();
    (
        (x - r.x as f64 - pad as f64 - gutter as f64) as i32,
        (y - r.y as f64 - pad as f64) as i32,
    )
}

/// Draws cosmic-text's output under the modeler's own glyph policy
/// (`crate::text::mode`). cosmic-text decides placement while it lays out —
/// quarter-pixel bins baked into each glyph's cache key — so the A/B has to
/// happen here, at raster time: we rewrite the key, dropping hinting or
/// folding the bin back to a whole pixel, then hand it to the same
/// `SwashCache`. Rectangles (selection, cursor) pass straight through.
struct PaneRenderer<'a, F: FnMut(i32, i32, u32, u32, Color)> {
    font_system: &'a mut FontSystem,
    cache: &'a mut SwashCache,
    callback: F,
}

impl<F: FnMut(i32, i32, u32, u32, Color)> Renderer for PaneRenderer<'_, F> {
    fn rectangle(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        (self.callback)(x, y, w, h, color);
    }

    fn glyph(&mut self, glyph: PhysicalGlyph, color: Color) {
        let mut key = glyph.cache_key;
        let mut x = glyph.x;
        if !crate::text::hinting() {
            key.flags |= CacheKeyFlags::DISABLE_HINTING;
        }
        if !crate::text::subpixel(true) {
            // Round to the nearest whole pixel: the upper two bins belong to
            // the next one over.
            if matches!(key.x_bin, SubpixelBin::Two | SubpixelBin::Three) {
                x += 1;
            }
            key.x_bin = SubpixelBin::Zero;
        }
        let cb = &mut self.callback;
        self.cache.with_pixels(self.font_system, key, color, |dx, dy, pixel| {
            // Coverage arrives as the pixel's alpha; the weight curve (if any)
            // applies here, where both pipelines can share it.
            let (r, g, b, a) = pixel.as_rgba_tuple();
            let a = (crate::text::weigh(a as f32 / 255.0) * 255.0).round() as u8;
            cb(x + dx, glyph.y + dy, 1, 1, Color::rgba(r, g, b, a));
        });
    }
}

/// One named edit action from the Scheme layer. `arg` carries insert text;
/// `x`/`y` are WINDOW coords for the mouse actions. Returns Some(text) for
/// the actions that yield text (copy / cut / kill-eol — the Scheme side owns
/// the clipboard). Unknown actions are loud, not silent.
pub fn act(id: u32, kind: &str, arg: Option<&str>, x: f64, y: f64) -> Option<String> {
    // View-only buffers refuse every EDIT — but not every action. A rendered
    // help doc is still text you want to lift a form out of, so the mouse
    // family still places and extends a selection, `copy` still reads it back,
    // and `escape` still drops it. An image has no text behind it, so it stays
    // fully inert.
    // A movie takes the mouse and a few keys for itself: a click plays or
    // pauses, a drag scrubs, space, the arrows and vi's h/l step.
    if with_view(id, |pane| pane.doc().video.is_some()) {
        let width = with_view(id, |pane| pane.doc().video.as_ref().map_or(1.0, |v| v.pane_w));
        let control = match (kind, arg) {
            ("click" | "double-click" | "triple-click", _) => {
                with_pane(id, |_, pane, _| pane.video.as_mut().map(|v| v.press(x)));
                None
            }
            ("drag", _) => {
                with_pane(id, |_, pane, _| pane.video.as_mut().map(|v| v.drag(x, width)));
                crate::host::mark_dirty_ui();
                None
            }
            ("release", _) => {
                with_pane(id, |_, pane, _| pane.video.as_mut().map(|v| v.release()));
                crate::host::mark_dirty_ui();
                None
            }
            ("insert", Some(" ")) => Some("toggle"),
            ("insert", Some("l")) | ("right", _) => Some("step"),
            ("insert", Some("h")) | ("left", _) => Some("back"),
            ("insert", Some("0")) | ("home" | "buffer-start", _) => Some("restart"),
            ("end" | "buffer-end", _) => Some("end"),
            _ => None,
        };
        if let Some(c) = control {
            video_control(id, c);
        }
        return None;
    }
    let (image, md) = with_view(id, |pane| (pane.doc().image.is_some(), pane.doc().md.is_some()));
    let view_only = image || md;
    if image
        || (md
            && !matches!(
                kind,
                "click" | "double-click" | "triple-click" | "drag" | "copy" | "escape"
            ))
    {
        return None;
    }
    let out = with_pane(id, |fs, pane, _| {
        // Vi mode: the keyboard family routes through the modal parser
        // (typed chars are COMMANDS in normal mode). The mouse family,
        // clipboard verbs, and buffer-start/-end stay direct — they mean
        // the same thing in any mode.
        if vi_on() && !view_only {
            let vikey = match kind {
                "insert" => {
                    for ch in arg.unwrap_or_default().chars() {
                        vi_feed(fs, pane, ViKey::Char(ch));
                    }
                    return None;
                }
                "enter" => Some(ViKey::Enter),
                "backspace" => Some(ViKey::Backspace),
                "delete" => Some(ViKey::Delete),
                "left" => Some(ViKey::Left),
                "right" => Some(ViKey::Right),
                "up" => Some(ViKey::Up),
                "down" => Some(ViKey::Down),
                "home" => Some(ViKey::Home),
                "end" => Some(ViKey::End),
                "page-up" => Some(ViKey::PageUp),
                "page-down" => Some(ViKey::PageDown),
                "escape" => Some(ViKey::Escape),
                "indent" => Some(ViKey::Tab),
                _ => None,
            };
            if let Some(key) = vikey {
                // Ex-command submit ( ':' line + Enter ). modit 0.1.5 throws
                // the typed command away (its Command/Enter arm is a
                // //TODO: run command), so grab the text BEFORE feeding Enter
                // resets the parser, then hand it back to Scheme — which owns
                // file policy (*scene-file*, save-as, path prompts). The ':'
                // prefix marks it as an ex-command; "enter" returns nil for an
                // ordinary newline, so a non-nil result is unambiguous.
                if matches!(key, ViKey::Enter) {
                    let ex = if let ViMode::Command { value } = &pane.vi.mode {
                        Some(value.clone())
                    } else {
                        None
                    };
                    if let Some(cmd) = ex {
                        vi_feed(fs, pane, key);
                        return Some(format!(":{cmd}"));
                    }
                }
                vi_feed(fs, pane, key);
                return None;
            }
        }
        let motion = |pane: &mut Doc, fs: &mut FontSystem, m: Motion| {
            pane.close_insert_run();
            pane.editor.action(fs, Action::Motion(m));
            None
        };
        match kind {
            "insert" => {
                let s = arg.unwrap_or_default();
                if !pane.insert_run {
                    pane.editor.start_change();
                    pane.insert_run = true;
                }
                // Typing over a selection replaces it (every editor's rule).
                pane.editor.delete_selection();
                pane.editor.insert_string(s, None);
                // A large paste is its own undo step; letters keep coalescing.
                if s.len() > 1 {
                    pane.close_insert_run();
                }
                pane.dirty = true;
                None
            }
            "enter" => {
                // Newline + lisp indent: the innermost unclosed paren's
                // column + 2, computed at the (post-selection-delete) cursor.
                let lang = pane.lang;
                pane.discrete(|e| {
                    e.delete_selection();
                    let c = e.cursor();
                    let indent = e.with_buffer(|b| {
                        let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                        lang.indent(&lines, c.line, c.index)
                    });
                    e.insert_string(&format!("\n{:indent$}", ""), None);
                });
                None
            }
            "indent" => {
                // Tab: re-indent the CURRENT line to the computed column,
                // cursor riding along (Emacs's TAB, not an insert).
                let c = pane.editor.cursor();
                let lang = pane.lang;
                let (target, ws) = pane.editor.with_buffer(|b| {
                    let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                    let target = lang.indent(&lines, c.line, 0);
                    let ws = lines
                        .get(c.line)
                        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
                        .unwrap_or(0);
                    (target, ws)
                });
                if target != ws {
                    pane.discrete(|e| {
                        e.delete_range(Cursor::new(c.line, 0), Cursor::new(c.line, ws));
                        e.insert_at(Cursor::new(c.line, 0), &" ".repeat(target), None);
                    });
                }
                let index = if c.index <= ws { target } else { c.index + target - ws };
                pane.editor.set_cursor(Cursor::new(c.line, index));
                None
            }
            "backspace" => {
                pane.discrete(|e| e.action(fs, Action::Backspace));
                None
            }
            "delete" => {
                pane.discrete(|e| e.action(fs, Action::Delete));
                None
            }
            "left" => motion(pane, fs, Motion::Left),
            "right" => motion(pane, fs, Motion::Right),
            "up" => motion(pane, fs, Motion::Up),
            "down" => motion(pane, fs, Motion::Down),
            "home" => motion(pane, fs, Motion::Home),
            "end" => motion(pane, fs, Motion::End),
            "page-up" => motion(pane, fs, Motion::PageUp),
            "page-down" => motion(pane, fs, Motion::PageDown),
            "buffer-start" => motion(pane, fs, Motion::BufferStart),
            "buffer-end" => motion(pane, fs, Motion::BufferEnd),
            "escape" => {
                pane.close_insert_run();
                pane.editor.action(fs, Action::Escape);
                None
            }
            "click" | "double-click" | "triple-click" | "drag" => {
                pane.close_insert_run();
                let (bx, by) = to_buffer_xy(id, pane.gutter, x, y);
                let action = match kind {
                    "click" => Action::Click { x: bx, y: by },
                    "double-click" => Action::DoubleClick { x: bx, y: by },
                    "triple-click" => Action::TripleClick { x: bx, y: by },
                    _ => Action::Drag { x: bx, y: by },
                };
                pane.editor.action(fs, action);
                None
            }
            "paste" => {
                // The GUI clipboard paste (C-y / cmd-V) — a direct insert
                // in ANY mode; vi's own registers ride y/p instead.
                let s = arg.unwrap_or_default();
                if !s.is_empty() {
                    pane.discrete(|e| {
                        e.delete_selection();
                        e.insert_string(s, None);
                    });
                }
                None
            }
            "copy" => {
                pane.close_insert_run();
                selection_text(&pane.editor)
            }
            "cut" => {
                let copied = selection_text(&pane.editor);
                if copied.is_some() {
                    pane.discrete(|e| e.delete_selection());
                }
                copied
            }
            "kill-eol" => {
                // Emacs C-k: kill to end of line; at the line's end, kill
                // the newline (join with the next line).
                pane.close_insert_run();
                let cursor = pane.editor.cursor();
                let (line_len, has_next) = pane.editor.with_buffer(|b| {
                    (
                        b.lines.get(cursor.line).map(|l| l.text().len()).unwrap_or(0),
                        cursor.line + 1 < b.lines.len(),
                    )
                });
                let (end, killed) = if cursor.index < line_len {
                    let text = pane.editor.with_buffer(|b| {
                        b.lines[cursor.line].text()[cursor.index..].to_string()
                    });
                    (Cursor::new(cursor.line, line_len), text)
                } else if has_next {
                    (Cursor::new(cursor.line + 1, 0), "\n".to_string())
                } else {
                    return None;
                };
                pane.discrete(|e| e.delete_range(cursor, end));
                Some(killed)
            }
            // A press ending: text has nothing to do (a movie does, above).
            "release" => None,
            other => {
                eprintln!("[editor] unknown action '{other}'");
                None
            }
        }
    });
    crate::host::mark_dirty_ui();
    out
}

/// Undo the most recent change. Returns true if there was one.
pub fn undo(id: u32) -> bool {
    let did = with_pane(id, |fs, pane, _| {
        pane.close_insert_run();
        match pane.undo.pop() {
            Some(mut change) => {
                change.reverse();
                pane.editor.apply_change(&change);
                shift_owners(&mut pane.owners, &change);
                pane.editor.shape_as_needed(fs, false);
                pane.redo.push(change); // stored reversed; redo re-reverses
                pane.dirty = true;
                true
            }
            None => false,
        }
    });
    crate::host::mark_dirty_ui();
    did
}

/// Redo the most recently undone change. Returns true if there was one.
pub fn redo(id: u32) -> bool {
    let did = with_pane(id, |fs, pane, _| {
        pane.close_insert_run();
        match pane.redo.pop() {
            Some(mut change) => {
                change.reverse();
                pane.editor.apply_change(&change);
                shift_owners(&mut pane.owners, &change);
                pane.editor.shape_as_needed(fs, false);
                pane.undo.push(change);
                pane.dirty = true;
                true
            }
            None => false,
        }
    });
    crate::host::mark_dirty_ui();
    did
}

/// Swap the mono face (riding `set-text-font`, so the editor pane and the
/// transcript panes never diverge). Buffer text and the dirty flag survive;
/// undo history does not (Changes hold layout-tied cursors from the old
/// shaping) — a font swap is rare and deliberate.
pub fn set_font(font_data: Vec<u8>) {
    STATE.with(|s| {
        let mut slot = s.borrow_mut();
        // Preserve every pane's full buffer list (id, text, dirty) + which is
        // active, so a font swap doesn't collapse the S5 buffer stack.
        struct SavedDoc {
            id: u32,
            text: String,
            dirty: bool,
            // The item-1c registry rides the swap too — name, path,
            // auto-opened flag; MRU order rides on the pane.
            name: String,
            path: Option<String>,
            auto_opened: bool,
            tag: String,
            owners: Vec<Owner>,
        }
        struct SavedPane {
            active: usize,
            docs: Vec<SavedDoc>,
            mru: Vec<u32>,
        }
        let saved: Vec<(u32, SavedPane)> = match slot.take() {
            Some(old) => old
                .panes
                .iter()
                .map(|(&id, p)| {
                    let docs = p
                        .docs
                        .iter()
                        .map(|d| SavedDoc {
                            id: d.id,
                            text: buffer_text(&d.editor),
                            dirty: d.dirty,
                            name: d.name.clone(),
                            path: d.path.clone(),
                            auto_opened: d.auto_opened,
                            tag: d.tag.clone(),
                            owners: d.owners.clone(),
                        })
                        .collect();
                    (id, SavedPane { active: p.active, docs, mru: p.mru.clone() })
                })
                .collect(),
            None => Vec::new(),
        };
        let mut state = build_state(font_data);
        for (id, sp) in saved {
            let pane = state.panes.entry(id).or_insert_with(PaneState::new);
            pane.docs.clear();
            for sd in sp.docs {
                let mut doc = Doc::new(sd.id);
                doc.editor.with_buffer_mut(|buffer| {
                    buffer.set_text(&sd.text, &Attrs::new().family(Family::Name(&state.family)), Shaping::Advanced, None);
                });
                doc.editor.shape_as_needed(&mut state.font_system, false);
                doc.dirty = sd.dirty;
                doc.name = sd.name;
                doc.path = sd.path;
                doc.auto_opened = sd.auto_opened;
                doc.tag = sd.tag;
                doc.owners = sd.owners;
                pane.docs.push(doc);
            }
            pane.active = sp.active.min(pane.docs.len().saturating_sub(1));
            pane.mru = sp.mru;
        }
        *slot = Some(state);
    });
    crate::host::mark_dirty();
}

// ── Vi mode (user request 2026-09-08): modit, cosmic's own vi engine ───────
// The modal state machine is `modit::ViParser` — the exact parser COSMIC's
// ViEditor wraps — but we translate its events onto OUR editor ourselves
// (adapted from cosmic-text's vi.rs), so the S4 lexer/theme, the coalesced
// undo, the lisp indent, and the SLIME sends all keep working. ViEditor
// itself is welded to syntect highlighting; this keeps our stack.

thread_local! {
    /// The global vi toggle — ON by default (user decision 2026-09-08);
    /// `(set-editor-vi false)` opts out live, `(%editor-set-vi 0)` in
    /// ~/.config/modeler/init.scm opts out at boot (the prim registers
    /// before init runs; the wrapper verb loads later, in unify.scm).
    static VI: Cell<bool> = const { Cell::new(true) };
}

/// Line numbers down the editor's left side (Kevin, 2026-11-16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineNumbers {
    Off,
    /// Each line's own number.
    Absolute,
    /// Distance from the cursor's line, as vi's `relativenumber` — what a
    /// `5j` or `d3k` counts; the cursor's line keeps its own number.
    Relative,
}

thread_local! {
    /// ON by default; `(set-line-numbers …)` changes it live.
    static LINE_NUMBERS: Cell<LineNumbers> = const { Cell::new(LineNumbers::Absolute) };
}

pub fn line_numbers() -> LineNumbers {
    LINE_NUMBERS.with(|l| l.get())
}

pub fn set_line_numbers(mode: LineNumbers) {
    LINE_NUMBERS.with(|l| l.set(mode));
    crate::host::mark_dirty();
}

/// The line-number column's width for a buffer of `lines` lines: room for
/// the widest number (three digits at least, so it does not jump at line
/// 10 and 100) and a cell's gap before the text.
fn gutter_width(lines: usize, advance: f32) -> f32 {
    let digits = lines.max(1).to_string().len().max(3);
    ((digits + 1) as f32 * advance).round()
}

pub fn vi_on() -> bool {
    VI.with(|v| v.get())
}

pub fn set_vi(on: bool) {
    VI.with(|v| v.set(on));
    // Fresh parsers: a toggle mid-`d`-pending would strand the state.
    with_state(|state| {
        for pane in state.panes.values_mut() {
            for doc in &mut pane.docs {
                doc.vi = ViParser::new();
            }
        }
    });
    crate::host::mark_dirty_ui();
}

/// The mode badge text ("NORMAL", "INSERT", ":wq", "/query"…), or None
/// with vi off.
pub fn vi_mode_label(id: u32) -> Option<String> {
    if !vi_on() {
        return None;
    }
    Some(with_pane(id, |_, pane, _| mode_text(&pane.vi.mode)))
}

fn mode_text(mode: &ViMode) -> String {
    match mode {
        ViMode::Normal | ViMode::Extra(_) => "NORMAL".to_string(),
        ViMode::Insert => "INSERT".to_string(),
        ViMode::Replace => "REPLACE".to_string(),
        ViMode::Visual => "VISUAL".to_string(),
        ViMode::VisualLine => "V-LINE".to_string(),
        ViMode::Command { value } => format!(":{value}"),
        ViMode::Search { value, forwards } => {
            format!("{}{value}", if *forwards { '/' } else { '?' })
        }
    }
}

/// Kernel Escape hook: while vi is on, Escape is a MODE key, ALWAYS — back
/// to normal / clear the selection, a no-op when already plainly normal
/// (as in vi itself). It never releases typing focus: vi hands tap Escape
/// reflexively, and a silent release left every bare key dead until the
/// next click — then further Escapes fell through the ladder toward quit
/// (the "/ doesn't work" + "too easy to exit" traps, user reports
/// 2026-09-08). Leave the pane by hovering a viewport, M-e, or clicking
/// elsewhere. False (release as usual) only with vi off.
pub fn vi_escape(id: u32) -> bool {
    if !vi_on() {
        return false;
    }
    with_pane(id, |fs, pane, _| {
        if pane.vi.mode != ViMode::Normal
            || !matches!(pane.editor.selection(), Selection::None)
        {
            vi_feed(fs, pane, ViKey::Escape);
        }
    });
    crate::host::mark_dirty_ui();
    true
}

/// Bank the editor's open change (if any) as one undo step.
fn finish_into(
    editor: &mut Editor<'static>,
    undo: &mut Vec<Change>,
    redo: &mut Vec<Change>,
    dirty: &mut bool,
    owners: &mut [Owner],
) {
    if let Some(change) = editor.finish_change()
        && !change.items.is_empty() {
            shift_owners(owners, &change);
            *dirty = true;
            undo.push(change);
            if undo.len() > MAX_UNDO {
                undo.remove(0);
            }
            redo.clear();
        }
}

/// Whole-line search for `value` from the cursor (cosmic-text's vi.rs).
fn vi_search(editor: &mut Editor<'static>, value: &str, forwards: bool) -> bool {
    let mut cursor = editor.cursor();
    let start_line = cursor.line;
    if forwards {
        while cursor.line < editor.with_buffer(|b| b.lines.len()) {
            if let Some(index) = editor.with_buffer(|b| {
                b.lines[cursor.line]
                    .text()
                    .match_indices(value)
                    .find_map(|(i, _)| (cursor.line != start_line || i > cursor.index).then_some(i))
            }) {
                cursor.index = index;
                editor.set_cursor(cursor);
                return true;
            }
            cursor.line += 1;
        }
    } else {
        cursor.line += 1;
        while cursor.line > 0 {
            cursor.line -= 1;
            if let Some(index) = editor.with_buffer(|b| {
                b.lines[cursor.line]
                    .text()
                    .rmatch_indices(value)
                    .find_map(|(i, _)| (cursor.line != start_line || i < cursor.index).then_some(i))
            }) {
                cursor.index = index;
                editor.set_cursor(cursor);
                return true;
            }
        }
    }
    false
}

/// Select inside/around a delimiter pair (`ci(`, `da"` …) — cosmic's vi.rs.
fn select_in(editor: &mut Editor<'static>, start_c: char, end_c: char, include: bool) {
    let cursor = editor.cursor();
    let (start, end) = editor.with_buffer(|buffer| {
        let mut end = cursor;
        let mut starts = 0;
        let mut ends = 0;
        'find_end: loop {
            let text = buffer.lines[end.line].text();
            for (i, c) in text[end.index..].char_indices() {
                if c == end_c {
                    ends += 1;
                } else if c == start_c {
                    starts += 1;
                }
                if ends > starts {
                    end.index += if include { i + c.len_utf8() } else { i };
                    break 'find_end;
                }
            }
            if end.line + 1 < buffer.lines.len() {
                end.line += 1;
                end.index = 0;
            } else {
                break 'find_end;
            }
        }
        let mut start = cursor;
        'find_start: loop {
            let text = buffer.lines[start.line].text();
            for (i, c) in text[..start.index].char_indices().rev() {
                if c == start_c {
                    starts += 1;
                } else if c == end_c {
                    ends += 1;
                }
                if starts >= ends {
                    start.index = if include { i } else { i + c.len_utf8() };
                    break 'find_start;
                }
            }
            if start.line > 0 {
                start.line -= 1;
                start.index = buffer.lines[start.line].text().len();
            } else {
                break 'find_start;
            }
        }
        (start, end)
    });
    editor.set_selection(Selection::Normal(start));
    editor.set_cursor(end);
}

/// Feed one key through the vi parser, translating its events onto our
/// editor (adapted from cosmic-text's ViEditor::action). Owns the change
/// grouping: modit brackets compound edits with ChangeStart/ChangeFinish;
/// anything left open banks at the end.
fn vi_feed(fs: &mut FontSystem, pane: &mut Doc, key: ViKey) {
    pane.close_insert_run();
    let Doc { editor, vi, registers, search, undo, redo, dirty, owners, lang, .. } = pane;
    editor.start_change();
    let has_selection = !matches!(editor.selection(), Selection::None);
    vi.parse(key, has_selection, |event| {
        let action = match event {
            ViEvent::AutoIndent => {
                // Re-indent the current line to the lisp column (cosmic
                // leaves this TODO; our lexer knows better).
                let c = editor.cursor();
                let (target, ws) = editor.with_buffer(|b| {
                    let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                    let target = lang.indent(&lines, c.line, 0);
                    let ws = lines
                        .get(c.line)
                        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
                        .unwrap_or(0);
                    (target, ws)
                });
                if target != ws {
                    editor.delete_range(Cursor::new(c.line, 0), Cursor::new(c.line, ws));
                    editor.insert_at(Cursor::new(c.line, 0), &" ".repeat(target), None);
                }
                let index = if c.index <= ws { target } else { c.index + target - ws };
                editor.set_cursor(Cursor::new(c.line, index));
                return;
            }
            ViEvent::Backspace => Action::Backspace,
            ViEvent::BackspaceInLine => {
                if editor.cursor().index > 0 {
                    Action::Backspace
                } else {
                    return;
                }
            }
            ViEvent::ChangeStart => {
                editor.start_change();
                return;
            }
            ViEvent::ChangeFinish => {
                finish_into(editor, undo, redo, dirty, owners);
                return;
            }
            ViEvent::Delete => {
                // Line-wise deletes (dd / VISUAL-LINE d) take the newline
                // too — the line is REMOVED, not emptied. (Upstream cosmic
                // leaves the empty line: its Line-selection bounds stop at
                // the text end.)
                if matches!(editor.selection(), Selection::Line(_)) {
                    if let Some((start, end)) = editor.selection_bounds() {
                        let last = editor.with_buffer(|b| b.lines.len().saturating_sub(1));
                        let (s, e) = if end.line < last {
                            (start, Cursor::new(end.line + 1, 0))
                        } else if start.line > 0 {
                            let prev_len = editor
                                .with_buffer(|b| b.lines[start.line - 1].text().len());
                            (Cursor::new(start.line - 1, prev_len), end)
                        } else {
                            (start, end)
                        };
                        editor.delete_range(s, e);
                        editor.set_selection(Selection::None);
                        let line = s
                            .line
                            .min(editor.with_buffer(|b| b.lines.len().saturating_sub(1)));
                        editor.set_cursor(Cursor::new(line, 0));
                    }
                    return;
                }
                Action::Delete
            }
            ViEvent::DeleteInLine => {
                let c = editor.cursor();
                if c.index < editor.with_buffer(|b| b.lines[c.line].text().len()) {
                    Action::Delete
                } else {
                    return;
                }
            }
            ViEvent::Escape => Action::Escape,
            ViEvent::Insert(c) => Action::Insert(c),
            ViEvent::NewLine => {
                // Our lisp Enter: newline + innermost-paren indent.
                editor.delete_selection();
                let c = editor.cursor();
                let indent = editor.with_buffer(|b| {
                    let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                    lang.indent(&lines, c.line, c.index)
                });
                editor.insert_string(&format!("\n{:indent$}", ""), None);
                return;
            }
            ViEvent::Put { register, after } => {
                if let Some((selection, data)) = registers.get(&register) {
                    if editor.delete_selection() {
                        editor.insert_string(data, None);
                    } else {
                        match selection {
                            Selection::None | Selection::Normal(_) | Selection::Word(_) => {
                                let mut cursor = editor.cursor();
                                if after {
                                    editor.with_buffer(|b| {
                                        let text = b.lines[cursor.line].text();
                                        if let Some(c) = text[cursor.index..].chars().next() {
                                            cursor.index += c.len_utf8();
                                        }
                                    });
                                    editor.set_cursor(cursor);
                                }
                                editor.insert_at(cursor, data, None);
                            }
                            Selection::Line(_) => {
                                let mut cursor = editor.cursor();
                                if after {
                                    cursor.line += 1;
                                } else {
                                    cursor.line += 1;
                                    editor.set_cursor(cursor);
                                    cursor.line -= 1;
                                }
                                cursor.index = 0;
                                editor.insert_at(cursor, "\n", None);
                                editor.insert_at(cursor, data, None);
                                editor.shape_as_needed(fs, false);
                                let motion =
                                    if after { Motion::Down } else { Motion::Up };
                                editor.action(fs, Action::Motion(motion));
                            }
                        }
                    }
                    finish_into(editor, undo, redo, dirty, owners);
                }
                return;
            }
            ViEvent::Redraw => {
                editor.with_buffer_mut(|b| b.set_redraw(true));
                return;
            }
            ViEvent::SelectClear => {
                editor.set_selection(Selection::None);
                return;
            }
            ViEvent::SelectStart => {
                let c = editor.cursor();
                editor.set_selection(Selection::Normal(c));
                return;
            }
            ViEvent::SelectLineStart => {
                let c = editor.cursor();
                editor.set_selection(Selection::Line(c));
                return;
            }
            ViEvent::SelectTextObject(obj, include) => {
                use modit::TextObject as T;
                match obj {
                    T::AngleBrackets => select_in(editor, '<', '>', include),
                    T::CurlyBrackets => select_in(editor, '{', '}', include),
                    T::DoubleQuotes => select_in(editor, '"', '"', include),
                    T::Parentheses => select_in(editor, '(', ')', include),
                    T::SingleQuotes => select_in(editor, '\'', '\'', include),
                    T::SquareBrackets => select_in(editor, '[', ']', include),
                    T::Ticks => select_in(editor, '`', '`', include),
                    T::Search { forwards } => {
                        if let Some((value, _)) = &*search {
                            let value = value.clone();
                            if vi_search(editor, &value, forwards) {
                                let mut cursor = editor.cursor();
                                editor.set_selection(Selection::Normal(cursor));
                                cursor.index += value.len();
                                editor.set_cursor(cursor);
                            }
                        }
                    }
                    T::Word(word) => {
                        let mut cursor = editor.cursor();
                        let mut selection = editor.selection();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if let Some((i, w)) = WordIter::new(text, word)
                                .find(|&(i, w)| i <= cursor.index && i + w.len() > cursor.index)
                            {
                                cursor.index = i;
                                selection = Selection::Normal(cursor);
                                cursor.index += w.len();
                            }
                        });
                        editor.set_selection(selection);
                        editor.set_cursor(cursor);
                    }
                    other => {
                        eprintln!("[vi] text object {other:?} not implemented");
                    }
                }
                return;
            }
            ViEvent::SetSearch(value, forwards) => {
                *search = Some((value, forwards));
                return;
            }
            ViEvent::ShiftLeft => Action::Unindent,
            ViEvent::ShiftRight => Action::Indent,
            ViEvent::SwapCase => {
                eprintln!("[vi] swap-case not implemented");
                return;
            }
            ViEvent::Undo => {
                finish_into(editor, undo, redo, dirty, owners);
                if let Some(mut change) = undo.pop() {
                    change.reverse();
                    editor.apply_change(&change);
                    shift_owners(owners, &change);
                    editor.shape_as_needed(fs, false);
                    redo.push(change);
                    *dirty = true;
                }
                return;
            }
            ViEvent::Yank { register } => {
                if let Some(data) = selection_text(editor) {
                    registers.insert(register, (editor.selection(), data));
                }
                return;
            }
            ViEvent::Motion(motion) => {
                use modit::Motion as M;
                match motion {
                    M::Around | M::Inside | M::Line | M::Selection => return,
                    M::Down => Action::Motion(Motion::Down),
                    M::End => Action::Motion(Motion::End),
                    M::GotoLine(line) => Action::Motion(Motion::GotoLine(line.saturating_sub(1))),
                    M::GotoEof => Action::Motion(Motion::GotoLine(
                        editor.with_buffer(|b| b.lines.len().saturating_sub(1)),
                    )),
                    M::Home => Action::Motion(Motion::Home),
                    M::Left => Action::Motion(Motion::Left),
                    M::LeftInLine => {
                        if editor.cursor().index > 0 {
                            Action::Motion(Motion::Left)
                        } else {
                            return;
                        }
                    }
                    M::NextChar(find_c) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if cursor.index < text.len()
                                && let Some((i, _)) = text[cursor.index..]
                                    .char_indices()
                                    .find(|&(i, c)| i > 0 && c == find_c)
                                {
                                    cursor.index += i;
                                }
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::NextCharTill(find_c) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if cursor.index < text.len() {
                                let mut last_i = 0;
                                for (i, c) in text[cursor.index..].char_indices() {
                                    if last_i > 0 && c == find_c {
                                        cursor.index += last_i;
                                        break;
                                    }
                                    last_i = i;
                                }
                            }
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::NextSearch => {
                        if let Some((value, forwards)) = search.clone() {
                            vi_search(editor, &value, forwards);
                        }
                        return;
                    }
                    M::PreviousSearch => {
                        if let Some((value, forwards)) = search.clone() {
                            vi_search(editor, &value, !forwards);
                        }
                        return;
                    }
                    M::NextWordEnd(word) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| loop {
                            let text = b.lines[cursor.line].text();
                            if cursor.index < text.len() {
                                cursor.index = WordIter::new(text, word)
                                    .map(|(i, w)| {
                                        i + w.char_indices().last().map(|(i, _)| i).unwrap_or(0)
                                    })
                                    .find(|&i| i > cursor.index)
                                    .unwrap_or(text.len());
                                if cursor.index == text.len() {
                                    continue;
                                }
                            } else if cursor.line + 1 < b.lines.len() {
                                cursor.line += 1;
                                cursor.index = 0;
                                continue;
                            }
                            break;
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::NextWordStart(word) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| loop {
                            let text = b.lines[cursor.line].text();
                            if cursor.index < text.len() {
                                cursor.index = WordIter::new(text, word)
                                    .map(|(i, _)| i)
                                    .find(|&i| i > cursor.index)
                                    .unwrap_or(text.len());
                                if cursor.index == text.len() {
                                    continue;
                                }
                            } else if cursor.line + 1 < b.lines.len() {
                                cursor.line += 1;
                                cursor.index = 0;
                                continue;
                            }
                            break;
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::PreviousChar(find_c) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if cursor.index > 0
                                && let Some((i, _)) =
                                    text[..cursor.index].char_indices().rfind(|&(_, c)| c == find_c)
                                {
                                    cursor.index = i;
                                }
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::PreviousCharTill(find_c) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if cursor.index > 0
                                && let Some(i) = text[..cursor.index]
                                    .char_indices()
                                    .filter(|&(i, c)| c == find_c && i + c.len_utf8() < cursor.index).map(|(i, c)| i + c.len_utf8())
                                    .next_back()
                                {
                                    cursor.index = i;
                                }
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::PreviousWordEnd(word) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| loop {
                            let text = b.lines[cursor.line].text();
                            if cursor.index > 0 {
                                cursor.index = WordIter::new(text, word)
                                    .map(|(i, w)| {
                                        i + w.char_indices().last().map(|(i, _)| i).unwrap_or(0)
                                    })
                                    .filter(|&i| i < cursor.index)
                                    .last()
                                    .unwrap_or(0);
                                if cursor.index == 0 {
                                    continue;
                                }
                            } else if cursor.line > 0 {
                                cursor.line -= 1;
                                cursor.index = b.lines[cursor.line].text().len();
                                continue;
                            }
                            break;
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::PreviousWordStart(word) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| loop {
                            let text = b.lines[cursor.line].text();
                            if cursor.index > 0 {
                                cursor.index = WordIter::new(text, word)
                                    .map(|(i, _)| i)
                                    .filter(|&i| i < cursor.index)
                                    .last()
                                    .unwrap_or(0);
                                if cursor.index == 0 {
                                    continue;
                                }
                            } else if cursor.line > 0 {
                                cursor.line -= 1;
                                cursor.index = b.lines[cursor.line].text().len();
                                continue;
                            }
                            break;
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::Right => Action::Motion(Motion::Right),
                    M::RightInLine => {
                        let c = editor.cursor();
                        if c.index < editor.with_buffer(|b| b.lines[c.line].text().len()) {
                            Action::Motion(Motion::Right)
                        } else {
                            return;
                        }
                    }
                    M::ScreenHigh => {
                        match editor
                            .with_buffer(|b| b.layout_runs().next().map(|r| r.line_i))
                        {
                            Some(line) => Action::Motion(Motion::GotoLine(line)),
                            None => return,
                        }
                    }
                    M::ScreenLow => {
                        match editor
                            .with_buffer(|b| b.layout_runs().last().map(|r| r.line_i))
                        {
                            Some(line) => Action::Motion(Motion::GotoLine(line)),
                            None => return,
                        }
                    }
                    M::ScreenMiddle => {
                        let target = editor.with_buffer(|b| {
                            let mut runs = b.layout_runs();
                            match (runs.next(), runs.last()) {
                                (Some(first), Some(last)) => {
                                    Some((last.line_i + first.line_i) / 2)
                                }
                                _ => None,
                            }
                        });
                        match target {
                            Some(line) => Action::Motion(Motion::GotoLine(line)),
                            None => return,
                        }
                    }
                    M::SoftHome => Action::Motion(Motion::SoftHome),
                    M::PageDown => Action::Motion(Motion::PageDown),
                    M::PageUp => Action::Motion(Motion::PageUp),
                    M::Up => Action::Motion(Motion::Up),
                }
            }
        };
        editor.action(fs, action);
    });
    finish_into(editor, undo, redo, dirty, owners);
}

/// What a draw last applied to a pane: (theme signature, show-paren pair,
/// font family).
type Applied = (u64, Vec<(usize, usize)>, String);

/// **The scan is kept until the text changes.** The editor pane is redrawn
/// on every repaint of the window — a camera drag in a 3D pane included —
/// and a scan of the whole buffer costs about 2 ms at 8,600 lines. A
/// scan depends on the text alone, so it is redone only when a line differs
/// from the copy kept here (a comparison, tens of microseconds at that
/// size), and the per-line pass is skipped as well when the show-paren
/// pair, the theme and the font family are what it last applied.
struct LexCache {
    lines: Vec<String>,
    scan: language::Scan,
    /// What the language's reader says of the same text (E3f) — made with
    /// the scan, so it costs nothing until the text changes. Empty for a
    /// buffer that does not read as the language ([`Doc::reads`]).
    diags: Vec<language::Diagnostic>,
    reads: bool,
    /// (theme signature, show-paren pair, font family) last applied.
    applied: Option<Applied>,
}

/// Make `lex` the scan of `buffer`'s text — and the reader's diagnostics
/// of it — unless it already is. A buffer with a file whose text changed is
/// reported to the [`Watcher`].
fn fresh_scan(lex: &mut Option<LexCache>, lang: &'static dyn Language, reads: bool, buffer: &Buffer, path: Option<&str>) {
    let same = lex.as_ref().is_some_and(|c| {
        c.reads == reads
            && c.lines.len() == buffer.lines.len()
            && c.lines.iter().zip(&buffer.lines).all(|(a, b)| a == b.text())
    });
    if !same {
        let lines: Vec<&str> = buffer.lines.iter().map(|l| l.text()).collect();
        if let Some(path) = path {
            watchers().iter().for_each(|w| w.text_changed(path, lang.name(), &lines));
        }
        let scan = lang.scan(&lines);
        let diags = if reads { lang.diagnose(&lines, &scan) } else { Vec::new() };
        *lex = Some(LexCache {
            scan,
            diags,
            reads,
            lines: lines.iter().map(|l| l.to_string()).collect(),
            applied: None,
        });
    }
}

/// Re-lex the whole buffer and apply syntax spans (plus the show-paren
/// pair at the cursor, in accent, while `typing`). Only lines whose span
/// signature changed get a fresh `AttrsList` — everything else keeps its
/// shaping. Colors come from the theme each call, so a live re-theme
/// recolors on the next paint.
fn relex(pane: &mut Doc, family: &str, theme: &crate::theme::Theme, typing: bool) {
    let ink = |c: u32| Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let color_of = |t: Tok| {
        ink(match t {
            Tok::Comment => theme.code_comment,
            Tok::Str => theme.code_string,
            Tok::Keyword => theme.code_keyword,
            Tok::Number => theme.code_number,
            Tok::Call => theme.code_call,
            Tok::Paren => theme.code_paren,
        })
    };
    let cursor = pane.editor.cursor();
    let reads = pane.reads();
    let line_sigs = &mut pane.line_sigs;
    let lex = &mut pane.lex;
    let lang = pane.lang;
    let path = pane.path.as_deref();
    pane.editor.with_buffer_mut(|buffer| {
        // Pass 1: lex every line, collecting spans and delimiter positions —
        // unless the text is what the kept scan was made from.
        fresh_scan(lex, lang, reads, buffer, path);
        let cache = lex.as_mut().expect("just filled");
        let all_spans = &cache.scan.spans;
        let parens: &[ParenPos] = &cache.scan.parens;
        // Show-paren: the pair for a closer just before the cursor, else an
        // opener at it (Emacs's rule). Only while the cursor is visible.
        let mut matched: Vec<(usize, usize)> = Vec::new();
        if typing {
            let at = parens
                .iter()
                .position(|p| {
                    p.line == cursor.line
                        && p.byte + 1 == cursor.index
                        && matches!(p.ch, b')' | b']' | b'}')
                })
                .or_else(|| {
                    parens.iter().position(|p| {
                        p.line == cursor.line
                            && p.byte == cursor.index
                            && matches!(p.ch, b'(' | b'[' | b'{')
                    })
                });
            if let Some(a) = at
                && let Some(b) = find_partner(parens, a) {
                    matched.push((parens[a].line, parens[a].byte));
                    matched.push((parens[b].line, parens[b].byte));
                }
        }
        // Pass 2: apply changed lines only — none, when nothing that
        // colours them moved and no line has been re-set since.
        let theme_sig = {
            let mut h = DefaultHasher::new();
            (theme.code_comment, theme.code_string, theme.code_keyword, theme.code_number,
             theme.code_call, theme.code_paren, theme.text_accent)
                .hash(&mut h);
            h.finish()
        };
        let key = (theme_sig, matched.clone(), family.to_string());
        if line_sigs.len() == buffer.lines.len() && cache.applied.as_ref() == Some(&key) {
            return;
        }
        line_sigs.resize(buffer.lines.len(), 0);
        for (i, spans) in all_spans.iter().enumerate() {
            let marks: Vec<usize> =
                matched.iter().filter(|&&(l, _)| l == i).map(|&(_, b)| b).collect();
            let sig = {
                let mut h = DefaultHasher::new();
                (theme_sig, spans, &marks).hash(&mut h);
                h.finish() | 1 // 0 stays the "unlexed" sentinel
            };
            if line_sigs[i] == sig {
                continue;
            }
            let base = Attrs::new().family(Family::Name(family));
            let mut list = AttrsList::new(&base);
            for &(s, e, t) in spans {
                if e > s {
                    list.add_span(s..e, &base.clone().color(color_of(t)));
                }
            }
            for &b in &marks {
                list.add_span(b..b + 1, &base.clone().color(ink(theme.text_accent)));
            }
            buffer.lines[i].set_attrs_list(list);
            line_sigs[i] = sig;
        }
        cache.applied = Some(key);
    });
}

/// A cosmic-text color over our 0x00RRGGBB framebuffer: blend by its alpha
/// via the UI's one sRGB compositor.
fn blend_px(dst: &mut u32, color: Color) {
    let a = color.a();
    if a == 0 {
        return;
    }
    let ink = ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | color.b() as u32;
    *dst = crate::text::blend(*dst, ink, a as f32 / 255.0);
}

/// The modeline bar across a pane's bottom: `left` (buffer name) and an
/// optional right-aligned `right` (vi mode, or image dimensions).
#[allow(clippy::too_many_arguments)]
fn draw_modeline(
    fb: &mut Framebuffer,
    rect: Rect,
    scale: f32,
    pad: f32,
    ml_h: f32,
    ml_px: f32,
    ml_adv: f32,
    theme: &crate::theme::Theme,
    left: &str,
    right: Option<&str>,
) {
    let ml_top = (rect.y + rect.h).saturating_sub(ml_h as u32);
    let bar = Rect { x: rect.x, y: ml_top, w: rect.w, h: (ml_h as u32).max(1) };
    fb.fill_rect(bar.clamped(fb.width, fb.height), theme.status_bg);
    let ty = ml_top as f32 + 2.0 * scale;
    crate::text::draw_mono_line(fb, rect.x as f32 + pad, ty, ml_px, theme.status_ink, left);
    if let Some(r) = right {
        let w = r.chars().count() as f32 * ml_adv;
        let rx = (rect.x + rect.w) as f32 - pad - w;
        crate::text::draw_mono_line(fb, rx, ty, ml_px, theme.status_ink, r);
    }
}

/// Blit a view-only image buffer: the decoded PNG scaled to fit the pane's
/// interior (letterboxed, nearest-neighbor, alpha-composited over the bg),
/// with a modeline showing the name and pixel dimensions.
#[allow(clippy::too_many_arguments)]
fn draw_image(
    fb: &mut Framebuffer,
    rect: Rect,
    scale: f32,
    pad: f32,
    ml_h: f32,
    ml_px: f32,
    ml_adv: f32,
    theme: &crate::theme::Theme,
    img: &ImageBuf,
    name: &str,
) {
    if img.w > 0 && img.h > 0 {
        let avail_w = (rect.w as f32 - 2.0 * pad).max(1.0);
        let avail_h = (rect.h as f32 - 2.0 * pad - ml_h).max(1.0);
        let s = (avail_w / img.w as f32).min(avail_h / img.h as f32).max(0.0);
        let dw = (img.w as f32 * s).round() as i32;
        let dh = (img.h as f32 * s).round() as i32;
        let ox = rect.x as i32 + pad as i32 + ((avail_w - dw as f32) / 2.0) as i32;
        let oy = rect.y as i32 + pad as i32 + ((avail_h - dh as f32) / 2.0) as i32;
        let (fbw, fbh) = (fb.width as i32, fb.height as i32);
        for dy in 0..dh {
            let py = oy + dy;
            if py < 0 || py >= fbh {
                continue;
            }
            let sy = (((dy as f32 + 0.5) / s) as u32).min(img.h - 1);
            for dx in 0..dw {
                let px_ = ox + dx;
                if px_ < 0 || px_ >= fbw {
                    continue;
                }
                let sx = (((dx as f32 + 0.5) / s) as u32).min(img.w - 1);
                let si = ((sy * img.w + sx) as usize) * 4;
                let ink = ((img.rgba[si] as u32) << 16)
                    | ((img.rgba[si + 1] as u32) << 8)
                    | img.rgba[si + 2] as u32;
                let idx = (py as u32 * fb.width + px_ as u32) as usize;
                fb.pixels[idx] = crate::text::blend(fb.pixels[idx], ink, img.rgba[si + 3] as f32 / 255.0);
            }
        }
    }
    let dim = format!("{}x{}", img.w, img.h);
    draw_modeline(fb, rect, scale, pad, ml_h, ml_px, ml_adv, theme, name, Some(&dim));
}

/// A movie buffer: the shown frame scaled to fit (letterboxed, nearest
/// neighbour, as an image is), a thin bar under it — how far through, and
/// how much is decoded — and the modeline's play state, frame and time.
#[allow(clippy::too_many_arguments)]
fn draw_video(
    fb: &mut Framebuffer,
    rect: Rect,
    scale: f32,
    pad: f32,
    ml_h: f32,
    ml_px: f32,
    ml_adv: f32,
    theme: &crate::theme::Theme,
    v: &crate::video::Player,
    name: &str,
) {
    let avail_w = (rect.w as f32 - 2.0 * pad).max(1.0);
    let bar_h = (3.0 * scale).round().max(2.0);
    let avail_h = (rect.h as f32 - 2.0 * pad - ml_h - 2.0 * bar_h).max(1.0);
    v.with_frame(|w, h, rgb| {
        if w == 0 || h == 0 {
            return;
        }
        let s = (avail_w / w as f32).min(avail_h / h as f32).max(0.0);
        let dw = (w as f32 * s).round() as i32;
        let dh = (h as f32 * s).round() as i32;
        let ox = rect.x as i32 + pad as i32 + ((avail_w - dw as f32) / 2.0) as i32;
        let oy = rect.y as i32 + pad as i32 + ((avail_h - dh as f32) / 2.0) as i32;
        let (fbw, fbh) = (fb.width as i32, fb.height as i32);
        let xs: Vec<usize> = (0..dw).map(|dx| ((((dx as f32 + 0.5) / s) as u32).min(w - 1)) as usize * 3).collect();
        for dy in 0..dh {
            let py = oy + dy;
            if py < 0 || py >= fbh {
                continue;
            }
            let row = ((((dy as f32 + 0.5) / s) as u32).min(h - 1) * w) as usize * 3;
            let base = (py as u32 * fb.width) as usize;
            for (dx, sx) in xs.iter().enumerate() {
                let px_ = ox + dx as i32;
                if px_ < 0 || px_ >= fbw {
                    continue;
                }
                let si = row + sx;
                fb.pixels[base + px_ as usize] =
                    ((rgb[si] as u32) << 16) | ((rgb[si + 1] as u32) << 8) | rgb[si + 2] as u32;
            }
        }
    });
    if let Some(e) = v.error() {
        let ty = rect.y as f32 + pad;
        crate::text::draw_mono_line(fb, rect.x as f32 + pad, ty, ml_px, theme.text_dim, &format!("cannot play {name}: {e}"));
    }
    // The bar, just above the modeline: the track, what is decoded, and how
    // far the frame shown is through the clip.
    let ml_top = (rect.y + rect.h) as f32 - ml_h;
    let by = (ml_top - bar_h - 2.0 * scale).max(rect.y as f32);
    let (bx, bw) = (rect.x as f32 + pad, avail_w);
    let seg = |fb: &mut Framebuffer, frac: f32, ink: u32| {
        let w = (bw * frac.clamp(0.0, 1.0)).round() as u32;
        if w > 0 {
            fb.fill_rect(Rect { x: bx as u32, y: by as u32, w, h: bar_h as u32 }.clamped(fb.width, fb.height), ink);
        }
    };
    seg(fb, 1.0, theme.status_bg);
    seg(fb, v.buffered(), crate::text::blend(theme.status_bg, theme.text_dim, 0.5));
    seg(fb, v.progress(), theme.text_accent);
    draw_modeline(fb, rect, scale, pad, ml_h, ml_px, ml_adv, theme, name, Some(&v.modeline()));
}

/// A text buffer's modeline, left of the vi mode: its name (+ the dirty
/// star, + its place in the loaded scene), the language it is read as
/// ("Rust", or "Scheme (ray)" with its dialect; not for markdown, which is
/// read, not edited), how the server
/// watching it is ("rust-analyzer indexing (30%)"), how many warnings it
/// has (E8), and where text that will not read is (in another language,
/// where its first error is) — so a mark scrolled out of view is still found (M-g n
/// goes there). A form left open does not say: it is what typing looks
/// like.
fn modeline_left(doc: &Doc, diags: &[language::Diagnostic]) -> String {
    let mut left = if doc.dirty { format!("{} *", doc.name) } else { doc.name.clone() };
    if !doc.tag.is_empty() {
        left = format!("{left} · {}", doc.tag);
    }
    if doc.md.is_none() {
        left = match doc.dialect.as_str() {
            "" => format!("{left} · {}", doc.lang.title()),
            dialect => format!("{left} · {} ({dialect})", doc.lang.title()),
        };
    }
    if let Some(mode) = watched_mode(doc) {
        left = format!("{left} · {mode}");
    }
    // A Lisp's error is its reader's; another language's is its server's
    // (rust-analyzer's type error), which reads fine.
    if let Some(d) = diags.iter().find(|d| d.is_error()) {
        let what = if matches!(doc.lang.name(), "scheme" | "guile") { "unreadable" } else { "error" };
        left = format!("{left} · {what} at line {}", d.start.0 + 1);
    }
    // Warnings are counted, not placed: M-g n goes to them.
    match diags.iter().filter(|d| d.severity == language::Severity::Warning).count() {
        0 => {}
        1 => left = format!("{left} · 1 warning"),
        n => left = format!("{left} · {n} warnings"),
    }
    left
}

/// The modeline's height in physical pixels at `scale` — what a host's
/// divider grabs (the standalone's bar between the editor and its panel).
pub fn modeline_height(scale: f32) -> f32 {
    let ml_px = crate::textpane::font_px() * scale * 0.8;
    crate::text::mono_metrics(ml_px).1 + 4.0 * scale
}

/// Draw the pane: background, then the shaped buffer through cosmic-text's
/// rect callback (text + selection + cursor in one pass), clipped to the
/// pane. The cursor draws only while the pane holds TYPING focus — matching
/// the transcript panes' armed-cursor rule. A modeline bar across the bottom
/// shows the buffer name (with a dirty star) and the vi mode.
pub fn draw(id: u32, fb: &mut Framebuffer, rect: Rect, _focused: bool) {
    let rect = rect.clamped(fb.width, fb.height);
    if rect.w < 20 || rect.h < 20 {
        return;
    }
    let theme = crate::theme::get();
    fb.fill_rect(rect, theme.editor_bg);

    let scale = crate::host::ui_scale();
    let px = crate::textpane::font_px() * scale;
    let pad = PAD * scale;
    let typing = crate::host::typing() == Some(id);
    // Modeline: a status bar across the pane's bottom (buffer name + vi mode).
    // Text lays out above it, so reserve its height.
    // The modeline is the editor's face at 0.8 of its size: smaller than
    // the text it labels (Kevin, 2026-11-13 — the same font, not the same
    // size; a full-size bar read as one more line of text).
    let ml_px = px * 0.8;
    let ml_adv = crate::text::mono_metrics(ml_px).0;
    let ml_h = modeline_height(scale);

    with_state(|state| {
        let pane = state.panes.entry(id).or_insert_with(PaneState::new);
        let pane = pane.doc_mut(); // draw the active buffer (S5)
        // An image buffer blits its picture instead of the text editor.
        if let Some(img) = &pane.image {
            draw_image(fb, rect, scale, pad, ml_h, ml_px, ml_adv, &theme, img, &pane.name);
            return;
        }
        if let Some(v) = &mut pane.video {
            v.pane_w = rect.w as f64;
            draw_video(fb, rect, scale, pad, ml_h, ml_px, ml_adv, &theme, v, &pane.name);
            return;
        }
        let advance = crate::text::mono_metrics(px).0;
        let numbered = line_numbers() != LineNumbers::Off && pane.md.is_none();
        pane.gutter = if numbered {
            gutter_width(pane.editor.with_buffer(|b| b.lines.len()), advance)
        } else {
            0.0
        };
        let gutter = pane.gutter;
        pane.editor.with_buffer_mut(|buffer| {
            if pane.applied_px != px {
                buffer.set_metrics(Metrics::new(px, (px * LEADING).round()));
                pane.applied_px = px;
            }
            buffer.set_size(
                Some(rect.w as f32 - 2.0 * pad - gutter),
                Some(rect.h as f32 - 2.0 * pad - ml_h),
            );
        });
        // Markdown buffers carry their own baked spans — never lex them
        // (the lexer would repaint every line and erase the markdown styling).
        // Re-bake only when the UI scale changed the body px (headings track it).
        if let Some(md) = &pane.md {
            if md.applied_px != px {
                md_render(pane, &mut state.font_system, &state.family, px, false);
            }
        } else {
            relex(pane, &state.family, &theme, typing);
        }
        let editor = &mut pane.editor;
        editor.shape_as_needed(&mut state.font_system, false);

        let ink = |c: u32| Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
        let text_color = ink(theme.text_ink);
        // Vi non-insert modes draw a BLOCK cursor (below) instead of the
        // line caret cosmic renders.
        let vi_block =
            typing && vi_on() && !matches!(pane.vi.mode, ViMode::Insert | ViMode::Replace);
        // The vi label, captured before the &mut editor borrow below.
        let vi_label = if vi_on() && typing && pane.md.is_none() {
            Some(mode_text(&pane.vi.mode))
        } else {
            None
        };
        let cursor_color = if typing && !vi_block {
            ink(theme.text_accent)
        } else {
            Color::rgba(0, 0, 0, 0)
        };
        let selection_color = {
            let a = theme.text_accent;
            Color::rgba((a >> 16) as u8, (a >> 8) as u8, a as u8, 0x50)
        };
        let (ox, oy) = (rect.x as f32 + pad + gutter, rect.y as f32 + pad);
        // Clip to the padded interior so long lines can't bleed into a
        // neighboring pane — or, scrolled sideways, into the line numbers.
        let (cx0, cy0) = (rect.x as i32 + gutter as i32, rect.y as i32);
        let (cx1, cy1) =
            ((rect.x + rect.w) as i32, (rect.y + rect.h) as i32 - ml_h as i32);
        let fbw = fb.width as i32;
        let fbh = fb.height as i32;
        // Editor::draw would hard-code cosmic-text's own glyph policy;
        // render() takes ours (PaneRenderer). It expects layout to be current,
        // which shape_as_needed above and this shape_until_scroll guarantee —
        // the pair Editor::draw makes for itself.
        editor.with_buffer_mut(|b| b.shape_until_scroll(&mut state.font_system, false));
        let mut renderer = PaneRenderer {
            font_system: &mut state.font_system,
            cache: &mut state.cache,
            callback: |x: i32, y: i32, w: u32, h: u32, color: Color| {
                let x0 = (x + ox as i32).max(cx0).max(0);
                let y0 = (y + oy as i32).max(cy0).max(0);
                let x1 = (x + ox as i32 + w as i32).min(cx1).min(fbw);
                let y1 = (y + oy as i32 + h as i32).min(cy1).min(fbh);
                for py in y0..y1 {
                    let row = py as u32 * fb.width;
                    for px_ in x0..x1 {
                        blend_px(&mut fb.pixels[(row + px_ as u32) as usize], color);
                    }
                }
            },
        };
        editor.render(&mut renderer, text_color, cursor_color, selection_color, text_color);

        // The line numbers, right-aligned in the gutter on each line's first
        // row (a wrapped line's other rows have none), level with its text;
        // the cursor's line in the body ink, the rest dim.
        let cursor = pane.editor.cursor();
        if numbered {
            let ascent = crate::text::mono_ascent(px);
            let right = rect.x as f32 + pad + gutter - advance;
            let relative = line_numbers() == LineNumbers::Relative;
            pane.editor.with_buffer(|buffer| {
                // Where a baseline sits in its row, from a line with text:
                // cosmic-text gives an EMPTY line no ascent, and its number
                // drew a row-half too high.
                let drop = buffer
                    .layout_runs()
                    .find(|r| !r.glyphs.is_empty())
                    .map_or(ascent, |r| r.line_y - r.line_top);
                let mut last = None;
                for run in buffer.layout_runs() {
                    if last == Some(run.line_i) {
                        continue;
                    }
                    last = Some(run.line_i);
                    let y = oy + run.line_top + drop - ascent;
                    if y + run.line_height < cy0 as f32 || y > cy1 as f32 {
                        continue;
                    }
                    let here = run.line_i == cursor.line;
                    let n = if relative && !here { run.line_i.abs_diff(cursor.line) } else { run.line_i + 1 };
                    let label = n.to_string();
                    let x = right - label.len() as f32 * advance;
                    let ink = if here { theme.text_ink } else { theme.text_dim };
                    crate::text::draw_mono_line(fb, x, y, px, ink, &label);
                }
            });
        }

        // What the reader cannot read (E3f), underlined: in the error ink for
        // text that will not read, and faintly for a form left open — which
        // is also what every form is while it is being typed.
        let mut diags = match (&pane.md, &pane.lex) {
            (None, Some(c)) => c.diags.clone(),
            _ => Vec::new(),
        };
        diags.extend(watched_diagnostics(pane));
        if !diags.is_empty() {
            let thick = (2.0 * scale).round().max(1.0) as i32;
            let advance = crate::text::mono_metrics(px).0;
            pane.editor.with_buffer(|buffer| {
                for run in buffer.layout_runs() {
                    for d in diags.iter().filter(|d| (d.start.0..=d.end.0).contains(&run.line_i)) {
                        let mut spans: Vec<(f32, f32)> = run
                            .highlight(Cursor::new(d.start.0, d.start.1), Cursor::new(d.end.0, d.end.1))
                            .collect();
                        // An empty span — the reader stopped at a line's end
                        // — still marks one cell where it stopped.
                        if spans.is_empty() && d.start == d.end && run.line_i == d.start.0 {
                            let x = run
                                .glyphs
                                .iter()
                                .find(|g| g.start >= d.start.1)
                                .map_or(run.line_w, |g| g.x);
                            spans.push((x, advance));
                        }
                        let (ink, a) = match d.severity {
                            language::Severity::Error => (theme.text_error, 0xff),
                            language::Severity::Warning => (theme.text_warning, 0xff),
                            language::Severity::Open => (theme.text_error, 0x70),
                        };
                        let color = Color::rgba((ink >> 16) as u8, (ink >> 8) as u8, ink as u8, a);
                        let y = (run.line_top + run.line_height) as i32 - thick;
                        for (x, w) in spans {
                            let x0 = (x as i32 + ox as i32).max(cx0).max(0);
                            let x1 = ((x + w.max(1.0)) as i32 + ox as i32).min(cx1).min(fbw);
                            let y0 = (y + oy as i32).max(cy0).max(0);
                            let y1 = (y + thick + oy as i32).min(cy1).min(fbh);
                            for py in y0..y1 {
                                let row = py as u32 * fb.width;
                                for px_ in x0..x1 {
                                    blend_px(&mut fb.pixels[(row + px_ as u32) as usize], color);
                                }
                            }
                        }
                    }
                }
            });
        }

        // Vi block cursor: a translucent accent cell over the character —
        // the glyph stays readable through it.
        if vi_block
            && let Some((cx, cy)) = pane.editor.cursor_position() {
                let advance = crate::text::mono_metrics(px).0;
                let line_h = (px * LEADING).round();
                let x0 = (cx + ox as i32).max(cx0).max(0);
                let y0 = (cy + oy as i32).max(cy0).max(0);
                let x1 = (cx + ox as i32 + advance.ceil() as i32).min(cx1).min(fbw);
                let y1 = (cy + oy as i32 + line_h as i32).min(cy1).min(fbh);
                let block = Color::rgba(
                    (theme.text_accent >> 16) as u8,
                    (theme.text_accent >> 8) as u8,
                    theme.text_accent as u8,
                    0x6e,
                );
                for py in y0..y1 {
                    let row = py as u32 * fb.width;
                    for px_ in x0..x1 {
                        blend_px(&mut fb.pixels[(row + px_ as u32) as usize], block);
                    }
                }
            }

        // Modeline: what modeline_left says at the left; vi mode at the
        // right while the pane holds typing focus. Replaces the old floating
        // badge and corner dirty dot.
        let left = modeline_left(pane, &diags);
        // The cursor's line and column, 1-based as vi counts them, before
        // the vi mode.
        let col = pane.editor.with_buffer(|b| {
            b.lines.get(cursor.line).map_or(0, |l| l.text().get(..cursor.index).map_or(0, |t| t.chars().count()))
        });
        let at = format!("{}:{}", cursor.line + 1, col + 1);
        let right = match vi_label {
            Some(mode) => format!("{at}  {mode}"),
            None => at,
        };
        draw_modeline(fb, rect, scale, pad, ml_h, ml_px, ml_adv, &theme, &left, Some(&right));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_draws_shaped_glyphs() {
        let id = 950;
        set_text(id, "(add-cuboid :size [1 2 3])\n(render)");
        assert_eq!(text(id), "(add-cuboid :size [1 2 3])\n(render)");

        let mut fb = Framebuffer::new(300, 120);
        let rect = Rect { x: 0, y: 0, w: 300, h: 120 };
        let theme = crate::theme::get();
        draw(id, &mut fb, rect, true);
        let bg = fb.pixels.iter().filter(|&&p| p == theme.editor_bg).count();
        let lit = fb
            .pixels
            .iter()
            .filter(|&&p| p != theme.editor_bg && p != 0)
            .count();
        assert!(bg > 1000, "editor background fills the pane");
        assert!(lit > 100, "two lines of source should light real pixels, got {lit}");
        // Syntax colors reach the pixels: keyword violet and number orange
        // both appear at full glyph coverage somewhere in the sample.
        let hits = |ink: u32| fb.pixels.iter().filter(|&&p| p == ink).count();
        assert!(hits(theme.code_keyword) > 0, ":size should paint keyword-colored pixels");
        assert!(hits(theme.code_number) > 0, "digits should paint number-colored pixels");
        assert!(hits(theme.code_call) > 0, "add-cuboid should paint call-colored pixels");
    }

    #[test]
    fn scroll_moves_the_viewport() {
        let id = 951;
        let many: String = (0..200).map(|i| format!("line {i}\n")).collect();
        set_text(id, &many);
        let mut top = Framebuffer::new(200, 80);
        let rect = Rect { x: 0, y: 0, w: 200, h: 80 };
        draw(id, &mut top, rect, false);
        scroll(id, 50.0);
        let mut moved = Framebuffer::new(200, 80);
        draw(id, &mut moved, rect, false);
        assert_ne!(top.pixels, moved.pixels, "scrolling must change what draws");
    }

    #[test]
    fn set_font_preserves_buffers() {
        let id = 952;
        set_text(id, "(keep me)");
        let other = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/IBMPlexSans-Regular.ttf");
        set_font(std::fs::read(other).expect("vendored font"));
        assert_eq!(text(id), "(keep me)", "a font swap must not drop buffer text");
        // Restore the mono face for other tests on this thread.
        set_font(crate::text::mono_font_bytes());
    }

    /// An owner anchor follows its form through typing above it, a
    /// deletion, undo and redo — and a reload forgets it (E1).
    #[test]
    fn an_owner_follows_its_form_through_edits() {
        set_vi(false);
        let id = 954;
        set_text(id, "(a)\n(b é)\n(c)");
        own(id, 4, vec!["shape-1".into()]); // "(b é)"
        own(id, 10, vec!["shape-2".into()]); // "(c)"
        let at = |id| owners(id).into_iter().map(|(o, _)| o).collect::<Vec<_>>();

        // A whole new form typed above both: they move by its length.
        act(id, "buffer-start", None, 0.0, 0.0);
        for ch in ["(", "z", ")", "\n"] {
            act(id, "insert", Some(ch), 0.0, 0.0);
        }
        assert_eq!(&text(id)[..4], "(z)\n");
        assert_eq!(at(id), vec![8, 14], "both anchors moved past the typed form");

        // An edit INSIDE (b é) moves (c), not (b é)'s start; the é is one
        // character, so offsets stay in characters.
        replace_range(id, 2, 3, 2, 3, "ee"); // line 2: "(z)" and "(a)" are above it
        assert_eq!(at(id), vec![8, 16]);

        // Undo both edits: back where they began.
        assert!(undo(id));
        assert!(undo(id));
        assert_eq!(text(id), "(a)\n(b é)\n(c)");
        assert_eq!(at(id), vec![4, 10]);
        assert!(redo(id));
        assert_eq!(at(id), vec![8, 14]);

        // Taking a span takes only the anchors in it.
        assert_eq!(take_owned(id, 8, 14), vec!["shape-1".to_string()]);
        assert_eq!(at(id), vec![14]);

        // A reload is a fresh document.
        set_text(id, "(c)");
        assert!(owners(id).is_empty());
    }

    /// A movie buffer (Kevin, 2026-11-14): ffmpeg decodes a 12-frame clip,
    /// the clock plays it, a click pauses, a drag scrubs, the keys step, and
    /// the draw shows the frame. Skipped where ffmpeg is not installed.
    #[test]
    fn a_movie_plays_pauses_scrubs_and_steps() {
        let dir = std::env::temp_dir().join(format!("edit-movie-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=12:duration=1"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let id = 961;
        assert!(open_video(id, &clip.to_string_lossy()));
        let decoded = with_pane(id, |_, pane, _| {
            let v = pane.video.as_ref().unwrap();
            v.wait_decoded(std::time::Duration::from_secs(20)) && v.error().is_none()
        });
        assert!(decoded, "ffmpeg decoded it");
        let state = || video_control(id, "state").unwrap();
        assert_eq!(state(), (0, 12, true, true), "12 frames, playing from the first, looping");

        // The clock: a tick a few frames' time later has moved on.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (moved, next) = video_tick();
        assert!(moved && next.is_some(), "playing advances and asks to be woken");
        assert!(state().0 >= 2, "frame {}", state().0);

        // A click that does not move pauses; another plays.
        act(id, "click", None, 100.0, 50.0);
        act(id, "release", None, 100.0, 50.0);
        assert!(!state().2, "a click paused it");
        assert_eq!(video_tick(), (false, None), "and a paused movie asks for no wake-up");
        act(id, "click", None, 100.0, 50.0);
        act(id, "release", None, 100.0, 50.0);
        assert!(state().2, "and played it");
        video_control(id, "pause");

        // A drag scrubs: the pane's width spans the clip.
        with_pane(id, |_, pane, _| pane.video.as_mut().unwrap().pane_w = 120.0);
        video_control(id, "restart");
        video_control(id, "pause");
        act(id, "click", None, 10.0, 50.0);
        act(id, "drag", None, 70.0, 50.0);
        assert_eq!(state().0, 6, "half the width is half the clip");
        act(id, "drag", None, 500.0, 50.0);
        assert_eq!(state().0, 11, "and it stops at the last frame");
        act(id, "release", None, 500.0, 50.0);
        assert!(!state().2, "a scrub is not a click: still paused");

        // Keys: h/l and the arrows step, space plays, 0 restarts.
        act(id, "insert", Some("h"), 0.0, 0.0);
        act(id, "left", None, 0.0, 0.0);
        assert_eq!(state().0, 9);
        act(id, "right", None, 0.0, 0.0);
        assert_eq!(state().0, 10);
        act(id, "insert", Some(" "), 0.0, 0.0);
        assert!(state().2, "space plays");
        act(id, "insert", Some("0"), 0.0, 0.0);
        assert_eq!(state().0, 0, "0 restarts");
        assert!(text(id).is_empty(), "and nothing was typed into the buffer");

        // Once, not looping: it stops on its last frame.
        video_control(id, "loop");
        assert!(!state().3);

        // The draw shows the frame: testsrc is colour bars, not the pane's bg.
        let mut fb = Framebuffer::new(200, 160);
        draw(id, &mut fb, Rect { x: 0, y: 0, w: 200, h: 160 }, false);
        let bg = crate::theme::get().editor_bg;
        let painted = fb.pixels.iter().filter(|&&p| p != bg).count();
        assert!(painted > 5000, "the frame is drawn ({painted} px)");

        // A text load turns it back into a text buffer.
        set_text(id, "(text)");
        assert_eq!(video_control(id, "state"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_edits_dirty_and_undo_redo_round_trip() {
        set_vi(false); // plain-mode editing: vi defaults ON
        let id = 953;
        set_text(id, "hello");
        assert!(!dirty(id), "a fresh document starts clean");

        // Coalesced typing: one undo step for the whole run.
        act(id, "buffer-end", None, 0.0, 0.0);
        act(id, "insert", Some(" "), 0.0, 0.0);
        act(id, "insert", Some("w"), 0.0, 0.0);
        act(id, "insert", Some("o"), 0.0, 0.0);
        assert_eq!(text(id), "hello wo");
        assert!(dirty(id));

        assert!(undo(id), "undo takes the whole insert run");
        assert_eq!(text(id), "hello");
        assert!(redo(id));
        assert_eq!(text(id), "hello wo");

        // Discrete ops are their own steps.
        act(id, "backspace", None, 0.0, 0.0);
        assert_eq!(text(id), "hello w");
        assert!(undo(id));
        assert_eq!(text(id), "hello wo");

        mark_saved(id);
        assert!(!dirty(id));
        assert!(undo(id), "history survives a save");
        assert_eq!(text(id), "hello");
    }

    #[test]
    fn enter_kill_eol_and_selection_ops() {
        set_vi(false); // plain-mode editing: vi defaults ON
        let id = 954;
        set_text(id, "first line\nsecond");

        // C-k from mid-line kills to EOL and hands the text back.
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        let killed = act(id, "kill-eol", None, 0.0, 0.0);
        assert_eq!(killed.as_deref(), Some(" line"));
        assert_eq!(text(id), "first\nsecond");
        // A second C-k at EOL eats the newline.
        let killed = act(id, "kill-eol", None, 0.0, 0.0);
        assert_eq!(killed.as_deref(), Some("\n"));
        assert_eq!(text(id), "firstsecond");
        assert!(undo(id) && undo(id), "both kills undo");
        assert_eq!(text(id), "first line\nsecond");

        // Enter splits; undo joins.
        act(id, "buffer-start", None, 0.0, 0.0);
        act(id, "enter", None, 0.0, 0.0);
        assert_eq!(text(id), "\nfirst line\nsecond");
        assert!(undo(id));
        assert_eq!(text(id), "first line\nsecond");
    }

    #[test]
    fn markdown_source_view_styles_headings_emphasis_and_code() {
        let theme = crate::theme::get();
        let rgb = |c: u32| Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
        let src = "# Title\n\nsome **bold** and `code` here\n";
        let (text, spans) = md_layout_source(&theme, src);
        assert_eq!(text, src, "source view keeps the raw text");
        let lists = md_bake("Monaco", &text, &spans, 14.0);
        // One AttrsList per '\n'-split line (the frame cosmic-text lays out).
        assert_eq!(lists.len(), src.split('\n').count());
        // Heading line: an accent-colored, bold span.
        let accent = rgb(theme.text_accent);
        assert!(
            lists[0].spans().iter().any(|(_, a)| a.color_opt == Some(accent) && a.weight == Weight::BOLD),
            "heading is accent + bold"
        );
        // Body line: a bold (orange) span and an inline-code (green) span.
        let body = lists[2].spans();
        assert!(
            body.iter().any(|(_, a)| a.color_opt == Some(rgb(theme.code_number)) && a.weight == Weight::BOLD),
            "**bold** is orange + bold"
        );
        assert!(
            body.iter().any(|(_, a)| a.color_opt == Some(rgb(theme.code_string))),
            "`code` is green"
        );
    }

    /// A view-only doc is read-only, not inert: the mouse still selects and
    /// `copy` still reads the selection back (how a form gets out of (help)
    /// and into the REPL), while every edit stays refused.
    /// Copy, cut and a drag on a buffer that never had text: nothing to
    /// take, and no panic (cosmic-text indexes the first line, and a fresh
    /// buffer has none).
    /// The line numbers take a column on the left, and a click still lands
    /// on the character under it — with the column, without it, and with
    /// a buffer long enough to widen it.
    #[test]
    fn a_click_lands_on_its_character_beside_the_line_numbers() {
        let id = 994;
        let rect = Rect { x: 0, y: 0, w: 600, h: 300 };
        crate::host::testing::set_rect(id, rect);
        let mut fb = Framebuffer::new(600, 300);
        let advance = crate::text::mono_metrics(crate::textpane::font_px()).0;
        let mut click_at_char = |n: f32| {
            draw(id, &mut fb, rect, true);
            let gutter = with_pane(id, |_, pane, _| pane.gutter);
            act(id, "click", None, f64::from(PAD + gutter + (n + 0.3) * advance), f64::from(PAD + 4.0));
            (gutter, cursor(id))
        };
        set_text(id, "(abcdef ghij)");
        set_line_numbers(LineNumbers::Absolute);
        let (gutter, at) = click_at_char(4.0);
        assert_eq!(gutter, gutter_width(1, advance));
        assert!(gutter >= 4.0 * advance - 1.0, "three digits and a gap: {gutter}");
        assert_eq!(at, (0, 4), "the fifth character, past the numbers");
        set_line_numbers(LineNumbers::Off);
        let (gutter, at) = click_at_char(4.0);
        assert_eq!((gutter, at), (0.0, (0, 4)), "no column, the same character");
        set_text(id, &"(x)\n".repeat(1200));
        set_line_numbers(LineNumbers::Relative);
        let (gutter, _) = click_at_char(1.0);
        assert_eq!(gutter, gutter_width(1201, advance), "four digits widen it");
        assert_eq!(cursor(id), (0, 1));
        set_line_numbers(LineNumbers::Absolute);
    }

    #[test]
    fn copy_on_an_empty_buffer_copies_nothing() {
        let id = 993;
        crate::host::testing::set_rect(id, Rect { x: 0, y: 0, w: 300, h: 200 });
        let _ = text(id);
        buffer_new(id);
        for kind in ["click", "drag", "copy", "cut", "double-click", "copy", "triple-click", "cut"] {
            assert_eq!(act(id, kind, None, 10.0, 10.0), None, "{kind} on an empty buffer");
        }
        // And a click alone — a point — on real text copies nothing either.
        set_text(id, "(a b)");
        act(id, "click", None, 10.0, 10.0);
        assert_eq!(act(id, "copy", None, 0.0, 0.0), None);
    }

    #[test]
    fn view_only_buffers_select_and_copy_but_refuse_edits() {
        let id = 961;
        let src = "# Title\n\n(add-sphere :at [0 0 0] :radius 1)\n";
        open_markdown_text(id, src);
        assert!(is_markdown(id), "opened as a view-only markdown buffer");
        // Click at the top, drag past the end: a selection spanning the doc.
        act(id, "click", None, 0.0, 0.0);
        act(id, "drag", None, 1000.0, 1000.0);
        let copied = act(id, "copy", None, 0.0, 0.0).expect("copy yields the selection");
        assert!(copied.contains("add-sphere"), "selection carries the text: {copied:?}");
        // Escape drops the selection — the other half of read-only mousing.
        act(id, "escape", None, 0.0, 0.0);
        assert_eq!(act(id, "copy", None, 0.0, 0.0), None, "escape cleared the selection");
        // Every edit is still swallowed, and the doc stays clean.
        for (kind, arg) in
            [("insert", Some("XXX")), ("paste", Some("XXX")), ("enter", None), ("backspace", None)]
        {
            act(id, kind, arg, 0.0, 0.0);
        }
        assert_eq!(text(id), src, "a view-only buffer refuses every edit");
        assert!(!dirty(id), "and never goes dirty");
    }

    #[test]
    fn markdown_reading_view_strips_markup_and_enlarges_headings() {
        let theme = crate::theme::get();
        let src = "# Title\n\nsome **bold** text\n\n- one\n- two\n";
        let (text, spans) = md_layout_reading(&theme, src);
        // Markup hidden; heading text, bullets, and body survive.
        assert!(!text.contains('#'), "heading marker hidden: {text:?}");
        assert!(!text.contains('*'), "bold markers hidden: {text:?}");
        assert!(text.contains("Title") && text.contains("bold"));
        assert!(text.contains('•'), "list items get bullet glyphs: {text:?}");
        // The heading run carries an enlarged per-span font size.
        let lists = md_bake("Monaco", &text, &spans, 14.0);
        let big = lists[0].spans().iter().any(|(_, a)| {
            a.metrics_opt.map(|m| Metrics::from(m).font_size > 14.0).unwrap_or(false)
        });
        assert!(big, "heading is rendered larger than body");
    }

    /// A table reads as aligned columns — it ran together into one
    /// paragraph before (Kevin, 2026-11-14, on :help).
    #[test]
    fn markdown_reading_view_sets_tables_in_columns() {
        let theme = crate::theme::get();
        let src = "before\n\n| key | what |\n|---|--:|\n| `C-s` | search |\n| `M-%` | replace, one by one |\n\nafter\n";
        let (text, spans) = md_layout_reading(&theme, src);
        let lines: Vec<&str> = text.lines().collect();
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {text:?}"));
        let (head, rule, r1, r2) = (at("key"), at("┼"), at("C-s"), at("M-%"));
        assert_eq!((rule, r1, r2), (head + 1, head + 2, head + 3), "a row per line, the rule under the header:\n{text}");
        assert!(!text.contains('|'), "no pipes left: {text:?}");
        // The column rule falls at the same place on every row.
        let bar = |l: &str| l.chars().position(|c| c == '│' || c == '┼');
        assert!([head, rule, r1, r2].iter().all(|&i| bar(lines[i]) == bar(lines[head])), "columns line up:\n{text}");
        // The second column is right-aligned (--:): its rows end together.
        assert_eq!(lines[r1].chars().count(), lines[r2].chars().count(), "{text}");
        assert!(text.contains("before") && text.contains("after"));
        let _ = md_bake("Monaco", &text, &spans, 14.0);
    }

    #[test]
    fn markdown_reading_view_handles_multibyte_without_panicking() {
        // Byte-range clipping must land on char boundaries — a multibyte line
        // straddled by a fenced block is the trap.
        let theme = crate::theme::get();
        let src = "# café ☕\n\n```\nlet π = 3;\n```\n\n> quote ✓\n";
        for (text, spans) in [md_layout_source(&theme, src), md_layout_reading(&theme, src)] {
            let _ = md_bake("Monaco", &text, &spans, 14.0);
        }
    }

    #[test]
    fn enter_indents_and_tab_reindents() {
        set_vi(false); // plain-mode editing: vi defaults ON
        let id = 956;
        set_text(id, "(defn f []");
        act(id, "buffer-end", None, 0.0, 0.0);
        act(id, "enter", None, 0.0, 0.0);
        assert_eq!(text(id), "(defn f []\n  ", "enter lands at paren col + 2");
        act(id, "insert", Some("x"), 0.0, 0.0);
        // Undo peels the insert, then the newline+indent as one step.
        assert!(undo(id) && undo(id));
        assert_eq!(text(id), "(defn f []");

        // Tab re-indents a mis-indented line, cursor riding along.
        set_text(id, "(a\nb)");
        act(id, "down", None, 0.0, 0.0);
        act(id, "indent", None, 0.0, 0.0);
        assert_eq!(text(id), "(a\n  b)");
    }

    #[test]
    fn vi_modal_editing_round_trip() {
        set_vi(true);
        let id = 957;
        set_text(id, "alpha beta\ngamma");
        let key = |s: &str| act(id, "insert", Some(s), 0.0, 0.0);

        // Normal mode: x deletes the char under the cursor.
        key("x");
        assert_eq!(text(id), "lpha beta\ngamma");
        assert_eq!(vi_mode_label(id).as_deref(), Some("NORMAL"));

        // i enters insert; typed chars insert; Escape returns to normal.
        key("i");
        assert_eq!(vi_mode_label(id).as_deref(), Some("INSERT"));
        key("Q");
        assert_eq!(text(id), "Qlpha beta\ngamma");
        act(id, "escape", None, 0.0, 0.0);
        assert_eq!(vi_mode_label(id).as_deref(), Some("NORMAL"));

        // dd deletes the line; u undoes it.
        key("d");
        key("d");
        assert_eq!(text(id), "gamma");
        key("u");
        assert_eq!(text(id), "Qlpha beta\ngamma");

        // w word motion, then dw deletes a word (from the buffer top —
        // undo leaves the cursor where the change was applied).
        act(id, "buffer-start", None, 0.0, 0.0);
        key("w");
        assert!(cursor(id).1 > 0, "w moves off the line start");
        act(id, "buffer-start", None, 0.0, 0.0);
        key("d");
        key("w");
        assert_eq!(text(id), "beta\ngamma");

        // yy + p duplicates the line through the register.
        key("y");
        key("y");
        key("p");
        assert_eq!(text(id), "beta\nbeta\ngamma");

        // vi_escape consumes while non-normal, declines when already there.
        key("v");
        assert_eq!(vi_mode_label(id).as_deref(), Some("VISUAL"));
        assert!(vi_escape(id), "escape leaves visual mode");
        assert!(
            vi_escape(id),
            "escape is consumed even in plain normal - never a silent focus release"
        );

        // GUI paste stays a direct insert in normal mode.
        act(id, "paste", Some("Z"), 0.0, 0.0);
        assert!(text(id).contains('Z'));

        set_vi(false);
        key("x");
        assert!(text(id).contains('x'), "vi off: x is just a letter again");
    }

    #[test]
    fn vi_ex_command_is_captured_on_enter() {
        set_vi(true);
        let id = 967;
        set_text(id, "hello");
        // ':' enters Command mode; the rest accumulates on the ':' line.
        act(id, "insert", Some(":w foo.scm"), 0.0, 0.0);
        assert_eq!(vi_mode_label(id).as_deref(), Some(":w foo.scm"));
        // Enter hands the command back (modit itself discards it) and resets.
        let ex = act(id, "enter", None, 0.0, 0.0);
        assert_eq!(ex.as_deref(), Some(":w foo.scm"), "ex-command returned to the prelude");
        assert_eq!(text(id), "hello", "command mode never touches the buffer");
        assert_eq!(vi_mode_label(id).as_deref(), Some("NORMAL"), "back to normal after :");

        // A bare Enter in normal mode is not an ex-command — returns nil.
        assert_eq!(act(id, "enter", None, 0.0, 0.0), None);
        set_vi(false);
    }


    /// A [`Language`] that counts its scans — how the cache test sees
    /// whether a draw scanned.
    struct Counting;
    static SCANS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static COUNTING: Counting = Counting;
    impl Language for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn title(&self) -> &'static str {
            "Counting"
        }
        fn scan(&self, lines: &[&str]) -> language::Scan {
            SCANS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            language::SCHEME.scan(lines)
        }
        fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
            language::SCHEME.indent(lines, line, byte)
        }
        fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
            language::SCHEME.enclosing_head(lines, line, byte)
        }
    }

    /// A watcher that records what it is told and marks the first byte.
    struct Recording;
    thread_local! {
        static TOLD: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
        static LET_GO: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    static RECORDING: Recording = Recording;
    impl Watcher for Recording {
        fn text_changed(&self, path: &str, lang: &str, lines: &[&str]) {
            TOLD.with(|t| t.borrow_mut().push((path.into(), lang.into(), lines.join("\n"))));
        }
        fn diagnostics(&self, path: &str) -> Vec<language::Diagnostic> {
            vec![language::Diagnostic { start: (0, 0), end: (0, 1), message: format!("from {path}"), severity: language::Severity::Error }]
        }
        fn closed(&self, path: &str) {
            LET_GO.with(|t| t.borrow_mut().push(path.into()));
        }
        fn mode(&self, _path: &str, lang: &str) -> Option<String> {
            Some(format!("{lang}-server"))
        }
    }

    #[test]
    fn the_modeline_names_the_language_and_the_server() {
        set_vi(false);
        crate::host::testing::reset();
        clear_watchers();
        let id = 964;
        set_text(id, "(display 1)");
        let left = || with_pane(id, |_, pane, _| modeline_left(pane, &[]));
        assert_eq!(left(), "*scratch* · Scheme");
        set_buffer_dialect(id, active_buffer(id), "ray");
        assert_eq!(left(), "*scratch* · Scheme (ray)", "the dialect in brackets");
        set_buffer_dialect(id, active_buffer(id), "");
        set_language(id, "guile");
        assert_eq!(left(), "*scratch* · Guile");
        set_buffer_path(id, active_buffer(id), Some("/tmp/m.rs"));
        set_buffer_name(id, active_buffer(id), "m.rs");
        set_language(id, "rust");
        set_watcher(&RECORDING);
        assert_eq!(left(), "m.rs · Rust · rust-server", "then how its server is");
        let unreadable = [language::Diagnostic { start: (2, 0), end: (2, 1), message: String::new(), severity: language::Severity::Error }];
        assert_eq!(
            with_pane(id, |_, pane, _| modeline_left(pane, &unreadable)),
            "m.rs · Rust · rust-server · error at line 3",
            "a Rust error is a compiler's, not a reader's"
        );
        let warned = [
            language::Diagnostic { start: (0, 0), end: (0, 1), message: String::new(), severity: language::Severity::Warning },
            language::Diagnostic { start: (1, 0), end: (1, 1), message: String::new(), severity: language::Severity::Warning },
        ];
        assert_eq!(with_pane(id, |_, pane, _| modeline_left(pane, &warned)), "m.rs · Rust · rust-server · 2 warnings");
        clear_watchers();
    }

    #[test]
    fn the_watcher_hears_a_file_let_go_once_no_buffer_visits_it() {
        set_vi(false);
        crate::host::testing::reset();
        set_watcher(&RECORDING);
        let let_go = || LET_GO.with(|t| t.borrow().clone());
        let (a, b) = (962, 963);
        set_text(a, "");
        set_text(b, "");
        let one = buffer_new(a);
        set_buffer_path(a, one, Some("/tmp/one.rs"));
        let two = buffer_new(b);
        set_buffer_path(b, two, Some("/tmp/one.rs"));
        let mode = with_view(a, |p| watched_mode(&p.docs[p.index_of(one).unwrap()]));
        assert_eq!(mode.as_deref(), Some("scheme-server"), "the modeline's word, from the watcher");
        assert!(buffer_close(a, one));
        assert!(let_go().is_empty(), "the other pane still visits it");
        set_buffer_path(b, two, Some("/tmp/two.rs"));
        assert_eq!(let_go(), vec!["/tmp/one.rs"], "visiting another file lets the first go");
        assert!(buffer_close(b, two));
        assert_eq!(let_go(), vec!["/tmp/one.rs", "/tmp/two.rs"]);
        clear_watchers();
    }

    #[test]
    fn the_watcher_hears_a_file_buffer_change_and_its_marks_are_drawn() {
        set_vi(false);
        crate::host::testing::reset();
        let id = 961;
        let rect = Rect { x: 0, y: 0, w: 300, h: 100 };
        crate::host::testing::set_rect(id, rect);
        set_watcher(&RECORDING);
        set_text(id, "fn main() {}");
        let mut fb = Framebuffer::new(300, 100);
        draw(id, &mut fb, rect, true);
        assert!(TOLD.with(|t| t.borrow().is_empty()), "a buffer with no file is not watched");
        assert!(diagnostics(id).is_empty());
        set_buffer_path(id, active_buffer(id), Some("/tmp/w.rs"));
        set_language(id, "rust");
        draw(id, &mut fb, rect, true);
        draw(id, &mut fb, rect, true);
        assert_eq!(
            TOLD.with(|t| t.borrow().clone()),
            vec![("/tmp/w.rs".to_string(), "rust".to_string(), "fn main() {}".to_string())],
            "told once, until the text changes"
        );
        let d = diagnostics(id);
        assert_eq!((d.len(), d[0].message.as_str(), d[0].is_error()), (1, "from /tmp/w.rs", true));
        act(id, "insert", Some("x"), 0.0, 0.0);
        draw(id, &mut fb, rect, true);
        assert_eq!(TOLD.with(|t| t.borrow().len()), 2, "and again when it does");
        clear_watchers();
    }

    #[test]
    fn a_draw_scans_only_when_the_text_changed() {
        set_vi(false);
        crate::host::testing::reset();
        let id = 959;
        let rect = Rect { x: 0, y: 0, w: 300, h: 100 };
        crate::host::testing::set_rect(id, rect);
        crate::host::testing::set_typing(Some(id));
        set_text(id, "(a (b) c)");
        with_pane(id, |_, pane, _| pane.lang = &COUNTING);
        let scans = || SCANS.load(std::sync::atomic::Ordering::SeqCst);
        let pair = || with_pane(id, |_, p, _| p.lex.as_ref().and_then(|c| c.applied.as_ref()).map(|a| a.1.clone()));
        let mut fb = Framebuffer::new(300, 100);

        draw(id, &mut fb, rect, true);
        let after_first = scans();
        draw(id, &mut fb, rect, true);
        draw(id, &mut fb, rect, true);
        assert_eq!(scans(), after_first, "unchanged text is not scanned again");

        // The cursor moves, the text doesn't: no scan, but show-paren follows.
        act(id, "buffer-start", None, 0.0, 0.0);
        draw(id, &mut fb, rect, true);
        assert_eq!(pair(), Some(vec![(0, 0), (0, 8)]), "the outer pair, at the opener");
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        act(id, "right", None, 0.0, 0.0);
        draw(id, &mut fb, rect, true);
        assert_eq!(pair(), Some(vec![(0, 3), (0, 5)]), "the inner pair");
        assert_eq!(scans(), after_first, "moving the cursor scans nothing");

        act(id, "insert", Some("x"), 0.0, 0.0);
        draw(id, &mut fb, rect, true);
        assert_eq!(scans(), after_first + 1, "an edit scans once");
        crate::host::testing::set_typing(None);
    }

    #[test]
    fn set_text_same_content_still_highlights() {
        // Close/reopen refreshes the buffer with IDENTICAL text; the syntax
        // spans must re-apply even though the lex output is unchanged.
        let id = 958;
        set_vi(false);
        let src = "(add-sphere :radius 2)";
        let theme = crate::theme::get();
        let rect = Rect { x: 0, y: 0, w: 300, h: 80 };
        let hits = |fb: &Framebuffer, ink: u32| fb.pixels.iter().filter(|&&p| p == ink).count();

        set_text(id, src);
        let mut fb = Framebuffer::new(300, 80);
        draw(id, &mut fb, rect, true); // first paint applies + caches spans
        assert!(hits(&fb, theme.code_keyword) > 0);

        set_text(id, src); // the reopen refresh: same text, fresh default attrs
        let mut fb2 = Framebuffer::new(300, 80);
        draw(id, &mut fb2, rect, true);
        assert!(
            hits(&fb2, theme.code_keyword) > 0,
            "re-set of identical text must not lose highlighting"
        );
    }

    #[test]
    fn typing_replaces_a_mouse_selection() {
        set_vi(false); // plain-mode editing: vi defaults ON
        // The mouse actions need a pane rect for window→buffer coords.
        crate::host::testing::reset();
        let pid = 0;
        let rect = Rect { x: 0, y: 0, w: 300, h: 100 };
        crate::host::testing::set_rect(pid, rect);
        set_text(pid, "alpha beta");
        let mut fb = Framebuffer::new(300, 100);
        draw(pid, &mut fb, rect, true); // lay out so hit-testing works
        // Double-click the first word, then type over it.
        act(pid, "double-click", None, 20.0, 14.0);
        let copied = act(pid, "copy", None, 0.0, 0.0);
        assert_eq!(copied.as_deref(), Some("alpha"), "double-click selects the word");
        act(pid, "insert", Some("Z"), 0.0, 0.0);
        assert_eq!(text(pid), "Z beta", "typing replaces the selection");
    }
}

#[cfg(test)]
mod vi_search_probe {
    use super::*;
    #[test]
    fn enclosing_head_finds_the_form() {
        set_vi(false);
        let id = 991;
        set_text(id, "(defn f []\n  (add-sphere :at [1 2");
        act(id, "buffer-end", None, 0.0, 0.0);
        assert_eq!(enclosing_head(id).as_deref(), Some("add-sphere"),
                   "innermost ( wins over the [ vector");
        set_text(id, "(a \"( in-string\" ");
        act(id, "buffer-end", None, 0.0, 0.0);
        assert_eq!(enclosing_head(id).as_deref(), Some("a"), "string parens don't count");
        set_text(id, "top-level");
        act(id, "buffer-end", None, 0.0, 0.0);
        assert_eq!(enclosing_head(id), None);
    }

    #[test]
    fn slash_search_moves_the_cursor() {
        set_vi(true);
        let id = 990;
        set_text(id, "alpha\nbeta\nalpha end");
        for ch in "/beta".chars() {
            act(id, "insert", Some(&ch.to_string()), 0.0, 0.0);
        }
        assert_eq!(vi_mode_label(id).as_deref(), Some("/beta"), "search mode accumulates");
        act(id, "enter", None, 0.0, 0.0);
        assert_eq!(cursor(id), (1, 0), "search lands on beta");
        // n repeats forward, N reverses.
        for ch in "/alpha".chars() {
            act(id, "insert", Some(&ch.to_string()), 0.0, 0.0);
        }
        act(id, "enter", None, 0.0, 0.0);
        assert_eq!(cursor(id), (2, 0), "second search finds line 2's alpha");
        set_vi(false);
    }
}
