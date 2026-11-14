//! The text-pane prims (`docs/modeler-plan.md` M4.5) — lifted out of
//! `script.rs` whole (A4).
//!
//! A text pane is rows of text the prelude appends to and scrolls; the
//! outliner, the console and the help pane are all one.

// ── Text-pane primitives (docs/modeler-plan.md M4.5) ────────────────────────
// The kernel half of the transcript component: content in, pixels out.
// What the lines say, how the input line edits, when panes hold typing
// focus — all Scheme (the prelude's REPL section).
//
// THE FIRST MODULE THROUGH THE `prim!` MACRO (modeler-port-plan M1): one
// declaration each, a body written against neither VM, and a wrapper
// generated for both. The bodies are unchanged — this is the same
// arithmetic, with the argument reading and the answer wrapping lifted out.

prim! {
    /// `(%pane-append id style text)` — append transcript lines (split on
    /// newlines). Styles: "normal", "dim", "accent", "error".
    "%pane-append", fn pane_append(id: num, style: text, text: opt_text) {
        let style = crate::textpane::Style::parse(&style)
            .unwrap_or(crate::textpane::Style::Normal);
        if let Some(t) = text {
            crate::textpane::append(id as u32, style, &t);
        }
        Out::Nil
    }
}

prim! {
    /// `(%pane-clear id)` — empty a pane's transcript.
    "%pane-clear", fn pane_clear(id: num) {
        crate::textpane::clear(id as u32);
        Out::Nil
    }
}

prim! {
    /// `(%pane-scroll id rows)` — scroll the transcript by wrapped rows
    /// (positive = back in time; 0 just reads). Returns the offset in effect.
    "%pane-scroll", fn pane_scroll(id: num, rows: num) {
        Out::Num(crate::textpane::scroll_by(id as u32, rows as i32) as f64)
    }
}

// Row-level outliner edits (perf item 7): the incremental refresh touches
// individual rows instead of clearing and re-appending every object.

prim! {
    /// `(%pane-row-count id)` — how many rows the pane holds.
    "%pane-row-count", fn pane_row_count(id: num) {
        Out::Num(crate::textpane::row_count(id as u32) as f64)
    }
}

