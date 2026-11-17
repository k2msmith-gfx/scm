//! The text-pane component (docs/modeler-plan.md M4.5): a scrollable,
//! wrapped, monospace transcript with an optional input line at the bottom —
//! the one surface behind the REPL console, the agent panel (M5.5) and the
//! outliner (M6), built once so they never grow divergent renderers.
//!
//! The Emacs split, applied to text: the kernel stores lines and draws them
//! (O(pixels) — wrap, scroll, glyphs); *what* the lines say and *how* the
//! input line edits are prelude policy over `%pane-*` primitives. The store is
//! keyed by pane id in its own thread_local, so text content rides beside
//! the pane registry without touching it.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use crate::framebuffer::{Framebuffer, Rect};
use crate::text;

/// Scrollback cap, in logical (unwrapped) lines. Old lines fall off the
/// front — a transcript, not an editor buffer.
const MAX_LINES: usize = 2000;

/// The monospace size every text pane draws at — settable from the prelude
/// (`set-text-size`, C-+/C-- bump it live), clamped to sanity.
const DEFAULT_FONT_PX: f32 = 13.0;
const MIN_FONT_PX: f32 = 8.0;
const MAX_FONT_PX: f32 = 32.0;

thread_local! {
    static FONT_PX: std::cell::Cell<f32> = const { std::cell::Cell::new(DEFAULT_FONT_PX) };
}

pub fn font_px() -> f32 {
    FONT_PX.with(|s| s.get())
}

/// Set the text-pane font size (0 or out-of-range low just reads).
/// Returns the size in effect.
pub fn set_font_px(px: f32) -> f32 {
    if px >= MIN_FONT_PX {
        FONT_PX.with(|s| s.set(px.min(MAX_FONT_PX)));
        crate::host::mark_dirty();
    }
    font_px()
}

/// Inner padding between the pane border and the character grid.
pub const PAD: f32 = 8.0;

/// A proportional list pane's text size, as a share of the mono size.
const PROPORTIONAL_SHARE: f32 = 1.08;

/// A line's ink, by role. The theme maps each to a color at draw time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Normal,
    /// De-emphasized: eval results, banners.
    Dim,
    /// Emphasized: echoed input, prompts.
    Accent,
    Error,
    /// Lisp source: drawn token by token in the editor's code palette
    /// (parens dim, keywords, strings, numbers, the head symbol of a form)
    /// — the agent pane's replies (Kevin, 2026-09-22: "format lisp output
    /// so the code looks pretty").
    Code,
    /// A line of a CODE BLOCK — an example the agent printed — on a band
    /// of the editor's own ground, so it reads apart from the prose round
    /// it (Kevin, 2026-11-17). Plain ink: the block may be any language.
    Block,
    /// The same, drawn token by token as `Code` is: a block of Lisp.
    BlockCode,
    /// A code block's header: its language, and the words that act on it
    /// when clicked, each between two [`ACTION_MARK`]s and drawn in accent.
    BlockHead,
}

/// Brackets a clickable word in a `BlockHead` row. A private-use code point
/// no face draws, so its cell is a space.
pub const ACTION_MARK: char = '\u{E010}';

impl Style {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "dim" => Some(Self::Dim),
            "accent" => Some(Self::Accent),
            "error" => Some(Self::Error),
            "code" => Some(Self::Code),
            "block" => Some(Self::Block),
            "block-code" => Some(Self::BlockCode),
            "block-head" => Some(Self::BlockHead),
            _ => None,
        }
    }

    /// The name [`Style::parse`] reads back.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Dim => "dim",
            Self::Accent => "accent",
            Self::Error => "error",
            Self::Code => "code",
            Self::Block => "block",
            Self::BlockCode => "block-code",
            Self::BlockHead => "block-head",
        }
    }

    /// The ground a row of this style is drawn on, when not the pane's own.
    fn band(self, theme: &crate::theme::Theme) -> Option<u32> {
        match self {
            Self::Block | Self::BlockCode => Some(theme.editor_bg),
            Self::BlockHead => Some(theme.status_bg),
            _ => None,
        }
    }

    fn ink(self, theme: &crate::theme::Theme) -> u32 {
        match self {
            Self::Normal | Self::Code | Self::Block | Self::BlockCode => theme.text_ink,
            Self::Dim | Self::BlockHead => theme.text_dim,
            Self::Accent => theme.text_accent,
            Self::Error => theme.text_error,
        }
    }
}

#[derive(Default)]
struct Content {
    lines: VecDeque<(Style, String)>,
    /// Wrapped-row offset up from the tail; 0 follows the newest line.
    scroll: usize,
    /// The input line: prompt + text + cursor (a char index into text).
    /// An empty prompt means the pane has no input line (outliner-style).
    prompt: String,
    input: String,
    cursor: usize,
    /// Whether the input line is armed (holds typing focus): draws the
    /// cursor block and reads as "type here".
    active: bool,
    /// Top-anchored panes (the outliner) list rows from the top down;
    /// transcripts (default) anchor at the bottom and grow upward.
    top_anchor: bool,
    /// A list pane whose rows' TEXT reads in the proportional label face —
    /// the file tree's names, as Zed's project panel sets them (IBM Plex
    /// Sans, which it ships as Zed Plex Sans) — while the leading marks
    /// (indent, ▸/▾, icon cells) keep the mono grid, so anything a host draws
    /// over a cell still lands on it.
    proportional: bool,
    /// Rows that fit in the pane at its last draw — cached so `reveal_row`
    /// (perf item 7) can scroll a selected row into view without re-deriving
    /// the pane geometry. 0 until the first draw.
    visible_rows: usize,
    /// Where each transcript row LANDED at the last draw, top to bottom —
    /// what mouse selection hit-tests against (Kevin, 2026-09-21: "copy from
    /// the repl and from the agent panel using the mouse"). A wrapped
    /// continuation row is marked so a copy rejoins it to its line.
    laid: Vec<LaidRow>,
    /// Cell advance and row height at the last draw, for column hit-tests.
    laid_metrics: (f32, f32, f32),
    /// The mouse selection as (anchor, extent) in (laid row, column); the
    /// anchor is where the press landed, so the order is whichever way the
    /// drag went.
    sel: Option<((usize, usize), (usize, usize))>,
}

#[derive(Clone, Debug)]
struct LaidRow {
    y: f32,
    text: String,
    /// True when this row continues the previous one's logical line.
    cont: bool,
    /// The logical line it shows (part of), in a transcript.
    line: usize,
}

thread_local! {
    static PANES: RefCell<HashMap<u32, Content>> = RefCell::new(HashMap::new());
}

fn with<R>(id: u32, f: impl FnOnce(&mut Content) -> R) -> R {
    PANES.with(|p| f(p.borrow_mut().entry(id).or_default()))
}

/// Append text (split on newlines) in one style; scroll snaps to the tail
/// so new output is always seen.
pub fn append(id: u32, style: Style, text: &str) {
    with(id, |c| {
        for line in text.split('\n') {
            if c.lines.len() >= MAX_LINES {
                c.lines.pop_front();
            }
            c.lines.push_back((style, line.to_string()));
        }
        c.scroll = 0;
    });
    crate::host::mark_dirty_ui();
}

pub fn clear(id: u32) {
    with(id, |c| {
        c.lines.clear();
        c.scroll = 0;
    });
    crate::host::mark_dirty_ui();
}

/// Scroll by wrapped rows: positive = back in time (up). Clamped to the
/// content at draw time; here only the floor at 0 (the tail) applies.
/// Returns the new (pre-clamp) offset so callers can see it took.
pub fn scroll_by(id: u32, rows: i32) -> usize {
    let s = with(id, |c| {
        c.scroll = c.scroll.saturating_add_signed(rows as isize);
        c.scroll
    });
    crate::host::mark_dirty_ui();
    s
}

// Row-level edits for the outliner's incremental refresh (perf item 7): the
// list is kept in sync with the scene by touching individual rows instead of
// clearing and re-appending every object on each selection/add/delete.

/// Number of rows currently held (the outliner's object count).
pub fn row_count(id: u32) -> usize {
    with(id, |c| c.lines.len())
}

