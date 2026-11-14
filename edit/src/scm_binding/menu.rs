//! The menu prims — the popup overlay (the linear menu and completion share
//! it) and the marking-menu pie — moved from the modeler's `script/scene.rs`,
//! `script/panes.rs` and `script.rs` with the menus' Scheme
//! (docs/editor-crate-plan.md, E3b), and the completion overlay's own
//! `%set-completions` with the minibuffer's (E3c). Each is one call into
//! `textpane`.

prim! {
    /// `(%show-menu lines selected x y)` — open the popup overlay as a context
    /// menu at a window point ('\n'-separated rows). `(%overlay-hit x y)` →
    /// the row index under a point, or nil. `(%overlay-select idx)` moves the
    /// highlight. `%clear-completions` closes it (shared with completion).
    "%show-menu", fn show_menu(lines: opt_text, a1: num, a2: num, a3: num, a4: num) {
        let lines = lines.unwrap_or_default();
        let selected = (a1 as f32).max(0.0) as usize;
        let x = a2 as f32;
        let y = a3 as f32;
        // Arg 4: solid rather than glass — a menu over a text pane.
        let opaque = (a4 as f32) > 0.5;
        crate::textpane::show_menu(&lines, selected, x, y, opaque);
        Out::Nil

    }
}

prim! {
    "%overlay-hit", fn overlay_hit(a0: num, a1: num) {
        let (x, y) = (
            (a0 as f32) as f64,
            (a1 as f32) as f64,
        );
        match crate::textpane::overlay_hit(x, y) {
            Some(row) => Out::Num(row as f64),
            None => Out::Nil,
        }

    }
}

prim! {
    "%overlay-select", fn overlay_select(a0: num) {
        crate::textpane::set_overlay_selected((a0 as f32).max(0.0) as usize);
        Out::Nil

    }
}

prim! {
    /// `(%show-pie lines selected cx cy)` — the marking-menu pie at an anchor
    /// ('\n'-separated labels, ring-ordered from up, clockwise; selected < 0 =
    /// dead zone, nothing armed).
    "%show-pie", fn show_pie(lines: opt_text, a1: num, a2: num, a3: num, a4: num) {
        let lines = lines.unwrap_or_default();
        let selected = (a1 as f32) as i32;
        let x = a2 as f32;
        let y = a3 as f32;
        let opaque = (a4 as f32) > 0.5;
        crate::textpane::show_pie(&lines, selected, x, y, opaque);
        Out::Nil

    }
}

prim! {
    /// `(%pie-mark x y)` — the gesture ink trail's end point (drag position).
    "%pie-mark", fn pie_mark(a0: num, a1: num) {
        crate::textpane::set_pie_mark(
            a0 as f32,
            a1 as f32,
        );
        Out::Nil

    }
}

prim! {
    "%clear-completions", fn clear_completions() {
        crate::textpane::clear_completions();
        Out::Nil
    }
}

prim! {
    /// `(%set-menu-alpha a)` — the menus' glass opacity (0..1); a negative or
    /// missing argument only reads it. Returns what is in effect.
    "%set-menu-alpha", fn set_menu_alpha(a0: num) {
        let a = a0 as f32;
        Out::Num(crate::textpane::set_menu_alpha(a) as f64)

    }
}

prim! {
    /// `(%set-completions pane lines selected)` — show the completion overlay:
    /// '\n'-separated rows anchored above the pane's input line.
    /// `(%clear-completions)` hides it.
    "%set-completions", fn set_completions(pane: num, lines: opt_text, selected: num) {
        let pane = pane as u32;
        let lines = lines.unwrap_or_default();
        let selected = selected.max(0.0) as usize;
        crate::textpane::set_completions(pane, &lines, selected);
        Out::Nil

    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        show_menu::install(interp)?,
        overlay_hit::install(interp)?,
        overlay_select::install(interp)?,
        show_pie::install(interp)?,
        pie_mark::install(interp)?,
        clear_completions::install(interp)?,
        set_menu_alpha::install(interp)?,
        set_completions::install(interp)?,
    ])
}
