//! **Filming the standalone editor** (Kevin, 2026-11-15: "a demo movie of
//! the editor"). A script drives `edit` the way the modeler's demo films
//! drive the modeler (ray's `modeler/tools/demo_loft.py`): forms over a
//! socket, input injected at a human pace, and the editor's OWN framebuffer
//! saved per frame — pixel-exact, and of the application rather than of a
//! desktop it happens to be on.
//!
//! The prims (`scm_binding/film.rs`) only QUEUE what they ask for: the pointer and the keyboard
//! are the program's (`bin/edit.rs` holds the pointer, the held button, the
//! marking menu's timer), so the program applies the queue after each form,
//! through the same code a real event takes — a film that took a shortcut
//! round the event handling would show something the editor does not do.
//! Installed only in film mode (`EDIT_FILM_PORT`): the modeler has its own
//! prims by these names.

use std::cell::RefCell;

/// One thing a film asks of the program.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A key by its descriptor (`"C-x"`, `"return"`, `"a"`).
    Key(String),
    /// `"move" x y`, `"press" button`, `"release" button`, `"wheel" notches`,
    /// `"shift" / "ctrl" / "alt" 0|1` — buttons 0 left, 2 right.
    Mouse(String, f64, f64),
    /// Save the frame as it is now to a PNG at this path.
    Shot(String),
    /// The caption under the editor, or none.
    Caption(Option<String>),
}

thread_local! {
    static OPS: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

/// Queue OP for the program (the prims are `scm_binding/film.rs`).
pub fn push(op: Op) {
    OPS.with(|o| o.borrow_mut().push(op));
}

/// What the forms just run asked for, in order.
pub fn take() -> Vec<Op> {
    OPS.with(|o| std::mem::take(&mut *o.borrow_mut()))
}