prim! {
    /// `(%pane-row-text id idx)` — a row's text, or nil if out of range.
    "%pane-row-text", fn pane_row_text(id: num, idx: num) {
        match crate::textpane::row_text(id as u32, idx.max(0.0) as usize) {
            Some(t) => Out::Text(t),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%pane-set-row id idx style text)` — replace one row's style + text.
    "%pane-set-row", fn pane_set_row(id: num, idx: num, style: text, body: text) {
        let style = crate::textpane::Style::parse(&style)
            .unwrap_or(crate::textpane::Style::Normal);
        crate::textpane::set_row(id as u32, idx.max(0.0) as usize, style, &body);
        Out::Nil
    }
}

prim! {
    /// `(%pane-insert-row id idx style text)` — insert a row at `idx`.
    "%pane-insert-row", fn pane_insert_row(id: num, idx: num, style: text, body: text) {
        let style = crate::textpane::Style::parse(&style)
            .unwrap_or(crate::textpane::Style::Normal);
        crate::textpane::insert_row(id as u32, idx.max(0.0) as usize, style, &body);
        Out::Nil
    }
}

prim! {
    /// `(%pane-remove-row id idx)` — remove the row at `idx`.
    "%pane-remove-row", fn pane_remove_row(id: num, idx: num) {
        crate::textpane::remove_row(id as u32, idx.max(0.0) as usize);
        Out::Nil
    }
}

prim! {
    /// `(%pane-reveal-row id idx)` — scroll a top-anchored list so row `idx`
    /// shows.
    "%pane-reveal-row", fn pane_reveal_row(id: num, idx: num) {
        crate::textpane::reveal_row(id as u32, idx.max(0.0) as usize);
        Out::Nil
    }
}

prim! {
    /// `(%pane-set-input id prompt text cursor)` — mirror the Scheme-side
    /// input editor's state for drawing. An empty prompt removes the input
    /// line.
    "%pane-set-input", fn pane_set_input(id: num, prompt: text, body: text, cursor: num) {
        crate::textpane::set_input(id as u32, &prompt, &body, cursor.max(0.0) as usize);
        Out::Nil
    }
}

prim! {
    /// `(%set-text-size px)` — the text panes' font size in pixels, clamped to
    /// sanity; px below the minimum (e.g. 0) just reads. Returns the size in
    /// effect.
    "%set-text-size", fn set_text_size(a0: num) {
        let px = a0 as f32;
        Out::Num(crate::textpane::set_font_px(px) as f64)

    }
}

prim! {
    /// `(%set-text-font path index)` — load a monospace face for the text
    /// panes from a local font file (.ttf/.otf/.ttc; index picks inside a
    /// collection). Returns "ok" or the error; the current face survives a bad
    /// load.
    "%set-text-font", fn set_text_font(path: opt_text, a1: num) {
        let path = path.unwrap_or_default();
        let index = (a1 as f32).max(0.0) as u32;
        let msg = match crate::text::set_mono_font(&path, index) {
            Ok(()) => "ok".to_string(),
            Err(e) => {
                eprintln!("[font] {e}");
                format!("error: {e}")
            }
        };
        Out::Text(msg)
    }
}

prim! {
    /// `(%set-text-rendering mode)` — the glyph hinting/placement knob
    /// (`docs/modeler-text-rendering-research.md`). MODE is "auto", "smooth",
    /// "zed", "crisp", "current", or "" to read without setting. Returns the
    /// mode now DECLARED (`%text-rendering` reports what "auto" resolves to);
    /// an unknown name is reported and changes nothing.
    "%set-text-rendering", fn set_text_rendering(mode: name) {
        use crate::text::TextMode;
        match mode.unwrap_or_default().as_str() {
            "current" => crate::text::set_mode(TextMode::Current),
            "zed" => crate::text::set_mode(TextMode::Zed),
            "crisp" => crate::text::set_mode(TextMode::Crisp),
            "smooth" => crate::text::set_mode(TextMode::Smooth),
            "auto" => crate::text::set_mode(TextMode::Auto),
            "" => {}
            other => eprintln!("[text] unknown rendering mode '{other}'"),
        }
        Out::Text(mode_name(crate::text::mode()).to_string())
    }
}

/// The spelling `(set-text-rendering)` and `(text-rendering)` speak.
fn mode_name(m: crate::text::TextMode) -> &'static str {
    use crate::text::TextMode;
    match m {
        TextMode::Current => "current",
        TextMode::Zed => "zed",
        TextMode::Crisp => "crisp",
        TextMode::Smooth => "smooth",
        TextMode::Auto => "auto",
    }
}

prim! {
    /// `(%text-rendering)` — the mode actually drawing, with "auto" resolved
    /// against the display's current scale.
    "%text-rendering", fn text_rendering() {
        Out::Text(mode_name(crate::text::effective()).to_string())
    }
}

prim! {
    /// `(%set-label-font path index)` — load the proportional UI/label face
    /// (menus, completion rows, the status strip, viewport HUDs) from a local
    /// font file. Same contract as `%set-text-font`: returns "ok" or the error,
    /// and the current face survives a bad load.
    "%set-label-font", fn set_label_font(path: opt_text, a1: num) {
        let path = path.unwrap_or_default();
        let index = (a1 as f32).max(0.0) as u32;
        let msg = match crate::text::set_label_font(&path, index) {
            Ok(()) => "ok".to_string(),
            Err(e) => {
                eprintln!("[font] {e}");
                format!("error: {e}")
            }
        };
        Out::Text(msg)
    }
}


prim! {
    /// `(%pane-row-at id x y)` — the row of text pane ID under a window
    /// point, or #f: what a click on a list (the file tree) hits.
    "%pane-row-at", fn pane_row_at(id: num, x: num, y: num) {
        match crate::textpane::row_at(id as u32, x, y) {
            Some(r) => Out::Num(r as f64),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%pane-top-anchor id on)` — lay text pane ID out as a LIST, rows
    /// from the top down (the file tree, an outliner), rather than as a
    /// transcript anchored at its bottom.
    "%pane-top-anchor", fn pane_top_anchor(id: num, on: num) {
        crate::textpane::set_top_anchor(id as u32, on != 0.0);
        Out::Nil
    }
}

prim! {
    /// `(%pane-proportional id on)` — set list pane ID's row text in the
    /// proportional label face, its leading marks kept on the mono grid (the
    /// file tree).
    "%pane-proportional", fn pane_proportional(id: num, on: num) {
        crate::textpane::set_proportional(id as u32, on != 0.0);
        Out::Nil
    }
}

/// This module's prims, installed; answers their names.
pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        set_text_rendering::install(interp)?,
        text_rendering::install(interp)?,
        set_text_size::install(interp)?,
        set_text_font::install(interp)?,
        set_label_font::install(interp)?,
        pane_append::install(interp)?,
        pane_clear::install(interp)?,
        pane_scroll::install(interp)?,
        pane_row_count::install(interp)?,
        pane_row_text::install(interp)?,
        pane_set_row::install(interp)?,
        pane_insert_row::install(interp)?,
        pane_remove_row::install(interp)?,
        pane_reveal_row::install(interp)?,
        pane_set_input::install(interp)?,
        pane_row_at::install(interp)?,
        pane_top_anchor::install(interp)?,
        pane_proportional::install(interp)?,
    ])
}