/// A row's text (for tests/inspection), or None if out of range.
pub fn row_text(id: u32, idx: usize) -> Option<String> {
    with(id, |c| c.lines.get(idx).map(|(_, t)| t.clone()))
}

/// Replace row `idx`'s style + text (no-op if out of range). Used to re-style
/// exactly the two rows a selection change touches.
pub fn set_row(id: u32, idx: usize, style: Style, text: &str) {
    with(id, |c| {
        if let Some(slot) = c.lines.get_mut(idx) {
            *slot = (style, text.to_string());
        }
    });
    crate::host::mark_dirty_ui();
}

/// Insert a row at `idx` (clamped to the end) — one added object.
pub fn insert_row(id: u32, idx: usize, style: Style, text: &str) {
    with(id, |c| {
        let at = idx.min(c.lines.len());
        c.lines.insert(at, (style, text.to_string()));
    });
    crate::host::mark_dirty_ui();
}

/// Remove row `idx` (no-op if out of range) — one deleted object.
pub fn remove_row(id: u32, idx: usize) {
    with(id, |c| {
        if idx < c.lines.len() {
            c.lines.remove(idx);
        }
    });
    crate::host::mark_dirty_ui();
}

/// Scroll a top-anchored list so row `idx` is visible: if it's above the
/// window, park it at the top; if below, at the bottom. No-op when it already
/// shows or the list fits. Uses the row capacity cached at the last draw.
pub fn reveal_row(id: u32, idx: usize) {
    with(id, |c| {
        let v = c.visible_rows;
        if v == 0 || c.lines.len() <= v {
            c.scroll = 0;
            return;
        }
        if idx < c.scroll {
            c.scroll = idx;
        } else if idx >= c.scroll + v {
            c.scroll = idx + 1 - v;
        }
        let max_scroll = c.lines.len().saturating_sub(v);
        c.scroll = c.scroll.min(max_scroll);
    });
    crate::host::mark_dirty_ui();
}

/// Set the input line wholesale — the prelude's line editor mirrors its
/// state here after every edit (the kernel never interprets keys). `cursor`
/// is a CHARACTER index, which is what a Scheme string's `string-length`
/// and `substring` count in, clamped to the line, and it is stored as the
/// grid column it already is. (Under Janet it was a BYTE index, Janet's
/// strings being bytes, and this floored it to a char boundary; read that
/// way a Scheme cursor after an `é` drew a column early.)
pub fn set_input(id: u32, prompt: &str, input: &str, cursor: usize) {
    let col = cursor.min(input.chars().count());
    with(id, |c| {
        c.prompt = prompt.to_string();
        c.input = input.to_string();
        c.cursor = col;
    });
    crate::host::mark_dirty_ui();
}

pub fn set_active(id: u32, active: bool) {
    with(id, |c| c.active = active);
    crate::host::mark_dirty_ui();
}

/// Anchor a pane's rows to the top (the outliner's list layout).
pub fn set_top_anchor(id: u32, top: bool) {
    with(id, |c| c.top_anchor = top);
    crate::host::mark_dirty_ui();
}

/// Set a list pane's row text in the proportional label face (see
/// `Content::proportional`).
pub fn set_proportional(id: u32, on: bool) {
    with(id, |c| c.proportional = on);
    crate::host::mark_dirty_ui();
}

/// The LOGICAL line index under a window point in a top-anchored pane
/// (list rows don't wrap in practice; wrapped rows resolve to their line).
pub fn row_at(id: u32, x: f64, y: f64) -> Option<usize> {
    let rect = crate::host::rect(id)?;
    if !rect.contains(x, y) {
        return None;
    }
    let scale = crate::host::ui_scale();
    let px = font_px() * scale;
    let (_, row_h) = text::mono_metrics(px);
    let pad = PAD * scale;
    with(id, |c| {
        if !c.top_anchor {
            return None;
        }
        let row = ((y - rect.y as f64 - (pad * 0.5) as f64) / row_h as f64).floor();
        (row >= 0.0)
            .then(|| row as usize + c.scroll)
            .filter(|&r| r < c.lines.len())
    })
}

/// The LOGICAL line under a window point in a transcript, and the column in
/// it — what a click on a row's own words needs. The column is the laid
/// row's, so it is the line's only on a line's first row; `None` for a
/// continuation row's column. As last drawn.
pub fn line_at(id: u32, x: f64, y: f64) -> Option<(usize, Option<usize>)> {
    with(id, |c| {
        let (x0, advance, row_h) = c.laid_metrics;
        let row = c.laid.iter().find(|r| (y as f32) >= r.y && (y as f32) < r.y + row_h)?;
        let col = ((x as f32 - x0) / advance).floor();
        Some((row.line, (!row.cont && col >= 0.0).then_some(col as usize)))
    })
}

/// The style of row `idx`, by name.
pub fn row_style(id: u32, idx: usize) -> Option<&'static str> {
    with(id, |c| c.lines.get(idx).map(|(s, _)| s.name()))
}

/// The (laid row, column) under a window point, or None outside the rows.
/// Columns clamp to the row's length, so a drag past the end selects to
/// the end; a point below the last row selects to the last row's end.
fn cell_at(c: &Content, rect: Rect, x: f64, y: f64) -> Option<(usize, usize)> {
    if c.laid.is_empty() {
        return None;
    }
    let (x0, advance, row_h) = c.laid_metrics;
    let row = c
        .laid
        .iter()
        .position(|r| (y as f32) < r.y + row_h)
        .unwrap_or(c.laid.len() - 1);
    let text_len = c.laid[row].text.chars().count();
    let col = (((x as f32 - x0) / advance).round().max(0.0) as usize).min(text_len);
    let _ = rect;
    Some((row, col))
}

/// Left press over a transcript pane: a new selection anchors here (and any
/// old one goes). Only transcript panes select — the outliner's rows are
/// clickable things, the minibuffer is one line.
pub fn begin_select(id: u32, x: f64, y: f64) -> bool {
    let Some(rect) = crate::host::rect(id) else { return false };
    with(id, |c| {
        if c.top_anchor {
            return false;
        }
        c.sel = None;
        if let Some(cell) = cell_at(c, rect, x, y) {
            c.sel = Some((cell, cell));
        }
        crate::host::mark_dirty_ui();
        true
    })
}

/// Drag with the press held: extend the selection to the cell under the
/// pointer.
pub fn extend_select(id: u32, x: f64, y: f64) {
    let Some(rect) = crate::host::rect(id) else { return };
    with(id, |c| {
        if let (Some((anchor, _)), Some(cell)) = (c.sel, cell_at(c, rect, x, y)) {
            c.sel = Some((anchor, cell));
            crate::host::mark_dirty_ui();
        }
    });
}

/// The selected text, or None when nothing (or an empty span) is selected.
/// Wrapped continuation rows rejoin their line; distinct lines get newlines.
pub fn selection(id: u32) -> Option<String> {
    with(id, |c| {
        let (a, b) = c.sel?;
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if lo == hi {
            return None;
        }
        let mut out = String::new();
        for (row, r) in c.laid.iter().enumerate().take(hi.0 + 1).skip(lo.0) {
            let chars: Vec<char> = r.text.chars().collect();
            let from = if row == lo.0 { lo.1.min(chars.len()) } else { 0 };
            let to = if row == hi.0 { hi.1.min(chars.len()) } else { chars.len() };
            if row > lo.0 && !r.cont {
                out.push('\n');
            }
            out.extend(chars[from..to].iter());
        }
        (!out.is_empty()).then_some(out)
    })
}

/// The rows as last drawn, top to bottom: (y, text) — the harness's way to
/// aim a selection.
pub fn laid_rows(id: u32) -> Vec<(f32, String)> {
    with(id, |c| c.laid.iter().map(|r| (r.y, r.text.clone())).collect())
}

pub fn clear_selection(id: u32) {
    with(id, |c| {
        if c.sel.take().is_some() {
            crate::host::mark_dirty_ui();
        }
    });
}

// ── Popup overlay (M4.5 completion + M5 menus) ──────────────────────────────
// One global overlay — at most one popup at a time. The prelude computes content
// and selection; the kernel computes the box ONCE at show time (so row
// hit-testing works headless and before the first paint) and draws it
// last in every frame. Two anchors: above a pane's input line (completion)
// or at a window point growing down-right (context menus).

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    /// Rows in a box: completion lists and the linear popup menu.
    #[default]
    List,
    /// The marking menu's pie: label boxes on a ring around an anchor.
    Pie,
}

