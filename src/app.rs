//! **The standalone program's window-free half** (docs/editor-crate-plan.md,
//! E4): its three panes and where they are, which one has the keyboard, and
//! drawing them into a framebuffer. `bin/edit.rs` is the window around it,
//! and `scm_binding::standalone` the Scheme session; this module names
//! neither winit nor the VM, so a test drives it with no window at all.
//!
//! The layout is fixed — the editor over a transcript over a minibuffer —
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

/// The transcript's share of the height under the editor.
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
    /// Whether the lower pane shows the terminal (when one runs) rather
    /// than the transcript — what the strip's terminal icon toggles.
    term_shown: bool,
    /// Where that icon was last drawn, for the click.
    toggle: Rect,
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
        term_shown: false,
        toggle: Rect::default(),
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
    fn strip_font_px(&self, _pane: u32) -> Option<f32> {
        None
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
/// text row high at the bottom, the transcript a quarter of what is above
/// it, the editor the rest.
pub fn resize(w: u32, h: u32, scale: f32) {
    let row = (crate::text::mono_metrics(textpane::font_px() * scale).1 + 8.0 * scale).ceil() as u32;
    let mini = row.min(h);
    let above = h - mini;
    let transcript = (above as f32 * TRANSCRIPT_SHARE).round() as u32;
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
    with(|s| s.rects.get(i).copied())
}

/// What a pane is, as `%pane-at` names it: "editor", "term" for the lower
/// pane while it shows a terminal, or "text" for a transcript or the
/// minibuffer.
pub fn kind_name(id: u32) -> Option<&'static str> {
    match id {
        EDITOR => Some("editor"),
        // The lower pane is the transcript until a terminal opens in it (E5).
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

/// Give `id` the keyboard: a text pane shows its cursor while it holds it.
pub fn hold_typing(id: u32) {
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

/// Show the terminal in the lower pane, or the transcript. The program
/// keeps running either way.
pub fn show_terminal(on: bool) {
    with(|s| {
        s.term_shown = on;
        s.dirty = true;
    });
}

/// Is the lower pane showing a running terminal?
pub fn terminal_showing() -> bool {
    with(|s| s.term_shown) && crate::termpane::is_open(TRANSCRIPT)
}

/// Is a window point on the strip's terminal icon?
pub fn toggle_at(x: f64, y: f64) -> bool {
    with(|s| s.toggle.w > 0 && s.toggle.contains(x, y))
}

/// The terminal icon, right-aligned in the minibuffer strip, Zed-style as
/// the modeler draws its panel toggles: accent over a faint backing while
/// the terminal shows, dim otherwise. A window outline, a `>` and a cursor.
fn draw_toggle(fb: &mut Framebuffer, strip: Rect, scale: f32, theme: &crate::theme::Theme) {
    let size = (15.0 * scale).round();
    let margin = (8.0 * scale).round();
    let x = (strip.x + strip.w) as f32 - margin - size;
    let y = strip.y as f32 + (strip.h as f32 - size) * 0.5;
    let r = Rect { x: x.max(0.0) as u32, y: y.max(0.0) as u32, w: size as u32, h: size as u32 };
    let on = terminal_showing();
    let ink = if on { theme.text_accent } else { theme.text_dim };
    if on {
        fb.round_rect(r, 3.0 * scale, theme.panel_bg, theme.panel_bg);
    }
    let pad = (3.0 * scale).max(2.0);
    let (ix, iy, iw, ih) = (r.x as f32 + pad, r.y as f32 + pad, r.w as f32 - 2.0 * pad, r.h as f32 - 2.0 * pad);
    let px = |v: f32| v.max(0.0).round() as u32;
    fb.stroke_rect(Rect { x: px(ix), y: px(iy), w: px(iw), h: px(ih) }, ink);
    let (cx, cy, ch, cw) = (ix + 2.5 * scale, iy + ih * 0.5, ih * 0.2, 2.0 * scale);
    crate::framebuffer::draw_line_2d(cx, cy - ch, cx + cw, cy, ink, fb, r);
    crate::framebuffer::draw_line_2d(cx + cw, cy, cx, cy + ch, ink, fb, r);
    fb.fill_rect(Rect { x: px(cx + cw + 1.5 * scale), y: px(cy - 0.75 * scale), w: px(3.0 * scale), h: px(1.5 * scale) }, ink);
    with(|s| s.toggle = r);
}

/// Paint every pane, then whatever floats over them (a popup, a menu).
pub fn draw(fb: &mut Framebuffer) {
    let theme = crate::theme::Theme::default();
    fb.fill_rect(Rect { x: 0, y: 0, w: fb.width, h: fb.height }, theme.panel_bg);
    let focused = focused();
    for id in [EDITOR, TRANSCRIPT, MINIBUFFER] {
        let Some(r) = rect(id) else { continue };
        if id == EDITOR {
            editorpane::draw(id, fb, r, focused == id);
        } else if id == TRANSCRIPT && terminal_showing() {
            crate::termpane::draw(id, fb, r, focused == id);
        } else {
            textpane::draw(id, fb, r, focused == id);
        }
    }
    // A rule over the minibuffer, so the echo line reads as its own strip
    // and not the transcript's last row.
    if let Some(r) = rect(MINIBUFFER) {
        let h = with(|s| s.scale).round().max(1.0) as u32;
        fb.fill_rect(Rect { x: r.x, y: r.y, w: r.w, h }, theme.status_bg);
        draw_toggle(fb, r, with(|s| s.scale), &theme);
    }
    textpane::draw_overlay(fb);
}
