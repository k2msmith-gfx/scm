//! **What the editor asks of the program it lives in.** The whole surface
//! the widget reached the modeler through, before E1 moved it out: two dirty
//! flags, a pane's rect, the UI scale, which pane has the keyboard, the
//! window's size, the bottom strip's text size, and the colours.
//!
//! A host installs itself for the process ([`install`]): the modeler when
//! its pane registry first holds a pane, so its tests, which never run
//! `main`, get it too. The host is stateless — it answers from the program's
//! own (thread-local) state — which is why one installation serves every
//! thread. Until a host is installed the answers are a detached widget's: no
//! rects, scale 1, nothing focused, the default colours, and dirt that goes
//! nowhere.

use std::sync::RwLock;

use crate::framebuffer::Rect;
use crate::theme::Theme;

pub trait Host: Sync {
    /// The screen is stale, overlays included: anything that floats over
    /// another pane (the popup, a menu) must mark this.
    fn mark_dirty(&self);
    /// Only a text pane's own contents changed — the keystroke fast path,
    /// which may redraw text panes without redrawing what is under them.
    fn mark_dirty_ui(&self);
    /// Where a pane is, in physical pixels; `None` for an unknown id.
    fn rect(&self, pane: u32) -> Option<Rect>;
    /// Logical-to-physical pixels (2.0 on retina). Every logical size in the
    /// widget multiplies by this at draw time.
    fn ui_scale(&self) -> f32;
    /// The pane that has the keyboard, if any.
    fn typing(&self) -> Option<u32>;
    /// The window's size in physical pixels — where a popup may go.
    fn window(&self) -> (u32, u32);
    /// The LOGICAL text size a pane shares with the host's own chrome, when
    /// it is not the reading size: the modeler's minibuffer sits in one strip
    /// with the status label and draws at its size. `None` for every other
    /// pane.
    fn strip_font_px(&self, pane: u32) -> Option<f32>;
    /// The colours the widget draws in.
    fn theme(&self) -> Theme;
}

/// The answers before a host is installed.
struct Detached;

impl Host for Detached {
    fn mark_dirty(&self) {}
    fn mark_dirty_ui(&self) {}
    fn rect(&self, _pane: u32) -> Option<Rect> {
        None
    }
    fn ui_scale(&self) -> f32 {
        1.0
    }
    fn typing(&self) -> Option<u32> {
        None
    }
    fn window(&self) -> (u32, u32) {
        (0, 0)
    }
    fn strip_font_px(&self, _pane: u32) -> Option<f32> {
        None
    }
    fn theme(&self) -> Theme {
        Theme::default()
    }
}

static HOST: RwLock<&'static dyn Host> = RwLock::new(&Detached);

/// Make `host` the one the widget asks. Installing the same host again is
/// harmless, so a host may do it wherever it is sure to have run.
pub fn install(host: &'static dyn Host) {
    *HOST.write().unwrap_or_else(|e| e.into_inner()) = host;
}

/// The host, copied out before the call, so one that calls back into the
/// widget never meets the lock.
fn with<R>(f: impl FnOnce(&dyn Host) -> R) -> R {
    let host = *HOST.read().unwrap_or_else(|e| e.into_inner());
    f(host)
}

pub fn mark_dirty() {
    with(|h| h.mark_dirty())
}

pub fn mark_dirty_ui() {
    with(|h| h.mark_dirty_ui())
}

pub fn rect(pane: u32) -> Option<Rect> {
    with(|h| h.rect(pane))
}

pub fn ui_scale() -> f32 {
    with(|h| h.ui_scale())
}

pub fn typing() -> Option<u32> {
    with(|h| h.typing())
}

pub fn window() -> (u32, u32) {
    with(|h| h.window())
}

pub fn strip_font_px(pane: u32) -> Option<f32> {
    with(|h| h.strip_font_px(pane))
}

pub fn theme() -> Theme {
    with(|h| h.theme())
}

/// A host for the crate's own tests: rects and a scale the test sets, the
/// default colours. Tests run one per thread and the state is per thread,
/// so they never see each other's.
#[cfg(test)]
pub mod testing {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use super::*;

    thread_local! {
        static RECTS: RefCell<HashMap<u32, Rect>> = RefCell::new(HashMap::new());
        static SCALE: Cell<f32> = const { Cell::new(1.0) };
        static TYPING: Cell<Option<u32>> = const { Cell::new(None) };
        static WINDOW: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
    }

    struct TestHost;

    impl Host for TestHost {
        fn mark_dirty(&self) {}
        fn mark_dirty_ui(&self) {}
        fn rect(&self, pane: u32) -> Option<Rect> {
            RECTS.with(|r| r.borrow().get(&pane).copied())
        }
        fn ui_scale(&self) -> f32 {
            SCALE.with(|s| s.get())
        }
        fn typing(&self) -> Option<u32> {
            TYPING.with(|t| t.get())
        }
        fn window(&self) -> (u32, u32) {
            WINDOW.with(|w| w.get())
        }
        fn strip_font_px(&self, _pane: u32) -> Option<f32> {
            None
        }
        fn theme(&self) -> Theme {
            Theme::default()
        }
    }

    /// The window's size, where a popup may go.
    pub fn set_window(w: u32, h: u32) {
        install(&TestHost);
        WINDOW.with(|c| c.set((w, h)));
    }

    /// Forget a pane's rect (a hidden pane).
    pub fn clear_rect(pane: u32) {
        install(&TestHost);
        RECTS.with(|r| r.borrow_mut().remove(&pane));
    }

    /// Install the test host on this thread, with no rects and scale 1.
    pub fn reset() {
        RECTS.with(|r| r.borrow_mut().clear());
        SCALE.with(|s| s.set(1.0));
        TYPING.with(|t| t.set(None));
        WINDOW.with(|w| w.set((0, 0)));
        install(&TestHost);
    }

    pub fn set_rect(pane: u32, rect: Rect) {
        install(&TestHost);
        RECTS.with(|r| r.borrow_mut().insert(pane, rect));
    }

    pub fn set_ui_scale(scale: f32) {
        install(&TestHost);
        SCALE.with(|s| s.set(scale));
    }

    pub fn set_typing(pane: Option<u32>) {
        install(&TestHost);
        TYPING.with(|t| t.set(pane));
    }
}