/// How translucent the MENUS are — the linear popup and the marking menu's
/// label boxes. Same glass as the toolbox (docs/modeler-toolbox-plan.md) and
/// the same default, so the two floating layers read as one material
/// (Kevin, 2026-09-23). Completion lists stay opaque: they sit over source
/// you are reading letter by letter.
const MENU_ALPHA_DEFAULT: f32 = 0.38;
/// A menu over a TEXT pane: nearly solid, so the text under it cannot fight
/// its labels (Kevin, 2026-09-28), but a little see-through, so it still reads
/// as floating over the pane (Kevin, 2026-11-14: "a bit of transparency").
const TEXT_MENU_ALPHA: f32 = 0.88;
thread_local! {
    static MENU_ALPHA: std::cell::Cell<f32> = const { std::cell::Cell::new(MENU_ALPHA_DEFAULT) };
}

/// Set the menu glass opacity (0 = invisible, 1 = solid); returns what is in
/// effect. A negative argument only reads it.
pub fn set_menu_alpha(a: f32) -> f32 {
    if a >= 0.0 {
        MENU_ALPHA.with(|m| m.set(a.clamp(0.0, 1.0)));
    }
    MENU_ALPHA.with(|m| m.get())
}

#[derive(Default)]
struct Overlay {
    kind: OverlayKind,
    /// Drawn on glass rather than a solid panel — the menus, not the
    /// completion lists that share this renderer.
    glass: bool,
    lines: Vec<String>,
    selected: usize,
    /// False while the pie's drag sits in the dead zone (no sector armed).
    sel_active: bool,
    active: bool,
    /// Menus draw in the proportional label face; completion stays
    /// monospace (symbols and paths align).
    proportional: bool,
    /// Precomputed at show time (physical px). For List: the one box.
    box_: Rect,
    /// For Pie: one label box per item, ring-ordered (0 = up, clockwise).
    boxes: Vec<Rect>,
    /// The pie's anchor (the press point).
    center: (f32, f32),
    /// The gesture's current end point — the ink trail (the MARK of
    /// marking menu) draws from the anchor to here.
    mark: Option<(f32, f32)>,
    row_h: f32,
    pad: f32,
}

thread_local! {
    static OVERLAY: RefCell<Overlay> = RefCell::new(Overlay::default());
}

/// A menu row as the prelude sends it: the label, then a TAB and the key
/// that runs the same command when it has one (`%item-text`).
fn split_key(line: &str) -> (&str, Option<&str>) {
    match line.split_once('\t') {
        Some((label, key)) => (label, Some(key)),
        None => (line, None),
    }
}

/// The space between a menu label and its key, at UI scale 1.
const KEY_GAP: f32 = 18.0;

/// The width of menu rows drawn as two columns — every label, the gap, then
/// every key — so the keys line up whatever the labels' lengths.
fn menu_text_width(px: f32, lines: &[String]) -> f32 {
    let (mut labels, mut keys) = (0.0f32, 0.0f32);
    for l in lines {
        let (label, key) = split_key(l);
        labels = labels.max(text::measure_label(px, label));
        if let Some(k) = key {
            keys = keys.max(text::measure_label(px, k));
        }
    }
    if keys > 0.0 { labels + KEY_GAP * crate::host::ui_scale() + keys } else { labels }
}

/// The overlay's box for `lines`, clamped into the window.
fn overlay_geometry(lines: &[String], proportional: bool) -> (f32, f32, f32, u32, u32) {
    let scale = crate::host::ui_scale();
    let px = font_px() * scale;
    let pad = PAD * scale;
    let (win_w, _) = crate::host::window();
    let (text_w, row_h) = if proportional {
        (
            menu_text_width(px, lines),
            // Menus breathe: a third extra leading over the bare label
            // height (user feedback — tight rows read as a list, not a
            // menu). Text centers in the row at draw time.
            text::label_height(px) * 1.35,
        )
    } else {
        let (advance, row_h) = text::mono_metrics(px);
        let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        (cols as f32 * advance, row_h)
    };
    let max_w = (win_w as f32 - 2.0 * pad).max(40.0);
    let w = (text_w.min(max_w) + 2.0 * pad).ceil() as u32;
    let h = (lines.len() as f32 * row_h + pad).ceil() as u32;
    (pad, row_h, px, w, h)
}

fn set_overlay(
    lines: Vec<String>,
    selected: usize,
    proportional: bool,
    box_: Rect,
    row_h: f32,
    pad: f32,
) {
    OVERLAY.with(|o| {
        *o.borrow_mut() = Overlay {
            kind: OverlayKind::List,
            glass: false,
            selected: selected.min(lines.len().saturating_sub(1)),
            sel_active: true,
            lines,
            active: true,
            proportional,
            box_,
            boxes: Vec::new(),
            center: (0.0, 0.0),
            mark: None,
            row_h,
            pad,
        };
    });
    crate::host::mark_dirty();
}

/// Show the marking-menu pie: label boxes on a ring around the anchor
/// (0 = up, clockwise — matching the drag-angle sector math in the prelude).
/// `selected` < 0 means no sector armed yet (dead zone).
pub fn show_pie(lines: &str, selected: i32, cx: f32, cy: f32, opaque: bool) {
    let labels: Vec<String> = lines.split('\n').map(|s| s.to_string()).collect();
    let scale = crate::host::ui_scale();
    let px = font_px() * scale;
    let pad = PAD * scale * 0.75;
    // The ring opens past eight sectors, so neighbours' boxes clear each
    // other once a key widens them.
    let radius = 84.0 * scale * (labels.len() as f32 / 8.0).max(1.0);
    let (win_w, win_h) = crate::host::window();
    let n = labels.len().max(1);
    let row_h = text::label_height(px);
    let boxes: Vec<Rect> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let ang = -std::f32::consts::FRAC_PI_2
                + i as f32 * std::f32::consts::TAU / n as f32;
            let w = menu_text_width(px, std::slice::from_ref(l)) + 2.0 * pad;
            let h = row_h + pad;
            // Anchor the box by its edge nearest the anchor: an east box grows
            // right from its ring point, a west one left, north and south
            // centred, and every angle between blends. A label with its key
            // is wide, and centred boxes on the west side ran into the
            // north ones.
            let (px_, py_) = (cx + ang.cos() * radius, cy + ang.sin() * radius);
            let bx = px_ - w / 2.0 + w / 2.0 * ang.cos();
            let by = py_ - h / 2.0 + h / 2.0 * ang.sin();
            Rect {
                x: bx.clamp(0.0, (win_w.saturating_sub(w as u32)) as f32) as u32,
                y: by.clamp(0.0, (win_h.saturating_sub(h as u32)) as f32) as u32,
                w: w as u32,
                h: h as u32,
            }
        })
        .collect();
    OVERLAY.with(|o| {
        *o.borrow_mut() = Overlay {
            kind: OverlayKind::Pie,
            glass: !opaque,
            selected: selected.max(0) as usize,
            sel_active: selected >= 0,
            lines: labels,
            active: true,
            proportional: true,
            box_: Rect::default(),
            boxes,
            center: (cx, cy),
            mark: None,
            row_h,
            pad,
        };
    });
    crate::host::mark_dirty();
}

/// Update the gesture's ink-trail end point (the drag position).
pub fn set_pie_mark(x: f32, y: f32) {
    OVERLAY.with(|o| {
        let mut o = o.borrow_mut();
        if o.kind == OverlayKind::Pie {
            o.mark = Some((x, y));
        }
    });
    crate::host::mark_dirty();
}

/// Show completion rows anchored above a pane's input line
/// ('\n'-separated; `selected` indexes into them).
pub fn set_completions(pane: u32, lines: &str, selected: usize) {
    let lines: Vec<String> = lines.split('\n').map(|s| s.to_string()).collect();
    let Some(rect) = crate::host::rect(pane) else { return };
    let (pad, row_h, _px, w, h) = overlay_geometry(&lines, false);
    let input_top = rect.y as f32 + rect.h as f32 - pad_y(rect.h as f32, row_h) - row_h;
    let x = rect.x + (pad * 0.5) as u32;
    let y = (input_top - h as f32 - 2.0).max(0.0) as u32;
    set_overlay(lines, selected, false, Rect { x, y, w, h }, row_h, pad);
}

