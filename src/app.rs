//! **The standalone program's window-free half** (docs/editor-crate-plan.md,
//! E4): its three panes and where they are, which one has the keyboard, and
//! drawing them into a framebuffer. `bin/edit.rs` is the window around it,
//! and `scm_binding::standalone` the Scheme session; this module names
//! neither winit nor the VM, so a test drives it with no window at all.
//!
//! The layout is fixed — the editor over the bottom PANEL over a minibuffer
//! — and the panel shows a terminal or the REPL (the transcript with a
//! prompt), never both, or is hidden: the two icons at the strip's right end
//! choose, as the modeler's dock toggles do.
//! rather than the modeler's docking `panes.rs`, which is tangled with its
//! 3D panes. A shared layout is extracted only if the two start to
//! duplicate each other.

use std::cell::RefCell;

use crate::framebuffer::{Framebuffer, Rect};
use crate::{editorpane, textpane};

/// The panes, by id. The shell asks for them by key through `*pane-id*`.
pub const EDITOR: u32 = 1;
pub const TRANSCRIPT: u32 = 2;
pub const MINIBUFFER: u32 = 3;
/// The file tree, a column on the left (`17-files.scm`).
pub const FILES: u32 = 4;

/// What the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Terminal,
    Repl,
}

/// A strip icon: the file tree's, or one of the bottom panel's two.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Files,
    Repl,
    Terminal,
}

/// A draggable edge: the bar between the editor and the panel, or the edge
/// between the file tree and the editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Divider {
    Panel,
    Files,
}

/// The file tree's share of the window's width, while it shows.
const FILES_SHARE: f32 = 0.2;

/// The minibuffer's text size, as a share of the panes'.
const MINI_SHARE: f32 = 0.88;

/// The panel's share of the height under the editor.
const TRANSCRIPT_SHARE: f32 = 0.25;

struct State {
    /// The window, in physical pixels, and the logical-to-physical scale.
    size: (u32, u32),
    scale: f32,
    rects: [Rect; 4],
    focused: u32,
    typing: Option<u32>,
    dirty: bool,
    quit: bool,
    pie_delay_ms: u64,
    /// What the bottom panel shows, or `None` when it is hidden and the
    /// editor has the height.
    panel: Option<Panel>,
    /// Where the strip's icons were last drawn, for the click.
    icons: [(Icon, Rect); 3],
    /// The panel's share of the height above the minibuffer, which dragging
    /// the divider sets.
    share: f32,
    /// Whether the file tree shows, and its share of the width.
    files: bool,
    files_share: f32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        size: (0, 0),
        scale: 1.0,
        rects: [Rect::default(); 4],
        focused: EDITOR,
        typing: None,
        dirty: true,
        quit: false,
        pie_delay_ms: 200,
        // Hidden at startup: the editor has the height until the panel is
        // asked for (an icon, C-c C-z, running a file).
        panel: None,
        icons: [(Icon::Files, Rect::default()), (Icon::Repl, Rect::default()), (Icon::Terminal, Rect::default())],
        share: TRANSCRIPT_SHARE,
        files: true,
        files_share: FILES_SHARE,
    });
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// The widget's host: answers from [`STATE`].
#[cfg_attr(test, allow(dead_code))]
struct AppHost;

impl crate::host::Host for AppHost {
    fn mark_dirty(&self) {
        mark_dirty();
    }
    fn mark_dirty_ui(&self) {
        mark_dirty();
    }
    fn rect(&self, pane: u32) -> Option<Rect> {
        rect(pane)
    }
    fn ui_scale(&self) -> f32 {
        with(|s| s.scale)
    }
    fn typing(&self) -> Option<u32> {
        typing()
    }
    fn window(&self) -> (u32, u32) {
        with(|s| s.size)
    }
    /// The minibuffer reads a little smaller than the panes, as the
    /// modeler's strip does: MINI_SHARE of the text (a touch above the
    /// modeline's 0.8 — Kevin, 2026-11-13). (The widget draws a strip
    /// pane's mono at 0.94 of this, to sit level with a status label beside
    /// it; the division undoes that.)
    fn strip_font_px(&self, pane: u32) -> Option<f32> {
        (pane == MINIBUFFER).then(|| textpane::font_px() * MINI_SHARE / 0.94)
    }
    fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::default()
    }
}

