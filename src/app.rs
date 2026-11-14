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

/// What the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Terminal,
    Repl,
}

/// The panel's share of the height under the editor.
const TRANSCRIPT_SHARE: f32 = 0.25;

struct State {
    /// The window, in physical pixels, and the logical-to-physical scale.
    size: (u32, u32),
    scale: f32,
    rects: [Rect; 3],
    focused: u32,
    typing: Option<u32>,
    dirty: bool,
    quit: bool,
    pie_delay_ms: u64,
    /// What the bottom panel shows, or `None` when it is hidden and the
    /// editor has the height.
    panel: Option<Panel>,
    /// Where the strip's two icons were last drawn, for the click.
    icons: [(Panel, Rect); 2],
    /// The panel's share of the height above the minibuffer, which dragging
    /// the divider sets.
    share: f32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        size: (0, 0),
        scale: 1.0,
        rects: [Rect::default(); 3],
        focused: EDITOR,
        typing: None,
        dirty: true,
        quit: false,
        pie_delay_ms: 200,
        panel: Some(Panel::Repl),
        icons: [(Panel::Repl, Rect::default()), (Panel::Terminal, Rect::default())],
        share: TRANSCRIPT_SHARE,
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
    /// The minibuffer reads at the modeline's size — 0.8 of the text —
    /// smaller than the panes, as the modeler's strip is. (The widget draws
    /// a strip pane's mono at 0.94 of this, to sit level with a status
    /// label beside it; the division undoes that.)
    fn strip_font_px(&self, pane: u32) -> Option<f32> {
        (pane == MINIBUFFER).then(|| textpane::font_px() * 0.8 / 0.94)
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
/// text row high at the bottom, the panel (when shown) a quarter of what is
/// above it, the editor the rest.
pub fn resize(w: u32, h: u32, scale: f32) {
    // The minibuffer's strip at its own text size (`strip_font_px`).
    let row = (crate::text::mono_metrics(textpane::font_px() * 0.8 * scale).1 + 10.0 * scale).ceil() as u32;
    let mini = row.min(h);
    let above = h - mini;
    let (shown, share) = with(|s| (s.panel.is_some(), s.share));
    let transcript = if shown { (above as f32 * share).round() as u32 } else { 0 };
    let editor = above - transcript;
    with(|s| {
        s.size = (w, h);
        s.scale = scale;
        s.rects = [
            Rect { x: 0, y: 0, w, h: editor },
            Rect { x: 0, y: editor, w, h: transcript },
            Rect { x: 0, y: above, w, h: mini },
        ];
        s.dirty = true;
    });
}

pub fn rect(id: u32) -> Option<Rect> {
    let i = (id as usize).checked_sub(1)?;
    with(|s| s.rects.get(i).copied()).filter(|r| r.h > 0)
}

/// What a pane is, as `%pane-at` names it: "editor", "term" for the panel
/// while it shows a terminal, or "text" for the REPL's transcript or the
/// minibuffer.
pub fn kind_name(id: u32) -> Option<&'static str> {
    match id {
        EDITOR => Some("editor"),
        TRANSCRIPT if terminal_showing() => Some("term"),
        TRANSCRIPT | MINIBUFFER => Some("text"),
        _ => None,
    }
}

/// The pane under a window point.
pub fn pane_at(x: f64, y: f64) -> Option<u32> {
    [EDITOR, TRANSCRIPT, MINIBUFFER].into_iter().find(|&id| {
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

/// The divider IS the editor's modeline — the bar along the editor's
/// bottom, just above the panel — so it is grabbed anywhere on the bar and
/// a drag moves the bar's centre line to the pointer (Kevin: grab it by its
/// centre, not by its bottom edge). Its centre line, in window pixels.
fn bar_centre() -> Option<(f64, f64)> {
    let top = rect(TRANSCRIPT)?.y as f64;
    let half = editorpane::modeline_height(with(|s| s.scale)) as f64 / 2.0;
    Some((top - half, half))
}

/// Is a window point on the divider between the editor and the panel?
pub fn divider_at(x: f64, y: f64) -> bool {
    let _ = x;
    bar_centre().is_some_and(|(centre, half)| (y - centre).abs() <= half)
}

/// Drag the divider's centre line to window height `y`: the panel takes
/// what is below the bar, kept between a few rows and most of the window.
pub fn drag_divider(y: f64) {
    let Some(mini) = rect(MINIBUFFER) else { return };
    let above = mini.y as f64;
    if above <= 0.0 {
        return;
    }
    let half = editorpane::modeline_height(with(|s| s.scale)) as f64 / 2.0;
    let share = ((above - (y + half)) / above).clamp(0.08, 0.85) as f32;
    let (w, h, scale) = with(|s| {
        s.share = share;
        (s.size.0, s.size.1, s.scale)
    });
    resize(w, h, scale);
}

/// Which of the strip's icons a window point is on.
pub fn icon_at(x: f64, y: f64) -> Option<Panel> {
    with(|s| s.icons.iter().find(|(_, r)| r.w > 0 && r.contains(x, y)).map(|(p, _)| *p))
}

/// The strip's two icons, right-aligned, Zed-style as the modeler draws its
/// dock toggles: the one whose content the panel shows reads in accent over
/// a faint backing, the other dim. The REPL is a λ; the terminal a window
/// with a `>` and a cursor.
fn draw_icons(fb: &mut Framebuffer, strip: Rect, scale: f32, theme: &crate::theme::Theme) {
    let size = (15.0 * scale).round();
    let gap = (5.0 * scale).round();
    // Clear of the window's corner, where the OS keeps its resize handle.
    let margin = (28.0 * scale).round();
    let px = |v: f32| v.max(0.0).round() as u32;
    let y = strip.y as f32 + (strip.h as f32 - size) * 0.5;
    let mut x = (strip.x + strip.w) as f32 - margin - 2.0 * size - gap;
    let shown = panel();
    let mut placed = [(Panel::Repl, Rect::default()), (Panel::Terminal, Rect::default())];
    for (i, which) in [Panel::Repl, Panel::Terminal].into_iter().enumerate() {
        let r = Rect { x: px(x), y: px(y), w: size as u32, h: size as u32 };
        let on = shown == Some(which);
        let ink = if on { theme.text_accent } else { theme.text_dim };
        if on {
            fb.round_rect(r, 3.0 * scale, theme.panel_bg, theme.panel_bg);
        }
        let pad = (3.0 * scale).max(2.0);
        let (ix, iy, iw, ih) = (r.x as f32 + pad, r.y as f32 + pad, r.w as f32 - 2.0 * pad, r.h as f32 - 2.0 * pad);
        match which {
            Panel::Repl => {
                let lpx = ih * 1.25;
                let adv = crate::text::mono_metrics(lpx).0;
                crate::text::draw_mono_line(fb, ix + (iw - adv) * 0.5, iy - ih * 0.2, lpx, ink, "λ");
            }
            Panel::Terminal => {
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

/// Paint every pane, then whatever floats over them (a popup, a menu).
pub fn draw(fb: &mut Framebuffer) {
    let theme = crate::theme::Theme::default();
    fb.fill_rect(Rect { x: 0, y: 0, w: fb.width, h: fb.height }, theme.panel_bg);
    let focused = focused();
    for id in [EDITOR, TRANSCRIPT, MINIBUFFER] {
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
    // A rule over the minibuffer, so the echo line reads as its own strip
    // and not the transcript's last row.
    if let Some(r) = rect(MINIBUFFER) {
        let h = with(|s| s.scale).round().max(1.0) as u32;
        fb.fill_rect(Rect { x: r.x, y: r.y, w: r.w, h }, theme.status_bg);
        draw_icons(fb, r, with(|s| s.scale), &theme);
    }
    textpane::draw_overlay(fb);
}