/// Show completion rows anchored at a WINDOW POINT — the scene editor's
/// cursor — growing down; flips above the anchor when the bottom edge is
/// close. Monospace, same as the input-line completion.
pub fn set_completions_at(x: f32, y: f32, line_h: f32, lines: &str, selected: usize) {
    let lines: Vec<String> = lines.split('\n').map(|s| s.to_string()).collect();
    let (pad, row_h, _px, w, h) = overlay_geometry(&lines, false);
    let (win_w, win_h) = crate::host::window();
    let mut bx = x.max(0.0) as u32;
    // Below the cursor's line; above it when there's no room.
    let mut by = (y + line_h + 2.0).max(0.0) as u32;
    if bx + w > win_w {
        bx = win_w.saturating_sub(w);
    }
    if by + h > win_h {
        by = (y - h as f32 - 2.0).max(0.0) as u32;
    }
    set_overlay(lines, selected, false, Rect { x: bx, y: by, w, h }, row_h, pad);
}

/// Show a context menu at a window point, growing down-right (flipping up
/// or left at the window edges).
pub fn show_menu(lines: &str, selected: usize, x: f32, y: f32, opaque: bool) {
    let lines: Vec<String> = lines.split('\n').map(|s| s.to_string()).collect();
    let (pad, row_h, _px, w, h) = overlay_geometry(&lines, true);
    let (win_w, win_h) = crate::host::window();
    let mut bx = x.max(0.0) as u32;
    let mut by = y.max(0.0) as u32;
    if bx + w > win_w {
        bx = win_w.saturating_sub(w);
    }
    if by + h > win_h {
        by = by.saturating_sub(h);
    }
    set_overlay(lines, selected, true, Rect { x: bx, y: by, w, h }, row_h, pad);
    // A MENU, not a completion list: same glass as the pie it is the browse
    // form of, and as the toolbox — EXCEPT over a text pane, where the glass
    // buys nothing (there is no model to see through to) and costs legibility
    // against the text underneath (Kevin, 2026-09-28).
    OVERLAY.with(|o| o.borrow_mut().glass = !opaque);
}

/// Is the overlay currently drawn on glass? The menus are, over a 3D pane;
/// over a text pane they are solid. Read by the test that holds that rule.
#[doc(hidden)]
// Not `#[cfg(test)]`, though only tests call it: the modeler's
// `script/vm_tests.rs` (the module that boots a VM) reaches it, and a
// dependency is built without `cfg(test)` when the modeler's tests run.
pub fn overlay_is_glass() -> bool {
    OVERLAY.with(|o| o.borrow().glass)
}

/// Move the selection highlight without recomputing geometry (hover).
pub fn set_overlay_selected(selected: usize) {
    OVERLAY.with(|o| {
        let mut o = o.borrow_mut();
        o.selected = selected.min(o.lines.len().saturating_sub(1));
    });
    crate::host::mark_dirty();
}

/// The overlay row under a window point, if the (active) overlay contains
/// it — the mouse half of menu navigation.
pub fn overlay_hit(x: f64, y: f64) -> Option<usize> {
    OVERLAY.with(|o| {
        let o = o.borrow();
        if !o.active || !o.box_.contains(x, y) {
            return None;
        }
        let row = ((y - o.box_.y as f64 - (o.pad * 0.5) as f64) / o.row_h as f64).floor();
        (row >= 0.0 && (row as usize) < o.lines.len()).then_some(row as usize)
    })
}

pub fn overlay_active() -> bool {
    OVERLAY.with(|o| o.borrow().active)
}

pub fn clear_completions() {
    OVERLAY.with(|o| o.borrow_mut().active = false);
    crate::host::mark_dirty();
}

/// Test-only: (lines, selected, active).
#[doc(hidden)]
// Not `#[cfg(test)]`, though only tests call it: the modeler's
// `script/vm_tests.rs` (the module that boots a VM) reaches it, and a
// dependency is built without `cfg(test)` when the modeler's tests run.
pub fn completions() -> (Vec<String>, usize, bool) {
    OVERLAY.with(|o| {
        let o = o.borrow();
        (o.lines.clone(), o.selected, o.active)
    })
}

/// Vertical padding that adapts to short panes (the 26px minibuffer strip
/// can't afford the full PAD around its one row).
fn pad_y(rect_h: f32, row_h: f32) -> f32 {
    PAD.min(((rect_h - row_h) / 2.0).max(1.0))
}

/// Draw the popup overlay, if active — called LAST in paint, after every
/// pane and the status strip.
pub fn draw_overlay(fb: &mut Framebuffer) {
    let (kind, lines, selected, sel_active, active, proportional, bx, boxes, center, mark, row_h, pad, glass) =
        OVERLAY.with(|o| {
            let o = o.borrow();
            (
                o.kind,
                o.lines.clone(),
                o.selected,
                o.sel_active,
                o.active,
                o.proportional,
                o.box_,
                o.boxes.clone(),
                o.center,
                o.mark,
                o.row_h,
                o.pad,
                o.glass,
            )
        });
    if !active || lines.is_empty() {
        return;
    }
    let theme = crate::theme::get();
    let scale = crate::host::ui_scale();
    let px = font_px() * scale;
    // 1.0 for a completion list; the menu glass for a menu. The SELECTED row
    // is lifted toward solid — a highlight you can see through stops reading
    // as "this one", which is the only thing the highlight is for.
    // A menu over a text pane is nearly solid; a completion list (the one
    // overlay in the monospace face) is solid.
    let a = if glass {
        MENU_ALPHA.with(|m| m.get())
    } else if proportional {
        TEXT_MENU_ALPHA
    } else {
        1.0
    };
    let a_sel = (a * 0.5 + 0.5).min(1.0);
    let full = Rect { x: 0, y: 0, w: fb.width, h: fb.height };

    if kind == OverlayKind::Pie {
        // The ink trail — the MARK — from the anchor along the stroke,
        // under everything else (Kurtenbach's visual signature).
        if let Some((mx, my)) = mark {
            for off in [-1.0f32, 0.0, 1.0] {
                crate::framebuffer::draw_line_2d(
                    center.0 + off,
                    center.1,
                    mx + off,
                    my,
                    theme.text_accent,
                    fb,
                    full,
                );
            }
        }
        // The anchor dot, then each label box on the ring; the armed
        // sector's box carries the accent.
        let dot = 4.0 * scale;
        fb.round_rect(
            Rect {
                x: (center.0 - dot).max(0.0) as u32,
                y: (center.1 - dot).max(0.0) as u32,
                w: (dot * 2.0) as u32,
                h: (dot * 2.0) as u32,
            }
            .clamped(fb.width, fb.height),
            dot,
            theme.text_accent,
            theme.text_accent,
        );
        for (i, (label, b)) in lines.iter().zip(boxes.iter()).enumerate() {
            let b = b.clamped(fb.width, fb.height);
            if b.w == 0 || b.h == 0 {
                continue;
            }
            let armed = sel_active && i == selected;
            let (bg, ink, key_ink) = if armed {
                (theme.text_accent, theme.popup_bg, theme.popup_bg)
            } else {
                (theme.popup_bg, theme.menu_ink, theme.menu_key_ink)
            };
            crate::framebuffer::glass_rect(
                fb,
                b,
                8.0 * scale,
                bg,
                if armed { a_sel } else { a },
                0.10 * a,
                full,
            );
            let ty = b.y as f32 + (b.h as f32 - row_h) * 0.5;
            let (text, key) = split_key(label);
            let x = b.x as f32 + pad;
            text::draw_label(fb, x as i32, ty as i32, px, ink, 1.0, text);
            if let Some(k) = key {
                let kx = x + text::measure_label(px, text) + KEY_GAP * scale;
                text::draw_label(fb, kx as i32, ty as i32, px, key_ink, if armed { 0.7 } else { 1.0 }, k);
            }
        }
        return;
    }

    let bx = bx.clamped(fb.width, fb.height);
    if bx.w == 0 || bx.h == 0 {
        return;
    }
    if a < 1.0 {
        crate::framebuffer::glass_rect(fb, bx, 9.0 * scale, theme.popup_bg, a, 0.10 * a, full);
    } else {
        fb.round_rect(bx, 9.0 * scale, theme.popup_bg, theme.border_focused);
    }

    // Center each row's text in its (possibly padded) row height.
    let text_dy = if proportional {
        ((row_h - text::label_height(px)) * 0.5).max(0.0)
    } else {
        0.0
    };
    // A menu's key sits right-aligned in its own column, in its own ink.
    let key_right = bx.x as f32 + bx.w as f32 - pad;
    let draw_row = |fb: &mut Framebuffer, x: f32, y: f32, ink: u32, key_ink: u32, line: &str| {
        if proportional {
            let (label, key) = split_key(line);
            text::draw_label(fb, x as i32, (y + text_dy) as i32, px, ink, 1.0, label);
            if let Some(k) = key {
                let kx = key_right - text::measure_label(px, k);
                text::draw_label(fb, kx as i32, (y + text_dy) as i32, px, key_ink, 1.0, k);
            }
        } else {
            text::draw_mono_line(fb, x, y, px, ink, line);
        }
    };
    for (i, line) in lines.iter().enumerate() {
        let ly = bx.y as f32 + pad * 0.5 + i as f32 * row_h;
        if i == selected {
            let sel = Rect {
                x: bx.x + 2,
                y: ly as u32,
                w: bx.w.saturating_sub(4),
                h: row_h as u32,
            };
            let sel = sel.clamped(fb.width, fb.height);
            if a < 1.0 {
                crate::framebuffer::glass_rect(
                    fb, sel, 5.0 * scale, theme.text_accent, a_sel, 0.0, full);
            } else {
                fb.round_rect(sel, 5.0 * scale, theme.text_accent, theme.text_accent);
            }
            draw_row(fb, bx.x as f32 + pad, ly, theme.status_bg, theme.status_bg, line);
        } else if proportional {
            draw_row(fb, bx.x as f32 + pad, ly, theme.menu_ink, theme.menu_key_ink, line);
        } else {
            draw_row(fb, bx.x as f32 + pad, ly, theme.text_ink, theme.text_ink, line);
        }
    }
}