/// Make this program the widget's host. Once, before anything draws.
///
/// Not under `cargo test`: the host is PROCESS-wide, and the widget's own
/// tests, on other threads of the same process, answer through the test
/// host (`host::testing`) — installing this one mid-run took it from under
/// them. The standalone's tests read `app`'s state directly instead.
pub fn install_host() {
    #[cfg(not(test))]
    crate::host::install(&AppHost);
}

/// Lay the panes out in a `w`×`h` window at `scale`: the minibuffer one
/// text row high across the bottom; above it the file tree (when shown) a
/// column on the left, and right of that the panel (when shown) under the
/// editor.
pub fn resize(w: u32, h: u32, scale: f32) {
    // The minibuffer's strip at its own text size (`strip_font_px`).
    let row = (crate::text::mono_metrics(textpane::font_px() * MINI_SHARE * scale).1 + 10.0 * scale).ceil() as u32;
    let mini = row.min(h);
    let above = h - mini;
    let (shown, share, files, files_share) = with(|s| (s.panel.is_some(), s.share, s.files, s.files_share));
    let transcript = if shown { (above as f32 * share).round() as u32 } else { 0 };
    let editor = above - transcript;
    let left = if files { (w as f32 * files_share).round() as u32 } else { 0 };
    let right = w - left;
    with(|s| {
        s.size = (w, h);
        s.scale = scale;
        s.rects = [
            Rect { x: left, y: 0, w: right, h: editor },
            Rect { x: left, y: editor, w: right, h: transcript },
            Rect { x: 0, y: above, w, h: mini },
            Rect { x: 0, y: 0, w: left, h: above },
        ];
        s.dirty = true;
    });
    // Under cargo test the program's host is not installed (install_host),
    // so the widget asks the TEST host: give it this layout, or a popup
    // lands at a zero-size window's corner and a pane cannot find its rows.
    #[cfg(test)]
    {
        crate::host::testing::set_window(w, h);
        for id in [EDITOR, TRANSCRIPT, MINIBUFFER, FILES] {
            match rect(id) {
                Some(r) => crate::host::testing::set_rect(id, r),
                None => crate::host::testing::clear_rect(id),
            }
        }
    }
}

pub fn rect(id: u32) -> Option<Rect> {
    let i = (id as usize).checked_sub(1)?;
    with(|s| s.rects.get(i).copied()).filter(|r| r.h > 0 && r.w > 0)
}

/// What a pane is, as `%pane-at` names it: "editor", "term" for the panel
/// while it shows a terminal, or "text" for the REPL's transcript or the
/// minibuffer.
pub fn kind_name(id: u32) -> Option<&'static str> {
    match id {
        EDITOR => Some("editor"),
        TRANSCRIPT if terminal_showing() => Some("term"),
        TRANSCRIPT | MINIBUFFER | FILES => Some("text"),
        _ => None,
    }
}

/// The pane under a window point.
pub fn pane_at(x: f64, y: f64) -> Option<u32> {
    [EDITOR, TRANSCRIPT, MINIBUFFER, FILES].into_iter().find(|&id| {
        rect(id).is_some_and(|r| {
            x >= r.x as f64 && y >= r.y as f64 && x < (r.x + r.w) as f64 && y < (r.y + r.h) as f64
        })
    })
}

pub fn focused() -> u32 {
    with(|s| s.focused)
}

pub fn focus(id: u32) {
    if kind_name(id).is_some() {
        with(|s| {
            s.focused = id;
            s.dirty = true;
        });
    }
}

pub fn typing() -> Option<u32> {
    with(|s| s.typing)
}

