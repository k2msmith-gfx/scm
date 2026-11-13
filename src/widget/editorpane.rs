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

/// Inner padding between the pane border and the text — textpane's PAD.
const PAD: f32 = 8.0;

/// Line height as a multiple of the font px (cosmic-text lays out with real
/// leading, unlike the transcript's char-grid).
const LEADING: f32 = 1.4;

/// Undo depth per pane — plenty for a scene file, bounded for sanity.
const MAX_UNDO: usize = 500;

// ── Syntax (S4): a tiny Lisp lexer → per-span theme colors ─────────────────
// Lexing is mechanism (like glyph rendering), so it lives here; the COLORS
// are theme slots (code-comment/string/keyword/number/call/paren), so the
// look stays Scheme policy. It lexes Scheme's spelling: `;` line comments,
// `#| … |#` block comments (nesting), `#;` datum comments, `#t`/`#f`/`#\c`
// literals and `#(` vectors. Until 2026-11-02 it lexed Janet's — `#` as a
// line comment — so `(set-denoise #t)` coloured from the `#t` to the end of
// the line as a comment and lost its closing paren from matching. The whole
// buffer re-lexes per draw — microseconds at scene-file sizes — and a
// per-line signature limits reshaping to lines whose spans actually changed.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tok {
    Comment,
    Str,
    /// A `:keyword`.
    Keyword,
    Number,
    /// The head symbol of a form — `(add-sphere …)`'s add-sphere.
    Call,
    Paren,
}

/// Lexer state carried across lines (multi-line strings and comments).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum LexIn {
    #[default]
    Code,
    /// Inside a `"…"` string.
    Str,
    /// Inside a `#| … |#` block comment, this many deep (they nest).
    Block(u8),
}

/// Scan a block comment from `i` at nesting `depth` → (end, depth left).
/// Depth 0 means it closed at `end`.
fn scan_block(b: &[u8], mut i: usize, mut depth: u8) -> (usize, u8) {
    let n = b.len();
    while i < n {
        if b[i] == b'|' && i + 1 < n && b[i + 1] == b'#' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return (i, 0);
            }
        } else if b[i] == b'#' && i + 1 < n && b[i + 1] == b'|' {
            depth = depth.saturating_add(1);
            i += 2;
        } else {
            i += 1;
        }
    }
    (n, depth)
}

/// Lex one line given the carry-in state → (spans, carry-out). Code-context
/// delimiters land in `parens` as (byte, ch) for match-finding and indent.
fn lex_line(state: LexIn, line: &str, parens: &mut Vec<(usize, u8)>) -> (Vec<(usize, usize, Tok)>, LexIn) {
    let b = line.as_bytes();
    let n = b.len();
    let mut spans = Vec::new();
    let mut i = 0usize;
    match state {
        LexIn::Str => {
            let mut closed = false;
            while i < n {
                match b[i] {
                    b'\\' => i += 2,
                    b'"' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    _ => i += 1,
                }
            }
            spans.push((0, i.min(n), Tok::Str));
            if !closed {
                return (spans, LexIn::Str);
            }
        }
        LexIn::Block(depth) => {
            let (end, left) = scan_block(b, 0, depth);
            spans.push((0, end, Tok::Comment));
            if left > 0 {
                return (spans, LexIn::Block(left));
            }
            i = end;
        }
        LexIn::Code => {}
    }
    let is_delim = |c: u8| {
        matches!(c, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b' ' | b'\t' | b'"' | b';')
    };
    // True right after an open `(` — the next symbol is the form's head.
    let mut call_pending = false;
    while i < n {
        match b[i] {
            b';' => {
                spans.push((i, n, Tok::Comment));
                i = n;
            }
            b'#' if i + 1 < n && b[i + 1] == b'|' => {
                let start = i;
                let (end, left) = scan_block(b, i + 2, 1);
                spans.push((start, end, Tok::Comment));
                if left > 0 {
                    return (spans, LexIn::Block(left));
                }
                i = end;
            }
            b'#' if i + 1 < n && b[i + 1] == b';' => {
                // A datum comment: the mark is coloured, the datum it
                // comments out still lexes (its parens must still match).
                spans.push((i, i + 2, Tok::Comment));
                i += 2;
            }
            b'#' if i + 1 < n && b[i + 1] == b'(' => {
                // A vector literal: `#(` opens like `(`, but its first
                // element is data, not a call.
                parens.push((i + 1, b'('));
                spans.push((i, i + 2, Tok::Paren));
                call_pending = false;
                i += 2;
            }
            b'#' if i + 1 < n && b[i + 1] == b'\\' => {
                // A character: `#\(` is a paren CHARACTER, not a paren.
                let start = i;
                i = (i + 3).min(n);
                while i < n && !is_delim(b[i]) {
                    i += 1;
                }
                spans.push((start, i, Tok::Str));
                call_pending = false;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut closed = false;
                while i < n {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            closed = true;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                spans.push((start, i.min(n), Tok::Str));
                if !closed {
                    return (spans, LexIn::Str);
                }
                call_pending = false;
            }
            b'\'' | b'`' | b',' => {
                // Quote prefixes: the form after them is data.
                i += 1;
                call_pending = false;
            }
            c @ (b'(' | b'[' | b'{' | b')' | b']' | b'}') => {
                parens.push((i, c));
                spans.push((i, i + 1, Tok::Paren));
                call_pending = c == b'(';
                i += 1;
            }
            b' ' | b'\t' => i += 1,
            _ => {
                let start = i;
                while i < n && !is_delim(b[i]) {
                    i += 1;
                }
                let word = &line[start..i];
                let first = word.as_bytes()[0];
                let tok = if first == b':' {
                    Some(Tok::Keyword)
                } else if first == b'#'
                    || first.is_ascii_digit()
                    || (word.len() > 1
                        && matches!(first, b'-' | b'+' | b'.')
                        && word.as_bytes()[1].is_ascii_digit())
                {
                    // `#t`, `#f`, `#true`, `#e1.5`: literals, coloured as numbers.
                    Some(Tok::Number)
                } else if call_pending {
                    Some(Tok::Call)
                } else {
                    None
                };
                if let Some(t) = tok {
                    spans.push((start, i, t));
                }
                call_pending = false;
            }
        }
    }
    (spans, LexIn::Code)
}

/// A code-context delimiter's position, buffer-wide.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ParenPos {
    line: usize,
    byte: usize,
    ch: u8,
}