/// Wrap one logical line into `cols`-wide rows (char-grid math — the
/// monospace dividend). An empty line still occupies one row.
/// Wrap a line to `cols` at WORD boundaries, continuation rows indented to
/// the line's own leading whitespace plus two — so a request reads as a
/// sentence and a Lisp form keeps the shape the model gave it (Kevin,
/// 2026-09-22). A word longer than a row is cut, as before.
fn wrap(line: &str, cols: usize) -> Vec<String> {
    wrap_mapped(line, cols).into_iter().map(|(t, _)| t).collect()
}

/// [`wrap`] plus, per row, which SOURCE char index each cell shows (None =
/// the synthetic hanging indent). The map is what lets the input line place
/// its cursor and re-ink its prompt on word-wrapped rows — char-grid
/// division (`cursor / cols`) lies the moment a break drops a space or a
/// continuation row opens with indent, and the cursor drifted into the
/// middle of the text (Kevin, 2026-09-23).
fn wrap_mapped(line: &str, cols: usize) -> Vec<(String, Vec<Option<usize>>)> {
    let cols = cols.max(1);
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return vec![(String::new(), Vec::new())];
    }
    if chars.len() <= cols {
        return vec![(line.to_string(), (0..chars.len()).map(Some).collect())];
    }
    let lead = chars.iter().take_while(|c| **c == ' ').count();
    // A hanging indent that would leave no room for words is no indent.
    let hang = if lead + 2 < cols / 2 { lead + 2 } else { 0 };
    // The hang indent is part of the row's CHARACTERS, so every row is
    // capped at `cols` outright — the old per-branch `limit` arithmetic
    // let a cut word carry the indent on top of a full-width chunk, and a
    // long unbroken word (the /login OAuth URL) overran the pane.
    let mut rows: Vec<(String, Vec<Option<usize>>)> = Vec::new();
    let mut row: Vec<char> = Vec::new();
    let mut map: Vec<Option<usize>> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // The next word, with the whitespace that precedes it.
        let start = i;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        while i < chars.len() && chars[i] != ' ' {
            i += 1;
        }
        if row.len() + (i - start) <= cols {
            for (k, &ch) in chars.iter().enumerate().take(i).skip(start) {
                row.push(ch);
                map.push(Some(k));
            }
            continue;
        }
        // The word does not fit: close the row (unless it holds nothing
        // but indent), then lay the word bare — its leading space fell at
        // the break — cutting it wherever a row fills.
        if row.iter().any(|c| *c != ' ') || (rows.is_empty() && !row.is_empty()) {
            rows.push((row.iter().collect(), std::mem::take(&mut map)));
            row.clear();
            row.extend(std::iter::repeat_n(' ', hang));
            map.extend(std::iter::repeat_n(None, hang));
        }
        let mut k = start;
        while k < i && chars[k] == ' ' {
            k += 1;
        }
        while k < i {
            if row.len() >= cols {
                rows.push((row.iter().collect(), std::mem::take(&mut map)));
                row.clear();
                row.extend(std::iter::repeat_n(' ', hang));
                map.extend(std::iter::repeat_n(None, hang));
            }
            row.push(chars[k]);
            map.push(Some(k));
            k += 1;
        }
    }
    if row.iter().any(|c| *c != ' ') || rows.is_empty() {
        rows.push((row.iter().collect(), map));
    }
    rows
}

/// Where source index CURSOR sits in MAPPED's rows — after the cell whose
/// source it follows when the index itself was dropped at a break (a space)
/// or is the end of the line.
fn cursor_cell(mapped: &[(String, Vec<Option<usize>>)], cursor: usize, cols: usize) -> (usize, usize) {
    let mut after: Option<(usize, usize, usize)> = None; // (src, row, col)
    for (ri, (_, map)) in mapped.iter().enumerate() {
        for (ci, s) in map.iter().enumerate() {
            match s {
                Some(s) if *s == cursor => return (ri, ci),
                Some(s) if *s < cursor && after.map(|(b, ..)| *s >= b).unwrap_or(true) => {
                    after = Some((*s, ri, ci));
                }
                _ => {}
            }
        }
    }
    match after {
        Some((_, ri, ci)) if ci + 1 < cols => (ri, ci + 1),
        Some((_, ri, _)) if ri + 1 < mapped.len() => {
            let map = &mapped[ri + 1].1;
            (ri + 1, map.iter().position(|s| s.is_some()).unwrap_or(map.len()))
        }
        Some((_, ri, ci)) => (ri, ci),
        None => (0, 0),
    }
}

/// Draw a Lisp source row token by token in the code palette.
fn draw_code_row(fb: &mut Framebuffer, x: f32, y: f32, px: f32, theme: &crate::theme::Theme, row: &str) {
    let advance = text::mono_metrics(px).0;
    let chars: Vec<char> = row.chars().collect();
    let mut i = 0;
    let mut after_open = false; // the next symbol is a form's head
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        let ink;
        if c == '(' || c == ')' || c == '[' || c == ']' || c == '{' || c == '}' {
            i += 1;
            ink = theme.code_paren;
            after_open = c == '(';
        } else if c == ' ' {
            i += 1;
            continue;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            ink = theme.code_string;
            after_open = false;
        } else if c == '#' {
            i = chars.len();
            ink = theme.code_comment;
        } else {
            while i < chars.len() && !matches!(chars[i], ' ' | '(' | ')' | '[' | ']' | '{' | '}' | '"') {
                i += 1;
            }
            let tok: String = chars[start..i].iter().collect();
            ink = if tok.starts_with(':') {
                theme.code_keyword
            } else if tok.parse::<f64>().is_ok() {
                theme.code_number
            } else if after_open {
                theme.code_call
            } else {
                theme.text_ink
            };
            after_open = false;
        }
        let seg: String = chars[start..i].iter().collect();
        text::draw_mono_line(fb, x + start as f32 * advance, y, px, ink, &seg);
    }
}