/// Give `id` the keyboard, and the focus with it — as the modeler's
/// `panes::hold_typing` does: the keymap reads the FOCUSED pane's local map,
/// so a prompt that held the keyboard without the focus never got its own
/// backspace or C-g, nor drew its cursor. A text pane shows its cursor while
/// it holds the keyboard.
pub fn hold_typing(id: u32) {
    focus(id);
    if let Some(old) = typing() {
        textpane::set_active(old, false);
    }
    if kind_name(id) == Some("text") {
        textpane::set_active(id, true);
    }
    with(|s| {
        s.typing = Some(id);
        s.dirty = true;
    });
}

pub fn release_typing() {
    if let Some(old) = typing() {
        textpane::set_active(old, false);
    }
    with(|s| {
        s.typing = None;
        s.dirty = true;
    });
}

pub fn mark_dirty() {
    with(|s| s.dirty = true);
}

/// Whether anything changed since the last call — the window redraws then.
pub fn take_dirty() -> bool {
    with(|s| std::mem::replace(&mut s.dirty, false))
}

/// `C-x C-c` asks; the window loop reads it.
pub fn request_quit() {
    with(|s| s.quit = true);
}

pub fn quit_requested() -> bool {
    with(|s| s.quit)
}

/// The marking menu's rehearsal delay, which the shell sets.
pub fn set_pie_delay(ms: u64) {
    with(|s| s.pie_delay_ms = ms);
}

pub fn pie_delay_ms() -> u64 {
    with(|s| s.pie_delay_ms)
}

/// Show the panel with `panel` in it, or hide it (`None`); the layout
/// follows. A terminal's program runs on while the panel is hidden or shows
/// the REPL.
pub fn show_panel(panel: Option<Panel>) {
    let (w, h, scale) = with(|s| {
        s.panel = panel;
        (s.size.0, s.size.1, s.scale)
    });
    resize(w, h, scale);
}

pub fn panel() -> Option<Panel> {
    with(|s| s.panel)
}

/// Is the panel showing a running terminal?
pub fn terminal_showing() -> bool {
    panel() == Some(Panel::Terminal) && crate::termpane::is_open(TRANSCRIPT)
}

/// The panel's divider IS the editor's modeline — the bar along the
/// editor's bottom, just above the panel — so it is grabbed anywhere on the
/// bar and a drag moves the bar's centre line to the pointer (Kevin: grab it
/// by its centre, not by its bottom edge). Its centre line, in window pixels.
fn bar_centre() -> Option<(f64, f64)> {
    let top = rect(TRANSCRIPT)?.y as f64;
    let half = editorpane::modeline_height(with(|s| s.scale)) as f64 / 2.0;
    Some((top - half, half))
}

/// Half the width of the file tree's edge, as grabbed, in logical pixels.
const EDGE_GRAB: f64 = 4.0;

/// The divider a window point is on, if any: the panel's bar (across the
/// editor's width), or the file tree's right edge.
pub fn divider_at(x: f64, y: f64) -> Option<Divider> {
    if let Some(f) = rect(FILES) {
        let grab = EDGE_GRAB * with(|s| s.scale) as f64;
        if (x - (f.x + f.w) as f64).abs() <= grab && y < (f.y + f.h) as f64 {
            return Some(Divider::Files);
        }
    }
    let on_bar = bar_centre().is_some_and(|(centre, half)| (y - centre).abs() <= half);
    let over_editor = rect(EDITOR).is_some_and(|e| x >= e.x as f64);
    (on_bar && over_editor).then_some(Divider::Panel)
}

/// Drag a divider to the pointer: the panel's bar's centre line to `y`
/// (8–85% of the height), or the file tree's edge to `x` (8–50% of the
/// width).
pub fn drag_divider(d: Divider, x: f64, y: f64) {
    let (w, h, scale) = with(|s| (s.size.0, s.size.1, s.scale));
    match d {
        Divider::Panel => {
            let Some(mini) = rect(MINIBUFFER) else { return };
            let above = mini.y as f64;
            if above <= 0.0 {
                return;
            }
            let half = editorpane::modeline_height(scale) as f64 / 2.0;
            let share = ((above - (y + half)) / above).clamp(0.08, 0.85) as f32;
            with(|s| s.share = share);
        }
        Divider::Files => {
            if w == 0 {
                return;
            }
            let share = (x / w as f64).clamp(0.08, 0.5) as f32;
            with(|s| s.files_share = share);
        }
    }
    resize(w, h, scale);
}