/// The partner of `parens[at]`, same-type depth counting (well-formed code).
fn find_partner(parens: &[ParenPos], at: usize) -> Option<usize> {
    let p = parens[at];
    let (open, close) = match p.ch {
        b'(' | b')' => (b'(', b')'),
        b'[' | b']' => (b'[', b']'),
        _ => (b'{', b'}'),
    };
    let mut depth = 0i32;
    if p.ch == open {
        for (j, q) in parens.iter().enumerate().skip(at + 1) {
            if q.ch == open {
                depth += 1;
            } else if q.ch == close {
                if depth == 0 {
                    return Some(j);
                }
                depth -= 1;
            }
        }
    } else {
        for j in (0..at).rev() {
            let q = parens[j];
            if q.ch == close {
                depth += 1;
            } else if q.ch == open {
                if depth == 0 {
                    return Some(j);
                }
                depth -= 1;
            }
        }
    }
    None
}

/// The column a NEW line at (`upto_line`, `upto_byte`) should indent to:
/// the innermost unclosed `(`'s column + 2 (the Lisp convention), +1 for
/// `[`/`{`, 0 at top level. Strings and comments don't count.
fn target_indent(lines: &[&str], upto_line: usize, upto_byte: usize) -> usize {
    let mut st = LexIn::Code;
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut scratch = Vec::new();
    for (ln, text) in lines.iter().enumerate() {
        if ln > upto_line {
            break;
        }
        let slice = if ln == upto_line { &text[..upto_byte.min(text.len())] } else { text };
        scratch.clear();
        let (_, next) = lex_line(st, slice, &mut scratch);
        for &(byte, ch) in &scratch {
            match ch {
                b'(' | b'[' | b'{' => stack.push((ch, byte)),
                _ => {
                    stack.pop();
                }
            }
        }
        st = next;
    }
    match stack.last() {
        Some(&(b'(', col)) => col + 2,
        Some(&(_, col)) => col + 1,
        None => 0,
    }
}

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
    /// Some when this is an IMAGE buffer (view-only): draw blits it instead
    /// of the text editor, and edits are refused.
    image: Option<ImageBuf>,
    /// Some for a MARKDOWN buffer (view-only): the styled spans are baked in
    /// and the lexer is skipped so it can't overwrite them. Edits
    /// refused. Carries the source so the reading-view toggle can re-render.
    md: Option<Markdown>,
    editor: Editor<'static>,
    /// The metrics px last applied to the buffer.
    applied_px: f32,
    /// Modified since the last set_text / mark_saved.
    dirty: bool,
    /// An insert-coalescing change is open (see the module doc).
    insert_run: bool,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Per-line signature of the last-applied syntax spans (0 = unlexed) —
    /// only lines whose spans changed pay the reshape.
    line_sigs: Vec<u64>,
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
            image: None,
            md: None,
            editor: Editor::new(Buffer::new_empty(Metrics::new(14.0, 14.0 * LEADING))),
            applied_px: 0.0,
            dirty: false,
            insert_run: false,
            undo: Vec::new(),
            redo: Vec::new(),
            line_sigs: Vec::new(),
            vi: ViParser::new(),
            registers: HashMap::new(),
            search: None,
            owners: Vec::new(),
        }
    }

    /// Close an open insert-coalescing run, banking it as one undo step.
    fn close_insert_run(&mut self) {
        if self.insert_run {
            self.insert_run = false;
            if let Some(change) = self.editor.finish_change() {
                if !change.items.is_empty() {
                    self.push_undo(change);
                }
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
        if let Some(change) = self.editor.finish_change() {
            if !change.items.is_empty() {
                self.push_undo(change);
            }
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
            (rect.x as f32 + pad + cx as f32, rect.y as f32 + pad + cy as f32, line_h)
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

/// The head symbol of the innermost UNCLOSED `(` at the cursor — eldoc's
/// anchor: in `(add-sphere :at [1 2|` that's "add-sphere". Lexes with the
/// syntax scanner, so parens inside strings/comments don't count.
/// Same-line heads only (the overwhelmingly common lisp shape).
pub fn enclosing_head(id: u32) -> Option<String> {
    with_pane(id, |_, pane, _| {
        let cursor = pane.editor.cursor();
        pane.editor.with_buffer(|buffer| {
            let mut st = LexIn::Code;
            let mut stack: Vec<(usize, usize, u8)> = Vec::new();
            let mut scratch = Vec::new();
            for (ln, line) in buffer.lines.iter().enumerate() {
                if ln > cursor.line {
                    break;
                }
                let text = line.text();
                let slice =
                    if ln == cursor.line { &text[..cursor.index.min(text.len())] } else { text };
                scratch.clear();
                let (_, next) = lex_line(st, slice, &mut scratch);
                for &(byte, ch) in &scratch {
                    match ch {
                        b'(' | b'[' | b'{' => stack.push((ln, byte, ch)),
                        _ => {
                            stack.pop();
                        }
                    }
                }
                st = next;
            }
            let (ln, byte) = stack
                .iter()
                .rev()
                .find(|&&(_, _, ch)| ch == b'(')
                .map(|&(l, b, _)| (l, b))?;
            let text = buffer.lines[ln].text();
            let rest = text[byte + 1..].trim_start();
            let head: String = rest
                .chars()
                .take_while(|&c| {
                    !matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '\t' | '"' | ';')
                })
                .collect();
            (!head.is_empty()).then_some(head)
        })
    })
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
    let ok = with_view(id, |pane| pane.buffer_close(buf));
    if ok {
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
    with_doc(id, buf, |d| d.path = path.map(str::to_string)).is_some()
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

    for (ev, _r) in Parser::new_ext(src, Options::empty()).into_offset_iter() {
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
/// origin sits at the pane rect + padding).
fn to_buffer_xy(id: u32, x: f64, y: f64) -> (i32, i32) {
    let pad = PAD * crate::host::ui_scale();
    let r = crate::host::rect(id).unwrap_or_default();
    (
        (x - r.x as f64 - pad as f64) as i32,
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
                pane.discrete(|e| {
                    e.delete_selection();
                    let c = e.cursor();
                    let indent = e.with_buffer(|b| {
                        let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                        target_indent(&lines, c.line, c.index)
                    });
                    e.insert_string(&format!("\n{:indent$}", ""), None);
                });
                None
            }
            "indent" => {
                // Tab: re-indent the CURRENT line to the computed column,
                // cursor riding along (Emacs's TAB, not an insert).
                let c = pane.editor.cursor();
                let (target, ws) = pane.editor.with_buffer(|b| {
                    let lines: Vec<&str> = b.lines.iter().map(|l| l.text()).collect();
                    let target = target_indent(&lines, c.line, 0);
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
                let (bx, by) = to_buffer_xy(id, x, y);
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
                pane.editor.copy_selection()
            }
            "cut" => {
                let copied = pane.editor.copy_selection();
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
    if let Some(change) = editor.finish_change() {
        if !change.items.is_empty() {
            shift_owners(owners, &change);
            *dirty = true;
            undo.push(change);
            if undo.len() > MAX_UNDO {
                undo.remove(0);
            }
            redo.clear();
        }
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
    let Doc { editor, vi, registers, search, undo, redo, dirty, owners, .. } = pane;
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
                    let target = target_indent(&lines, c.line, 0);
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
                    target_indent(&lines, c.line, c.index)
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
                if let Some(data) = editor.copy_selection() {
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
                            if cursor.index < text.len() {
                                if let Some((i, _)) = text[cursor.index..]
                                    .char_indices()
                                    .find(|&(i, c)| i > 0 && c == find_c)
                                {
                                    cursor.index += i;
                                }
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
                            if cursor.index > 0 {
                                if let Some((i, _)) =
                                    text[..cursor.index].char_indices().rfind(|&(_, c)| c == find_c)
                                {
                                    cursor.index = i;
                                }
                            }
                        });
                        editor.set_cursor(cursor);
                        return;
                    }
                    M::PreviousCharTill(find_c) => {
                        let mut cursor = editor.cursor();
                        editor.with_buffer(|b| {
                            let text = b.lines[cursor.line].text();
                            if cursor.index > 0 {
                                if let Some(i) = text[..cursor.index]
                                    .char_indices()
                                    .filter_map(|(i, c)| {
                                        (c == find_c && i + c.len_utf8() < cursor.index)
                                            .then(|| i + c.len_utf8())
                                    })
                                    .next_back()
                                {
                                    cursor.index = i;
                                }
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
    let line_sigs = &mut pane.line_sigs;
    pane.editor.with_buffer_mut(|buffer| {
        // Pass 1: lex every line, collecting spans and delimiter positions.
        let mut st = LexIn::Code;
        let mut all_spans: Vec<Vec<(usize, usize, Tok)>> = Vec::with_capacity(buffer.lines.len());
        let mut parens: Vec<ParenPos> = Vec::new();
        let mut scratch = Vec::new();
        for (i, line) in buffer.lines.iter().enumerate() {
            scratch.clear();
            let (spans, next) = lex_line(st, line.text(), &mut scratch);
            parens.extend(scratch.iter().map(|&(byte, ch)| ParenPos { line: i, byte, ch }));
            all_spans.push(spans);
            st = next;
        }
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
            if let Some(a) = at {
                if let Some(b) = find_partner(&parens, a) {
                    matched.push((parens[a].line, parens[a].byte));
                    matched.push((parens[b].line, parens[b].byte));
                }
            }
        }
        // Pass 2: apply changed lines only.
        line_sigs.resize(buffer.lines.len(), 0);
        let theme_sig = {
            let mut h = DefaultHasher::new();
            (theme.code_comment, theme.code_string, theme.code_keyword, theme.code_number,
             theme.code_call, theme.code_paren, theme.text_accent)
                .hash(&mut h);
            h.finish()
        };
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
    let ml_px = px * 0.8;
    let (ml_adv, ml_row) = crate::text::mono_metrics(ml_px);
    let ml_h = ml_row + 4.0 * scale;

    with_state(|state| {
        let pane = state.panes.entry(id).or_insert_with(PaneState::new);
        let pane = pane.doc_mut(); // draw the active buffer (S5)
        // An image buffer blits its picture instead of the text editor.
        if let Some(img) = &pane.image {
            draw_image(fb, rect, scale, pad, ml_h, ml_px, ml_adv, &theme, img, &pane.name);
            return;
        }
        pane.editor.with_buffer_mut(|buffer| {
            if pane.applied_px != px {
                buffer.set_metrics(Metrics::new(px, (px * LEADING).round()));
                pane.applied_px = px;
            }
            buffer.set_size(
                Some(rect.w as f32 - 2.0 * pad),
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
        // Modeline contents, captured before the &mut editor borrow below.
        let buf_name = pane.name.clone();
        let buf_dirty = pane.dirty;
        let buf_tag = pane.tag.clone();
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
        let (ox, oy) = (rect.x as f32 + pad, rect.y as f32 + pad);
        // Clip to the padded interior so long lines can't bleed into a
        // neighboring pane.
        let (cx0, cy0) = (rect.x as i32, rect.y as i32);
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

        // Vi block cursor: a translucent accent cell over the character —
        // the glyph stays readable through it.
        if vi_block {
            if let Some((cx, cy)) = pane.editor.cursor_position() {
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
        }

        // Modeline: buffer name (+ dirty star, + its place in the loaded
        // scene) at the left; vi mode at the right while the pane holds
        // typing focus. Replaces the old floating badge and corner dirty dot.
        let left = if buf_dirty { format!("{buf_name} *") } else { buf_name };
        let left = if buf_tag.is_empty() { left } else { format!("{left} · {buf_tag}") };
        draw_modeline(fb, rect, scale, pad, ml_h, ml_px, ml_adv, &theme, &left, vi_label.as_deref());
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
        let dejavu = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/DejaVuSans.ttf");
        set_font(std::fs::read(dejavu).expect("vendored font"));
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
    fn lexer_classifies_scheme_tokens() {
        let mut parens = Vec::new();
        let (spans, out) =
            lex_line(LexIn::Code, "(add-sphere :at [1 2] 0.5) ; hi", &mut parens);
        assert_eq!(out, LexIn::Code);
        let find = |spans: &[(usize, usize, Tok)], t: Tok| spans.iter().filter(|s| s.2 == t).count();
        assert_eq!(find(&spans, Tok::Call), 1, "head symbol is a call");
        assert_eq!(find(&spans, Tok::Keyword), 1, ":at is a keyword");
        assert_eq!(find(&spans, Tok::Number), 3, "1 2 0.5 are numbers");
        assert_eq!(find(&spans, Tok::Comment), 1);
        assert_eq!(find(&spans, Tok::Paren), 4);
        assert_eq!(parens.len(), 4);

        // `#t` is a literal, not Janet's comment: the closing paren still
        // counts, and only the `;` tail is a comment.
        let mut p = Vec::new();
        let line = "(set-denoise #t) ; on";
        let (spans, _) = lex_line(LexIn::Code, line, &mut p);
        assert_eq!(p.len(), 2, "both parens count");
        let comments: Vec<_> = spans.iter().filter(|s| s.2 == Tok::Comment).collect();
        assert_eq!(comments.len(), 1);
        assert_eq!(&line[comments[0].0..comments[0].1], "; on");
        assert_eq!(find(&spans, Tok::Number), 1, "#t is a literal");

        // A vector opens a paren; a character `#\(` does not.
        let mut p = Vec::new();
        let (_, _) = lex_line(LexIn::Code, "(list #(1 2) #\\( 3)", &mut p);
        assert_eq!(p.len(), 4, "( #( ) ) — the #\\( character is not a paren");

        // Multi-line string state carries; code resumes after the closer.
        let (_, st) = lex_line(LexIn::Code, "(print \"open", &mut Vec::new());
        assert_eq!(st, LexIn::Str);
        let mut p2 = Vec::new();
        let (spans, st) = lex_line(st, "still\" (f)", &mut p2);
        assert_eq!(st, LexIn::Code);
        assert_eq!(spans[0], (0, 6, Tok::Str));
        assert_eq!(p2.len(), 2, "parens after the string close count again");

        // Block comments carry across lines and nest; parens inside don't count.
        let mut p3 = Vec::new();
        let (_, st) = lex_line(LexIn::Code, "#| a ) #| b", &mut p3);
        assert_eq!(st, LexIn::Block(2));
        let (_, st) = lex_line(st, "c |# d |# (g)", &mut p3);
        assert_eq!(st, LexIn::Code);
        assert_eq!(p3.len(), 2, "only (g)'s parens count");
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
    fn indent_follows_the_innermost_open_paren() {
        let lines = ["(define (f)", "  (let ((x 1"];
        // After line 1: innermost open is the second `(` of `((` at byte 8
        // → a `(` indents its body two past itself = 10.
        assert_eq!(target_indent(&lines, 1, lines[1].len()), 10);
        // A `[` aligns one past itself.
        assert_eq!(target_indent(&["(f [x"], 0, 5), 4);
        // After just line 0: `(f)` closed; innermost is `(` at 0 → 2.
        assert_eq!(target_indent(&lines[..1], 0, lines[0].len()), 2);
        // Parens inside strings/comments don't count.
        let tricky = ["(a \"( [\" ; ("];
        assert_eq!(target_indent(&tricky, 0, tricky[0].len()), 2);
        assert_eq!(target_indent(&["(done)"], 0, 6), 0);
    }

    #[test]
    fn paren_partner_matching_skips_across_lines() {
        let text = ["(a [b", "   c] d)"];
        let mut parens = Vec::new();
        let mut st = LexIn::Code;
        for (i, l) in text.iter().enumerate() {
            let mut p = Vec::new();
            let (_, next) = lex_line(st, l, &mut p);
            parens.extend(p.iter().map(|&(byte, ch)| ParenPos { line: i, byte, ch }));
            st = next;
        }
        // parens: ( [ ] )
        assert_eq!(find_partner(&parens, 0), Some(3), "( matches the final )");
        assert_eq!(find_partner(&parens, 1), Some(2), "[ matches ]");
        assert_eq!(find_partner(&parens, 3), Some(0), ") matches back to (");
        let unbalanced = [ParenPos { line: 0, byte: 0, ch: b')' }];
        assert_eq!(find_partner(&unbalanced, 0), None);
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