/// The characters a file-tree row holds where its icon goes: private-use
/// code points no face draws, so the cells stay blank under the icon. Two
/// of one make the icon's width; which one says what the row is (17-files
/// picks it from the name).
pub const FOLDER_MARK: char = '\u{E000}';
pub const CODE_MARK: char = '\u{E001}';
pub const IMAGE_MARK: char = '\u{E002}';
pub const FILE_MARK: char = '\u{E003}';

/// Whether `c` is one of the file tree's icon marks.
pub fn is_icon_mark(c: char) -> bool {
    ('\u{E000}'..='\u{E003}').contains(&c)
}

/// The file tree's icons, drawn over the mark cells its rows carry, and
/// its indent guides: a faint rule down from each open folder through the
/// rows under it, as Zed's project panel draws them. Drawn for any
/// proportional list pane, so every host's tree has them (the standalone's
/// and the modeler's).
#[allow(clippy::too_many_arguments)]
fn draw_tree_marks(fb: &mut Framebuffer, pane: Rect, theme: &crate::theme::Theme, px: f32, scale: f32, laid: &[LaidRow]) {
    let (adv, row_h) = text::mono_metrics(px);
    let pad = PAD * scale;
    let at = |v: f32| v.max(0.0).round() as u32;
    let t = scale.round().max(1.0);
    let dim = text::blend(theme.panel_bg, theme.text_dim, 0.85);
    let guide = text::blend(theme.panel_bg, theme.text_dim, 0.28);
    let outline = |fb: &mut Framebuffer, x: f32, y: f32, w: f32, h: f32| {
        fb.fill_rect(Rect { x: at(x), y: at(y), w: at(w), h: at(t) }, dim);
        fb.fill_rect(Rect { x: at(x), y: at(y + h - t), w: at(w), h: at(t) }, dim);
        fb.fill_rect(Rect { x: at(x), y: at(y), w: at(t), h: at(h) }, dim);
        fb.fill_rect(Rect { x: at(x + w - t), y: at(y), w: at(t), h: at(h) }, dim);
    };
    let bottom = (pane.y + pane.h) as f32;
    for LaidRow { y, text: row, .. } in laid {
        let y = *y;
        let Some((col, mark)) = row.chars().enumerate().find(|&(_, c)| is_icon_mark(c)) else { continue };
        let x = pane.x as f32 + pad + col as f32 * adv;
        // A row at depth d has its icon at column 2d+2 and sits under every
        // folder above it: one guide per level, under that folder's marker.
        let depth = col.saturating_sub(2) / 2;
        for level in 1..depth {
            let gx = pane.x as f32 + pad + (2 * level) as f32 * adv + adv * 0.5;
            let h = row_h.min(bottom - y);
            if h > 0.0 {
                fb.fill_rect(Rect { x: at(gx), y: at(y), w: at(t), h: at(h) }, guide);
            }
        }
        match mark {
            FOLDER_MARK => {
                let ink = theme.code_number;
                let (w, h) = (adv * 1.7, row_h * 0.5);
                let (x0, y0) = (x + adv * 0.1, y + (row_h - h) * 0.55);
                let tab_h = (h * 0.25).max(1.0);
                fb.fill_rect(Rect { x: at(x0), y: at(y0), w: at(w * 0.45), h: at(tab_h) }, ink);
                fb.fill_rect(Rect { x: at(x0), y: at(y0 + tab_h), w: at(w), h: at(h - tab_h) }, ink);
            }
            // Source: braces, as Zed marks code.
            CODE_MARK => {
                let small = px * 0.85;
                let dy = text::mono_ascent(px) - text::mono_ascent(small);
                text::draw_mono_line(fb, x + adv * 0.15, y + dy, small, dim, "{}");
            }
            // A picture: a frame, a hill, a sun.
            IMAGE_MARK => {
                let (w, h) = (adv * 1.5, row_h * 0.55);
                let (x0, y0) = (x + adv * 0.2, y + (row_h - h) * 0.5);
                outline(fb, x0, y0, w, h);
                let base = y0 + h - t;
                for i in 0..3 {
                    let step = h * 0.16 * (i + 1) as f32;
                    let half = w * (0.38 - 0.11 * i as f32);
                    let cx = x0 + w * 0.42;
                    fb.fill_rect(Rect { x: at(cx - half), y: at(base - step), w: at(2.0 * half), h: at(h * 0.16) }, dim);
                }
                let s = (h * 0.18).max(t);
                fb.fill_rect(Rect { x: at(x0 + w * 0.68), y: at(y0 + h * 0.22), w: at(s), h: at(s) }, dim);
            }
            // Anything else: a page with lines on it.
            _ => {
                let (w, h) = (adv * 1.1, row_h * 0.62);
                let (x0, y0) = (x + adv * 0.4, y + (row_h - h) * 0.5);
                outline(fb, x0, y0, w, h);
                for i in 1..=3 {
                    let ly = y0 + h * 0.22 * i as f32;
                    fb.fill_rect(Rect { x: at(x0 + w * 0.25), y: at(ly), w: at(w * 0.5), h: at(t) }, dim);
                }
            }
        }
    }
}