/// Start a mouse selection at a window point, in the REPL's transcript or
/// the terminal, whichever the panel shows: the pane it started in, or
/// `None` (the point is elsewhere — the press is an ordinary click).
pub fn begin_select(x: f64, y: f64) -> Option<u32> {
    let id = pane_at(x, y)?;
    match kind_name(id) {
        Some("term") => crate::termpane::begin_select(id, x, y).then_some(id),
        Some("text") if id == TRANSCRIPT => textpane::begin_select(id, x, y).then_some(id),
        _ => None,
    }
}

/// Carry a selection begun in pane `id` to a window point.
pub fn extend_select(id: u32, x: f64, y: f64) {
    if kind_name(id) == Some("term") {
        crate::termpane::extend_select(id, x, y);
    } else {
        textpane::extend_select(id, x, y);
    }
}

/// Show or hide the file tree; the layout follows.
pub fn show_files(on: bool) {
    let (w, h, scale) = with(|s| {
        s.files = on;
        (s.size.0, s.size.1, s.scale)
    });
    resize(w, h, scale);
}

pub fn files_shown() -> bool {
    with(|s| s.files)
}

/// Which of the strip's icons a window point is on.
pub fn icon_at(x: f64, y: f64) -> Option<Icon> {
    with(|s| s.icons.iter().find(|(_, r)| r.w > 0 && r.contains(x, y)).map(|(i, _)| *i))
}

/// The strip's icons, right-aligned, Zed-style as the modeler draws its
/// dock toggles: lit in accent over a faint backing while what it shows is
/// shown, dim otherwise. The file tree is a folder; the REPL a λ; the
/// terminal a window with a `>` and a cursor.
fn draw_icons(fb: &mut Framebuffer, strip: Rect, scale: f32, theme: &crate::theme::Theme) {
    let size = (15.0 * scale).round();
    let gap = (5.0 * scale).round();
    // Clear of the window's corner, where the OS keeps its resize handle.
    let margin = (28.0 * scale).round();
    let px = |v: f32| v.max(0.0).round() as u32;
    let y = strip.y as f32 + (strip.h as f32 - size) * 0.5;
    let mut x = (strip.x + strip.w) as f32 - margin - 3.0 * size - 2.0 * gap;
    let shown = panel();
    let mut placed = [(Icon::Files, Rect::default()), (Icon::Repl, Rect::default()), (Icon::Terminal, Rect::default())];
    for (i, which) in [Icon::Files, Icon::Repl, Icon::Terminal].into_iter().enumerate() {
        let r = Rect { x: px(x), y: px(y), w: size as u32, h: size as u32 };
        let on = match which {
            Icon::Files => files_shown(),
            Icon::Repl => shown == Some(Panel::Repl),
            Icon::Terminal => shown == Some(Panel::Terminal),
        };
        let ink = if on { theme.text_accent } else { theme.text_dim };
        if on {
            fb.round_rect(r, 3.0 * scale, theme.panel_bg, theme.panel_bg);
        }
        let pad = (3.0 * scale).max(2.0);
        let (ix, iy, iw, ih) = (r.x as f32 + pad, r.y as f32 + pad, r.w as f32 - 2.0 * pad, r.h as f32 - 2.0 * pad);
        match which {
            Icon::Files => {
                // A folder: a body, and a tab on its top left.
                let tab_h = (ih * 0.22).max(1.0);
                fb.fill_rect(Rect { x: px(ix), y: px(iy), w: px(iw * 0.45), h: px(tab_h) }, ink);
                fb.stroke_rect(Rect { x: px(ix), y: px(iy + tab_h), w: px(iw), h: px(ih - tab_h) }, ink);
            }
            Icon::Repl => {
                let lpx = ih * 1.25;
                let adv = crate::text::mono_metrics(lpx).0;
                crate::text::draw_mono_line(fb, ix + (iw - adv) * 0.5, iy - ih * 0.2, lpx, ink, "λ");
            }
            Icon::Terminal => {
                fb.stroke_rect(Rect { x: px(ix), y: px(iy), w: px(iw), h: px(ih) }, ink);
                let (cx, cy, ch, cw) = (ix + 2.5 * scale, iy + ih * 0.5, ih * 0.2, 2.0 * scale);
                crate::framebuffer::draw_line_2d(cx, cy - ch, cx + cw, cy, ink, fb, r);
                crate::framebuffer::draw_line_2d(cx + cw, cy, cx, cy + ch, ink, fb, r);
                fb.fill_rect(
                    Rect { x: px(cx + cw + 1.5 * scale), y: px(cy - 0.75 * scale), w: px(3.0 * scale), h: px(1.5 * scale) },
                    ink,
                );
            }
        }
        placed[i] = (which, r);
        x += size + gap;
    }
    with(|s| s.icons = placed);
}

