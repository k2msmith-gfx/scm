//! The system clipboard, for the shell's copy, cut, paste and kill
//! (docs/editor-crate-plan.md, E3c) — moved from the modeler's
//! `script/mesh.rs`, where it had landed beside nothing it was about.

use std::cell::RefCell;

thread_local! {
    /// The system clipboard handle, created on first use (main thread —
    /// macOS pasteboard requirement, which the eval spine satisfies).
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

fn with_clipboard<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Option<R> {
    CLIPBOARD.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = arboard::Clipboard::new().ok();
        }
        c.as_mut().map(f)
    })
}

prim! {
    /// `(%clipboard)` — the system clipboard's text, or nil.
    /// `(%set-clipboard str)` — set it.
    "%clipboard", fn clipboard_get() {
        match with_clipboard(|c| c.get_text().ok()).flatten() {
            Some(t) => Out::Text(t),
            None => Out::Nil,
        }

    }
}

prim! {
    "%set-clipboard", fn clipboard_set(a0: opt_text) {
        if let Some(text) = a0
            && let Some(Err(e)) = with_clipboard(|c| c.set_text(text))
        {
            eprintln!("[clipboard] {e}");
        }
        Out::Nil
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        clipboard_get::install(interp)?,
        clipboard_set::install(interp)?,
    ])
}