/// Draw the pane: background, transcript bottom-anchored above the input
/// line, input line (when a prompt is set) with a cursor block if armed.
/// Returns nothing; clamps and clips everywhere.
pub fn draw(id: u32, fb: &mut Framebuffer, rect: Rect, focused: bool) {
    let rect = rect.clamped(fb.width, fb.height);
    if rect.w < 20 || rect.h < 20 {
        return;
    }
    let theme = crate::theme::get();
    fb.fill_rect(rect, theme.panel_bg);

    // Logical sizes scale to the display (retina draws twice the pixels). The
    // minibuffer/echo strip renders at the status size (it shares the bottom
    // strip with the right-aligned status label), not the larger reading-pane
    // font — so the echo line matches the status text.
    let scale = crate::host::ui_scale();
    // The minibuffer's prompt/editor draws in the MONO face; the status + echo
    // it shares the strip with use the PROPORTIONAL face at STRIP_TEXT_PX. Mono
    // reads chunkier at the same px, so scale it down to sit visually level with
    // them (≈ Monaco/JetBrains-vs-DejaVu cap-height ratio).
    let strip = crate::host::strip_font_px(id);
    let is_mini = strip.is_some();
    let logical = match strip {
        Some(strip_px) => strip_px * 0.94,
        None => font_px(),
    };
    let px = logical * scale;
    let pad = PAD * scale;
    let (advance, row_h) = text::mono_metrics(px);
    let cols = ((rect.w as f32 - 2.0 * pad) / advance).max(1.0) as usize;

    PANES.with(|p| {
        let mut panes = p.borrow_mut();
        let c = panes.entry(id).or_default();

        // The input line owns the bottom row when a prompt is set. Vertical
        // padding shrinks for short panes (the minibuffer strip). The
        // minibuffer shows a single line — center it in the (taller) status
        // strip instead of bottom-anchoring, so echo/prompt sit level with the
        // right-hand status label and toggle icons.
        let mut bottom = if is_mini {
            // Center on the cap band, not the line box, so the ink sits level
            // with the toggle icons (which center their tight box).
            text::mono_center_top(px, rect.y as f32, rect.h as f32)
        } else {
            rect.y as f32 + rect.h as f32 - pad_y(rect.h as f32, row_h) - row_h
        };
        if !c.prompt.is_empty() {
            let line: String = format!("{}{}", c.prompt, c.input);
            let cursor_col = c.prompt.chars().count() + c.cursor;
            let x = rect.x as f32 + pad;
            let prompt_len = c.prompt.chars().count();
            // The input WRAPS in a transcript pane (Kevin, 2026-09-21: "word
            // wrap in the agent panel when I type") — a request is a
            // sentence, and a sentence scrolling off to the left is
            // unreadable. The minibuffer is one strip high, so it keeps the
            // sliding window (prompt scrolls off first — Emacs behavior).
            let mapped: Vec<(String, Vec<Option<usize>>)> = if is_mini {
                let start = cursor_col.saturating_sub(cols.saturating_sub(1));
                let text: String = line.chars().skip(start).take(cols).collect();
                let map = (start..start + text.chars().count()).map(Some).collect();
                vec![(text, map)]
            } else {
                // One spare row so a cursor at the very end of a full row
                // has somewhere to sit.
                let mut mapped = wrap_mapped(&line, cols);
                if cursor_col == line.chars().count()
                    && mapped.last().map(|(t, _)| t.chars().count() >= cols).unwrap_or(false)
                {
                    mapped.push((String::new(), Vec::new()));
                }
                mapped
            };
            let n_rows = mapped.len();
            let block_top = bottom - row_h * (n_rows as f32 - 1.0);
            for (i, (row, map)) in mapped.iter().enumerate() {
                let y = block_top + row_h * i as f32;
                text::draw_mono_line(fb, x, y, px, theme.text_ink, row);
                // Re-ink the prompt span in accent (cheap: draw it again on
                // top) — the cells at the row's start whose source char is
                // still inside the prompt.
                let vis_prompt: String = row
                    .chars()
                    .zip(map.iter())
                    .take_while(|(_, s)| matches!(s, Some(x) if *x < prompt_len))
                    .map(|(ch, _)| ch)
                    .collect();
                if !vis_prompt.is_empty() {
                    text::draw_mono_line(fb, x, y, px, theme.text_accent, &vis_prompt);
                }
            }
            if c.active && focused {
                // Block cursor: filled cell behind, glyph re-inked in the
                // panel color so the character stays readable.
                let (crow, ccol) = cursor_cell(&mapped, cursor_col, cols);
                let cy = block_top + row_h * crow as f32;
                let col = ccol as f32;
                let cur = Rect {
                    x: (x + col * advance) as u32,
                    y: cy as u32,
                    w: advance.ceil() as u32,
                    h: row_h as u32 - 1,
                };
                fb.fill_rect(cur.clamped(fb.width, fb.height), theme.text_accent);
                // Re-ink what the CELL shows (with word wrap that is not
                // always line[cursor] — a dropped break space, the indent).
                if let Some(ch) = mapped[crow].0.chars().nth(ccol) {
                    text::draw_mono_line(
                        fb,
                        x + col * advance,
                        cy,
                        px,
                        theme.panel_bg,
                        &ch.to_string(),
                    );
                }
            }
            bottom = block_top - row_h * 1.2;
        }

        if c.top_anchor {
            // List layout: rows from the top down, `scroll` = rows skipped
            // off the top. No input line, no wrapping upward.
            let top = rect.y as f32 + pad * 0.5;
            let visible = ((rect.h as f32 - pad) / row_h).max(0.0) as usize;
            c.visible_rows = visible; // cache for reveal_row (item 7)
            let max_scroll = c.lines.len().saturating_sub(visible);
            c.scroll = c.scroll.min(max_scroll);
            let mut y = top;
            // Where the rows land, as the transcript records them: a host
            // that draws over a row (the file tree's folder icons) reads it.
            let mut laid: Vec<LaidRow> = Vec::new();
            for (style, line) in c.lines.iter().skip(c.scroll) {
                if y + row_h > rect.y as f32 + rect.h as f32 {
                    break;
                }
                let row: String = line.chars().take(cols).collect();
                if c.proportional {
                    // The leading marks on the mono grid; the rest — the
                    // name — in the label face, on the same baseline. The
                    // name is cut to the pane by its OWN width, not the mono
                    // column count, which would cut a narrower face short.
                    let split = line
                        .char_indices()
                        .find(|&(_, ch)| !matches!(ch, ' ' | '▸' | '▾' | '\u{E000}'..='\u{E003}'))
                        .map_or(line.len(), |(i, _)| i);
                    let (marks, name) = line.split_at(split);
                    let x0 = rect.x as f32 + pad;
                    text::draw_mono_line(fb, x0, y, px, style.ink(&theme), marks);
                    let nx = x0 + marks.chars().count() as f32 * advance;
                    // A touch above the text size: Plex Sans reads smaller
                    // than a mono face at the same px (Kevin, 2026-11-13).
                    let lpx = px * PROPORTIONAL_SHARE;
                    let room = rect.x as f32 + rect.w as f32 - pad - nx;
                    let mut name = name.to_string();
                    while !name.is_empty() && text::measure_label(lpx, &name) > room {
                        name.pop();
                    }
                    let ny = y + text::mono_ascent(px) - text::label_ascent(lpx);
                    text::draw_label(fb, nx.round() as i32, ny.round() as i32, lpx, style.ink(&theme), 1.0, &name);
                } else {
                    text::draw_mono_line(fb, rect.x as f32 + pad, y, px, style.ink(&theme), &row);
                }
                laid.push(LaidRow { y, text: row, cont: false, line: laid.len() + c.scroll });
                y += row_h;
            }
            // The file tree's icons and indent guides, over its mark cells.
            if c.proportional {
                draw_tree_marks(fb, rect, &theme, px, scale, &laid);
            }
            c.laid = laid;
            return;
        }

        // Transcript: walk lines newest-first, wrap each, and lay rows
        // upward from the bottom, skipping `scroll` rows off the tail.
        let top = rect.y as f32 + pad * 0.5;
        let visible_rows = ((bottom - top) / row_h).max(0.0) as usize + 1;
        // (style, text, continues-previous-row, logical line)
        let mut rows: Vec<(Style, String, bool, usize)> = Vec::new();
        'collect: for (index, (style, line)) in c.lines.iter().enumerate().rev() {
            let wrapped = wrap(line, cols);
            let n = wrapped.len();
            for (i, row) in wrapped.into_iter().enumerate().rev() {
                rows.push((*style, row, i > 0, index));
                if rows.len() >= visible_rows + c.scroll {
                    // A cut mid-line: the row above is off screen, so this
                    // one cannot claim to continue it.
                    let _ = n;
                    break 'collect;
                }
            }
        }
        // Clamp scroll to what exists so scrolling past the top parks at it.
        let max_scroll = rows.len().saturating_sub(visible_rows);
        c.scroll = c.scroll.min(max_scroll);
        // With no input line the transcript's newest row takes the bottom
        // slot itself — a one-row pane (the minibuffer's echo duty) shows
        // its latest line instead of nothing.
        let mut y = if c.prompt.is_empty() { bottom } else { bottom - row_h };
        let x0 = rect.x as f32 + pad;
        // Record where the rows land (top to bottom) for the mouse.
        let mut laid: Vec<LaidRow> = Vec::new();
        for (style, row, cont, line) in rows.iter().skip(c.scroll) {
            if y < top {
                break;
            }
            laid.push(LaidRow { y, text: row.clone(), cont: *cont, line: *line });
            // A code block's ground, under the selection and the ink: the
            // pane's width less a margin, so it reads as a block set in.
            if let Some(ground) = style.band(&theme) {
                let inset = (pad * 0.5) as u32;
                let band = Rect { x: rect.x + inset, y: y as u32, w: rect.w.saturating_sub(2 * inset), h: row_h.ceil() as u32 };
                fb.fill_rect(band.clamped(fb.width, fb.height), ground);
            }
            y -= row_h;
        }
        laid.reverse();
        if let Some(first) = laid.first_mut() {
            first.cont = false; // nothing above it on screen to continue
        }
        // The selection highlight goes under the ink.
        if let Some((a, b)) = c.sel {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            for (i, r) in laid.iter().enumerate() {
                if i < lo.0 || i > hi.0 {
                    continue;
                }
                let len = r.text.chars().count();
                let from = if i == lo.0 { lo.1.min(len) } else { 0 };
                let to = if i == hi.0 { hi.1.min(len) } else { len };
                if to > from {
                    let hl = Rect {
                        x: (x0 + from as f32 * advance) as u32,
                        y: r.y as u32,
                        w: ((to - from) as f32 * advance).ceil() as u32,
                        h: row_h as u32,
                    };
                    fb.fill_rect(hl.clamped(fb.width, fb.height), theme.popup_bg);
                }
            }
        }
        let mut y = if c.prompt.is_empty() { bottom } else { bottom - row_h };
        for (style, row, _, _) in rows.iter().skip(c.scroll) {
            if y < top {
                break;
            }
            if matches!(style, Style::Code | Style::BlockCode) {
                draw_code_row(fb, x0, y, px, &theme, row);
            } else if *style == Style::BlockHead {
                // Its language dim; the words between marks in accent, as
                // things to click.
                let mut action = false;
                for (i, part) in row.split(ACTION_MARK).enumerate() {
                    let before: usize = row.split(ACTION_MARK).take(i).map(|p| p.chars().count() + 1).sum();
                    let ink = if action { theme.text_accent } else { theme.text_dim };
                    text::draw_mono_line(fb, x0 + before as f32 * advance, y, px, ink, part);
                    action = !action;
                }
            } else {
                text::draw_mono_line(fb, x0, y, px, style.ink(&theme), row);
            }
            y -= row_h;
        }
        c.laid = laid;
        c.laid_metrics = (x0, advance, row_h);
    });
}