/// The character a file-tree row holds where a folder icon goes: a
/// private-use code point no face draws, so the cell stays blank under the
/// icon. Two of them make the icon's width.
pub const FOLDER_MARK: char = '\u{E000}';

/// A little folder over each FOLDER_MARK pair in the file tree's rows.
fn draw_folders(fb: &mut Framebuffer, pane: Rect, theme: &crate::theme::Theme) {
    let scale = with(|s| s.scale);
    let px = textpane::font_px() * scale;
    let (adv, row_h) = crate::text::mono_metrics(px);
    let pad = textpane::PAD * scale;
    let ink = theme.code_number;
    let at = |v: f32| v.max(0.0).round() as u32;
    for (y, text) in textpane::laid_rows(FILES) {
        let Some(col) = text.chars().position(|c| c == FOLDER_MARK) else { continue };
        let x = pane.x as f32 + pad + col as f32 * adv;
        let (w, h) = (adv * 1.7, row_h * 0.5);
        let (x0, y0) = (x + adv * 0.1, y + (row_h - h) * 0.55);
        let tab_h = (h * 0.25).max(1.0);
        fb.fill_rect(Rect { x: at(x0), y: at(y0), w: at(w * 0.45), h: at(tab_h) }, ink);
        fb.fill_rect(Rect { x: at(x0), y: at(y0 + tab_h), w: at(w), h: at(h - tab_h) }, ink);
    }
}

/// Paint every pane, then whatever floats over them (a popup, a menu).
pub fn draw(fb: &mut Framebuffer) {
    let theme = crate::theme::Theme::default();
    fb.fill_rect(Rect { x: 0, y: 0, w: fb.width, h: fb.height }, theme.panel_bg);
    let focused = focused();
    for id in [EDITOR, TRANSCRIPT, MINIBUFFER, FILES] {
        let Some(r) = rect(id) else { continue };
        let lit = focused == id;
        if id == EDITOR {
            editorpane::draw(id, fb, r, lit);
        } else if id == TRANSCRIPT && terminal_showing() {
            crate::termpane::draw(id, fb, r, lit);
        } else {
            textpane::draw(id, fb, r, lit);
        }
    }
    // The file tree's folder icons, drawn over the one-cell placeholders its
    // rows carry (17-files): the mono face has no folder glyph.
    if let Some(f) = rect(FILES) {
        draw_folders(fb, f, &theme);
    }
    // A rule down the file tree's edge, so the tree reads as its own column.
    if let Some(f) = rect(FILES) {
        let w = with(|s| s.scale).round().max(1.0) as u32;
        fb.fill_rect(Rect { x: f.x + f.w - w, y: f.y, w, h: f.h }, theme.status_bg);
    }
    // A rule over the minibuffer, so the echo line reads as its own strip
    // and not the transcript's last row.
    if let Some(r) = rect(MINIBUFFER) {
        let h = with(|s| s.scale).round().max(1.0) as u32;
        fb.fill_rect(Rect { x: r.x, y: r.y, w: r.w, h }, theme.status_bg);
        draw_icons(fb, r, with(|s| s.scale), &theme);
    }
    textpane::draw_overlay(fb);
}