/// Test-only: the transcript as (style, text) pairs.
#[doc(hidden)]
// Not `#[cfg(test)]`, though only tests call it: the modeler's
// `script/vm_tests.rs` (the module that boots a VM) reaches it, and a
// dependency is built without `cfg(test)` when the modeler's tests run.
pub fn lines(id: u32) -> Vec<(Style, String)> {
    with(id, |c| c.lines.iter().cloned().collect())
}

/// Test-only: the input line's text and cursor column.
#[doc(hidden)]
// Not `#[cfg(test)]`, though only tests call it: the modeler's
// `script/vm_tests.rs` (the module that boots a VM) reaches it as well as
// this file's own cursor test, and a dependency is built without
// `cfg(test)` when the modeler's tests run.
pub fn input(id: u32) -> (String, usize) {
    with(id, |c| (c.input.clone(), c.cursor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_words_with_a_hanging_indent() {
        assert_eq!(wrap("add a tall thin cylinder", 12), vec!["add a tall", "  thin", "  cylinder"]);
        // A form keeps its own indentation; continuation rows hang two deeper.
        assert_eq!(
            wrap("  (add-sphere :at [0 1 0] :radius 1.5)", 20),
            vec!["  (add-sphere :at [0", "    1 0] :radius", "    1.5)"]
        );
        // Deep indentation that would starve the rows gets no hang.
        let deep = format!("{}{}", " ".repeat(10), "x ".repeat(8));
        assert!(wrap(deep.trim_end(), 16).iter().all(|r| r.chars().count() <= 16));
    }

    #[test]
    fn wrap_never_exceeds_cols() {
        // A long unbroken word mid-sentence (the /login OAuth URL) used to
        // carry the hanging indent ON TOP of a full-width chunk and overrun
        // the pane.
        let url = format!("If the browser didn't open, visit: https://claude.com/oauth?{}", "x".repeat(300));
        for cols in [12, 20, 40, 80] {
            assert!(wrap(&url, cols).iter().all(|r| r.chars().count() <= cols), "cols={cols}");
        }
    }

    #[test]
    fn cursor_follows_word_wrap() {
        let line = "agent> make a tall thin cylinder";
        let mapped = wrap_mapped(line, 12);
        // Every source char is somewhere, exactly once.
        let cells: Vec<usize> = mapped.iter().flat_map(|(_, m)| m.iter().filter_map(|s| *s)).collect();
        let mut seen = cells.clone();
        seen.dedup();
        assert_eq!(seen.len(), cells.len());
        // The cursor at the very end sits after the last glyph, not at
        // len % cols of some earlier row (the drift Kevin saw).
        let (row, col) = cursor_cell(&mapped, line.chars().count(), 12);
        assert_eq!(row, mapped.len() - 1);
        let last = &mapped[row].0;
        assert_eq!(col, last.chars().count());
        // A cursor ON a char sits on that char's cell.
        let idx = line.find("cylinder").unwrap();
        let (row, col) = cursor_cell(&mapped, idx, 12);
        assert_eq!(mapped[row].1[col], Some(idx));
        assert_eq!(mapped[row].0.chars().nth(col), Some('c'));
    }

    #[test]
    fn wrap_is_char_grid_math() {
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("abcdef", 10), vec!["abcdef"]);
        assert_eq!(wrap("abcdef", 3), vec!["abc", "def"]);
        assert_eq!(wrap("abcdefg", 3), vec!["abc", "def", "g"]);
    }

    #[test]
    fn append_caps_scrollback_and_snaps_to_tail() {
        let id = 900;
        clear(id);
        for i in 0..(MAX_LINES + 10) {
            append(id, Style::Normal, &format!("line {i}"));
        }
        scroll_by(id, 5);
        append(id, Style::Normal, "tail");
        with(id, |c| {
            assert_eq!(c.lines.len(), MAX_LINES, "scrollback must cap");
            assert_eq!(c.scroll, 0, "new output snaps the view to the tail");
            assert_eq!(c.lines.back().unwrap().1, "tail");
            assert!(c.lines.front().unwrap().1.starts_with("line "), "old lines dropped from the front");
        });
        clear(id);
    }

    /// The prelude's line editor counts its cursor in CHARACTERS (Scheme
    /// strings), so a cursor after a multi-byte character is the column
    /// after it — not floored back, as a byte index into `é` would be.
    #[test]
    fn the_input_cursor_is_a_character_index() {
        let id = 913;
        clear(id);
        set_input(id, "> ", "héllo", 2);
        assert_eq!(input(id), ("héllo".to_string(), 2), "after the é, not before it");
        set_input(id, "> ", "héllo", 5);
        assert_eq!(input(id).1, 5, "the end of the line is its length in chars");
        set_input(id, "> ", "héllo", 99);
        assert_eq!(input(id).1, 5, "past the end clamps to it");
    }

    #[test]
    fn draws_transcript_input_and_cursor() {
        let id = 901;
        clear(id);
        let mut fb = Framebuffer::new(300, 120);
        let rect = Rect { x: 0, y: 0, w: 300, h: 120 };
        let theme = crate::theme::get();

        append(id, Style::Normal, "hello transcript");
        set_input(id, "modeler> ", "abc", 3);
        set_active(id, true);
        draw(id, &mut fb, rect, true);

        fn count(fb: &Framebuffer, ink: u32) -> usize {
            fb.pixels.iter().filter(|&&p| p == ink).count()
        }
        assert!(
            fb.pixels.iter().any(|&p| p != 0),
            "pane should paint at all"
        );
        // Background filled, accent cursor block visible (a solid run of
        // pixels in text_accent).
        assert!(count(&fb, theme.panel_bg) > 1000, "panel background fills the pane");
        assert!(count(&fb, theme.text_accent) > 40, "armed cursor block draws in accent");

        // Inactive: the cursor block goes away.
        set_active(id, false);
        draw(id, &mut fb, rect, true);
        assert!(count(&fb, theme.text_accent) < 40, "no cursor block when inactive");
        clear(id);
    }

    #[test]
    fn ui_scale_grows_glyphs() {
        // The strips the host draws grow with it too: the modeler's
        // `ui_scale_grows_the_strips` (`panes.rs`).
        crate::host::testing::reset();
        crate::host::testing::set_ui_scale(2.0);

        // The same content lights roughly 4x the pixels at 2x scale.
        let id = 903;
        let theme = crate::theme::get();
        let lit = |fb: &Framebuffer| {
            fb.pixels.iter().filter(|&&p| p != theme.panel_bg && p != 0).count()
        };
        clear(id);
        set_input(id, "> ", "hello scale", 5);
        let rect = Rect { x: 0, y: 0, w: 300, h: 80 };
        let mut fb2 = Framebuffer::new(300, 80);
        draw(id, &mut fb2, rect, false);
        let big = lit(&fb2);
        crate::host::testing::set_ui_scale(1.0);
        let mut fb1 = Framebuffer::new(300, 80);
        draw(id, &mut fb1, rect, false);
        let small = lit(&fb1);
        assert!(
            big > small * 2,
            "2x scale should light far more pixels ({big} vs {small})"
        );
        clear(id);
    }

    #[test]
    fn scroll_walks_history_and_clamps() {
        let id = 902;
        clear(id);
        let mut fb = Framebuffer::new(200, 60);
        let rect = Rect { x: 0, y: 0, w: 200, h: 60 };
        for i in 0..50 {
            append(id, Style::Normal, &format!("row {i}"));
        }
        scroll_by(id, 10_000);
        draw(id, &mut fb, rect, false);
        with(id, |c| {
            assert!(c.scroll < 60, "scroll clamps to the content, got {}", c.scroll);
            assert!(c.scroll > 0, "clamped scroll still sits above the tail");
        });
        clear(id);
    }
}
