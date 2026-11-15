//! The editor-pane prims and the buffer registry
//! (`docs/modeler-scene-editor-plan.md`) — lifted out of `script.rs` whole
//! (A4).
//!
//! The cosmic-text editor pane, its vi mode and undo, and the registry that
//! gives a buffer a path, a name, an MRU position and an auto-opened flag.

// ── Editor-pane primitives (docs/modeler-scene-editor-plan.md) ─────────────
// The kernel half of the scene-editor pane: a cosmic-text buffer per pane
// (editorpane.rs). WHICH file loads and when it saves are Scheme policy
// (unify.scm's scene-editor section).

prim! {
    /// `(%editor-set-text id text)` — replace an editor pane's whole buffer
    /// (scroll and cursor reset to the top).
    "%editor-set-text", fn editor_set_text(id: num, a1: opt_text) {
        let id = id as u32;
        if let Some(text) = a1 {
            crate::editorpane::set_text(id, &text);
        }
        Out::Nil

    }
}

prim! {
    /// `(%editor-text id)` → an editor pane's buffer content as one string —
    /// what save writes.
    "%editor-text", fn editor_text(id: num) {
        let id = id as u32;
        Out::Text(crate::editorpane::text(id))

    }
}

prim! {
    /// `(%editor-scroll id rows)` — scroll an editor pane by text rows
    /// (positive = toward the end of the file).
    "%editor-scroll", fn editor_scroll(id: num, rows: num) {
        crate::editorpane::scroll(id as u32, rows as f32);
        Out::Nil
    }
}

prim! {
    /// `(%editor-act id kind &opt arg x y)` — one named edit action: "insert"
    /// (arg = text), "enter"/"backspace"/"delete"/"escape", the motions
    /// ("left"…"buffer-end"/"page-up"/"page-down"), the mouse family
    /// ("click"/"double-click"/"triple-click"/"drag", x y = WINDOW coords), and
    /// the text-yielding trio "copy"/"cut"/"kill-eol" (returns the text — the
    /// Scheme side owns the clipboard). "enter" returns the submitted vi
    /// ex-command as ":<text>" (e.g. ":w foo.scm") when one was on the ':'
    /// line, so Scheme can dispatch it; nil otherwise.
    "%editor-act", fn editor_act(id: num, kind: opt_text, arg: opt_text, x: num, y: num) {
        let id = id as u32;
        let kind = kind.unwrap_or_default();
        let x = x as f64;
        let y = y as f64;
        match crate::editorpane::act(id, &kind, arg.as_deref(), x, y) {
            Some(text) => Out::Text(text),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%editor-completions id lines selected)` — show the completion popup
    /// anchored at the editor pane's cursor ('\n'-separated rows; flips above
    /// when near the bottom). `(%clear-completions)` hides it.
    "%editor-completions", fn editor_completions(id: num, lines: opt_text, selected: num) {
        let id = id as u32;
        let lines = lines.unwrap_or_default();
        let selected = selected.max(0.0) as usize;
        if let Some((x, y, line_h)) = crate::editorpane::cursor_window_pos(id) {
            crate::textpane::set_completions_at(x, y, line_h, &lines, selected);
        }
        Out::Nil

    }
}

prim! {
    /// `(%editor-replace id l0 i0 l1 i1 text)` — replace the byte range with
    /// `text` as one undo step, cursor after it (completion's token swap).
    "%editor-replace", fn editor_replace(id: num, l0: num, i0: num, l1: num, i1: num, a5: opt_text) {
        let id = id as u32;
        let l0 = l0.max(0.0) as usize;
        let i0 = i0.max(0.0) as usize;
        let l1 = l1.max(0.0) as usize;
        let i1 = i1.max(0.0) as usize;
        if let Some(text) = a5 {
            crate::editorpane::replace_range(id, l0, i0, l1, i1, &text);
        }
        Out::Nil

    }
}

prim! {
    /// `(%editor-head id)` — the head symbol of the innermost unclosed `(` at
    /// the cursor, or nil: eldoc's anchor for signature help.
    "%editor-head", fn editor_head(id: num) {
        let id = id as u32;
        match crate::editorpane::enclosing_head(id) {
            Some(head) => Out::Text(head),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%editor-set-language id name)` — the active buffer's language, by
    /// name ("scheme", "guile"): highlighting, indentation and read marks
    /// follow. 1 when NAME is a language.
    "%editor-set-language", fn editor_set_language(id: num, name: text) {
        Out::Num(if crate::editorpane::set_language(id as u32, &name) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%buffer-language id buf)` — buffer BUF's language, by name.
    "%buffer-language", fn buffer_language(id: num, buf: num) {
        match crate::editorpane::buffer_language(id as u32, buf as u32) {
            Some(n) => Out::Text(n.to_string()),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%editor-diagnostics id)` — what is wrong with the active buffer:
    /// what its reader cannot read (E3f), and what its watchers say (a
    /// language server's marks, E7; the warnings, E8), as `((line column
    /// message severity) …)`, the line from 0, the column in CHARACTERS,
    /// severity 1 for an error, 2 a warning and 0 a form left open.
    "%editor-diagnostics", fn editor_diagnostics(id: num) {
        let id = id as u32;
        let text = crate::editorpane::text(id);
        let lines: Vec<&str> = text.split('\n').collect();
        Out::rows(crate::editorpane::diagnostics(id).into_iter().map(|d| {
            let (line, byte) = d.start;
            let column = lines.get(line).map_or(0, |l| l[..byte.min(l.len())].chars().count());
            Out::Many(vec![
                Out::Num(line as f64),
                Out::Num(column as f64),
                Out::Text(d.message),
                Out::Num(match d.severity {
                    crate::language::Severity::Error => 1.0,
                    crate::language::Severity::Open => 0.0,
                    crate::language::Severity::Warning => 2.0,
                }),
            ])
        }))
    }
}

prim! {
    /// `(%editor-set-vi on)` — global vi-mode toggle for editor panes (modit,
    /// COSMIC's vi engine: NORMAL/INSERT/VISUAL, motions, operators, text
    /// objects). `(%editor-vi-mode id)` reads the mode badge text ("" = off).
    "%editor-set-vi", fn editor_set_vi(a0: num) {
        crate::editorpane::set_vi(a0 != 0.0);
        Out::Nil

    }
}

prim! {
    "%editor-vi-mode", fn editor_vi_mode(id: num) {
        let id = id as u32;
        Out::Text(crate::editorpane::vi_mode_label(id).unwrap_or_default())

    }
}

prim! {
    /// `(%editor-find id needle line byte forward fold)` → `[line start end
    /// wrapped]`, the next match of NEEDLE from (LINE, BYTE) — forward, or
    /// backward from before it (FORWARD 0) — wrapping round once (WRAPPED 1);
    /// nil when there is none. FOLD 1 ignores ASCII case.
    "%editor-find", fn editor_find(id: num, needle: text, line: num, byte: num, forward: num, fold: num) {
        match crate::editorpane::find(id as u32, &needle, (line.max(0.0) as usize, byte.max(0.0) as usize), forward != 0.0, fold != 0.0) {
            Some(((l, i0, i1), wrapped)) => Out::Many(vec![
                Out::Num(l as f64),
                Out::Num(i0 as f64),
                Out::Num(i1 as f64),
                Out::Num(if wrapped { 1.0 } else { 0.0 }),
            ]),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%editor-select id line i0 i1)` — select bytes I0..I1 of LINE, the
    /// cursor at the end, scrolled into view: a search's match.
    "%editor-select", fn editor_select(id: num, line: num, i0: num, i1: num) {
        crate::editorpane::select_range(id as u32, line.max(0.0) as usize, i0.max(0.0) as usize, i1.max(0.0) as usize);
        Out::Nil
    }
}

prim! {
    /// `(%editor-replace-all id needle with fold l0 b0 l1 b1)` → how many:
    /// every match of NEEDLE starting in [(L0,B0), (L1,B1)) replaced by WITH,
    /// one undo step. An end before the start wraps round the buffer's end.
    "%editor-replace-all", fn editor_replace_all(id: num, needle: text, with: text, fold: num, l0: num, b0: num, l1: num, b1: num) {
        let at = |l: f64, b: f64| (l.max(0.0) as usize, b.max(0.0) as usize);
        Out::Num(crate::editorpane::replace_all(id as u32, &needle, &with, fold != 0.0, at(l0, b0), at(l1, b1)) as f64)
    }
}

prim! {
    /// `(%editor-cursor-xy id)` → `[x y]`, the window point just under the
    /// cursor — where a popup about the text there opens; nil before the
    /// first layout.
    "%editor-cursor-xy", fn editor_cursor_xy(id: num) {
        match crate::editorpane::cursor_window_pos(id as u32) {
            Some((x, y, line_h)) => Out::Many(vec![Out::Num(x as f64), Out::Num((y + line_h) as f64)]),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%editor-cursor id)` → `[line byte-index]` — the cursor position, the
    /// anchor for send-expression-at-point (unify.scm's sexp scanner).
    "%editor-cursor", fn editor_cursor(a0: num) {
        let id = (a0 as f32) as u32;
        let (line, index) = crate::editorpane::cursor(id);
        Out::Many(vec![
            Out::Num(line as f64),
            Out::Num(index as f64),
        ])

    }
}

prim! {
    /// `(%editor-goto-line id line)` — place point at the start of LOGICAL line
    /// `line` (0-based, clamped), clearing any selection. Layout-independent
    /// (addresses the logical line, not a visual row), so scripts can position
    /// reliably in a wrapping pane where the `"down"` motion would miscount.
    "%editor-goto-line", fn editor_goto_line(id: num, line: num) {
        let id = id as u32;
        let line = line.max(0.0) as usize;
        crate::editorpane::goto_line(id, line);
        Out::Nil

    }
}

prim! {
    /// `(%editor-select-line id line)` — select the whole of LOGICAL line `line`
    /// (0-based, clamped), cursor left at its end. The keyboard counterpart to a
    /// triple-click, for driving region sends deterministically.
    "%editor-select-line", fn editor_select_line(id: num, line: num) {
        let id = id as u32;
        let line = line.max(0.0) as usize;
        crate::editorpane::select_line(id, line);
        Out::Nil

    }
}

prim! {
    /// `(%editor-undo id)` / `(%editor-redo id)` — buffer-content undo/redo
    /// (separate from the scene's (undo) — text is text). Returns 1 if applied,
    /// 0 if the stack was empty (a NUMBER, from Janet days, when
    /// `janet_wrap_boolean` was a macro bindgen could not see; callers
    /// compare with `positive?`).
    "%editor-undo", fn editor_undo(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::undo(id) as u8 as f64)

    }
}

prim! {
    "%editor-redo", fn editor_redo(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::redo(id) as u8 as f64)

    }
}

prim! {
    /// `(%editor-dirty? id)` — 1 when modified since load/save, else 0 (a
    /// NUMBER; compare with `positive?` — 0 is truthy). The pane also wears
    /// an accent dot while dirty. `(%editor-mark-saved id)` clears it (save
    /// wrote the buffer to disk).
    "%editor-dirty?", fn editor_dirty(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::dirty(id) as u8 as f64)

    }
}

prim! {
    "%editor-mark-saved", fn editor_mark_saved(id: num) {
        let id = id as u32;
        crate::editorpane::mark_saved(id);
        Out::Nil

    }
}

// ── S5: multiple buffers per editor pane ───────────────────────────────────

prim! {
    /// `(%editor-buffer-new id)` → the new buffer's id — a fresh empty document,
    /// made active. Scheme names it (`%buffer-set-name`, `%buffer-set-path`).
    "%editor-buffer-new", fn editor_buffer_new(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::buffer_new(id) as f64)

    }
}

prim! {
    /// `(%editor-buffer-switch id buf)` — make `buf` the active buffer; preserves
    /// every buffer's cursor/scroll/undo/dirty. Returns 1 if it existed, else 0.
    "%editor-buffer-switch", fn editor_buffer_switch(id: num, buf: num) {
        let id = id as u32;
        let buf = buf as u32;
        Out::Num(crate::editorpane::buffer_switch(id, buf) as u8 as f64)

    }
}

prim! {
    /// `(%editor-buffer-close id buf)` — drop `buf` (never the last one). Returns
    /// 1 if removed, else 0.
    "%editor-buffer-close", fn editor_buffer_close(id: num, buf: num) {
        let id = id as u32;
        let buf = buf as u32;
        Out::Num(crate::editorpane::buffer_close(id, buf) as u8 as f64)

    }
}

prim! {
    /// `(%editor-active-buffer id)` → the active buffer's id.
    "%editor-active-buffer", fn editor_active_buffer(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::active_buffer(id) as f64)

    }
}

// ── Buffer registry prims (review item 1c): path / name / MRU / auto-opened
// live on the Rust Doc — Scheme consults these instead of a parallel table. ──

prim! {
    /// `(%editor-set-name id name)` — set the active buffer's modeline name.
    /// `(%buffer-path id buf)` → the file `buf` visits, or nil (scratch).
    "%buffer-path", fn buffer_path_prim(id: num, buf: num) {
        let id = id as u32;
        let buf = buf as u32;
        match crate::editorpane::buffer_path(id, buf) {
            Some(p) => Out::Text(p),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%buffer-set-path id buf path-or-nil)` — point `buf` at a file.
    "%buffer-set-path", fn buffer_set_path_prim(id: num, buf: num, path: opt_text) {
        let id = id as u32;
        let buf = buf as u32;
        crate::editorpane::set_buffer_path(id, buf, path.as_deref());
        Out::Nil

    }
}

prim! {
    /// `(%buffer-name id buf)` → `buf`'s modeline display name, or nil.
    "%buffer-name", fn buffer_name_prim(id: num, buf: num) {
        let id = id as u32;
        let buf = buf as u32;
        match crate::editorpane::buffer_name(id, buf) {
            Some(n) => Out::Text(n),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%buffer-dialect id buf)` — the dialect `buf`'s text is written for
    /// ("ray"), "" for none.
    "%buffer-dialect", fn buffer_dialect_prim(id: num, buf: num) {
        Out::Text(crate::editorpane::buffer_dialect(id as u32, buf as u32).unwrap_or_default())
    }
}

prim! {
    /// `(%buffer-set-dialect id buf text)` — set it: the modeline says
    /// "Scheme (ray)".
    "%buffer-set-dialect", fn buffer_set_dialect_prim(id: num, buf: num, a2: opt_text) {
        crate::editorpane::set_buffer_dialect(id as u32, buf as u32, &a2.unwrap_or_default());
        Out::Nil
    }
}

prim! {
    /// `(%buffer-tag id buf)` — `buf`'s modeline tag ("" for none).
    "%buffer-tag", fn buffer_tag_prim(id: num, buf: num) {
        Out::Text(crate::editorpane::buffer_tag(id as u32, buf as u32).unwrap_or_default())
    }
}

prim! {
    /// `(%buffer-set-tag id buf text)` — what the modeline says after
    /// `buf`'s name: its place in the loaded scene.
    "%buffer-set-tag", fn buffer_set_tag_prim(id: num, buf: num, a2: opt_text) {
        crate::editorpane::set_buffer_tag(id as u32, buf as u32, &a2.unwrap_or_default());
        Out::Nil
    }
}

prim! {
    /// `(%editor-take-owned id start end)` — the scene keys owned by the
    /// anchors at character offsets START..END of the active buffer, which
    /// are removed: that text is about to run again (E1).
    "%editor-take-owned", fn editor_take_owned(id: num, start: num, end: num) {
        let keys = crate::editorpane::take_owned(id as u32, start.max(0.0) as usize, end.max(0.0) as usize);
        Out::Arr(keys.into_iter().map(Out::Text).collect())
    }
}

prim! {
    /// `(%editor-own id at keys)` — anchor KEYS (what the text from AT made)
    /// at character offset AT of the active buffer; the anchor moves with
    /// the text.
    "%editor-own", fn editor_own(id: num, at: num, keys: textv) {
        crate::editorpane::own(id as u32, at.max(0.0) as usize, keys);
        Out::Nil
    }
}

prim! {
    /// `(%editor-owned-clear id)` — forget the active buffer's anchors.
    "%editor-owned-clear", fn editor_owned_clear(id: num) {
        crate::editorpane::owned_clear(id as u32);
        Out::Nil
    }
}

prim! {
    /// `(%editor-owners id)` — the active buffer's anchors, `((offset key …)
    /// …)` in offset order.
    "%editor-owners", fn editor_owners(id: num) {
        Out::Arr(
            crate::editorpane::owners(id as u32)
                .into_iter()
                .map(|(at, keys)| {
                    let mut row = vec![Out::Num(at as f64)];
                    row.extend(keys.into_iter().map(Out::Text));
                    Out::Many(row)
                })
                .collect(),
        )
    }
}

prim! {
    /// `(%buffer-set-name id buf name)` — set `buf`'s display name.
    "%buffer-set-name", fn buffer_set_name_prim(id: num, buf: num, a2: opt_text) {
        let id = id as u32;
        let buf = buf as u32;
        if let Some(name) = a2 {
            crate::editorpane::set_buffer_name(id, buf, &name);
        }
        Out::Nil

    }
}

prim! {
    /// `(%buffer-by-name id name)` → the buffer named exactly NAME, or nil.
    "%buffer-by-name", fn buffer_by_name_prim(id: num, found: opt_text) {
        let id = id as u32;
        let found = found
            .and_then(|n| crate::editorpane::buffer_by_name(id, &n));
        match found {
            Some(b) => Out::Num(b as f64),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%buffer-by-path id path)` → the buffer visiting PATH, or nil.
    "%buffer-by-path", fn buffer_by_path_prim(id: num, found: opt_text) {
        let id = id as u32;
        let found = found
            .and_then(|p| crate::editorpane::buffer_by_path(id, &p));
        match found {
            Some(b) => Out::Num(b as f64),
            None => Out::Nil,
        }

    }
}

prim! {
    /// `(%buffer-mru id)` → buffer ids, most-recently-visited first. Always
    /// consistent with the live buffer list (close prunes) — no janitor.
    "%buffer-mru", fn buffer_mru_prim(id: num) {
        Out::nums(crate::editorpane::buffer_mru(id as u32).into_iter().map(|b| b as f64))
    }
}

prim! {
    /// `(%buffer-auto-opened? id buf)` → 1/0: did showing `buf` auto-open the dock?
    "%buffer-auto-opened?", fn buffer_auto_opened_prim(id: num, buf: num) {
        let id = id as u32;
        let buf = buf as u32;
        Out::Num(u8::from(crate::editorpane::buffer_auto_opened(id, buf)) as f64)

    }
}

prim! {
    /// `(%buffer-set-auto-opened id buf on)` — record the dock-auto-open flag.
    "%buffer-set-auto-opened", fn buffer_set_auto_opened_prim(id: num, buf: num, on: num) {
        let id = id as u32;
        let buf = buf as u32;
        let on = on != 0.0;
        crate::editorpane::set_buffer_auto_opened(id, buf, on);
        Out::Nil

    }
}

prim! {
    "%editor-set-name", fn editor_set_name(id: num, a1: opt_text) {
        let id = id as u32;
        if let Some(name) = a1 {
            crate::editorpane::set_name(id, &name);
        }
        Out::Nil

    }
}

prim! {
    /// `(%editor-open-video id path)` — play PATH (a movie) in the active
    /// buffer, view-only (crate::video; ffmpeg decodes it). Returns 1: a
    /// clip that cannot play says why on its pane.
    "%editor-open-video", fn editor_open_video(id: num, path: opt_text) {
        let ok = path.map(|p| crate::editorpane::open_video(id as u32, &p)).unwrap_or(false);
        Out::Num(ok as u8 as f64)
    }
}

prim! {
    /// `(%video-control id what)` — a movie buffer's control by name
    /// ("toggle" "play" "pause" "restart" "loop" "step" "back" "end", or
    /// "state" to only ask) → (frame decoded playing looping), 1/0 flags;
    /// nil when the active buffer is not a movie.
    "%video-control", fn video_control(id: num, what: opt_text) {
        match crate::editorpane::video_control(id as u32, &what.unwrap_or_default()) {
            Some((f, n, playing, looping)) => {
                Out::nums([f as f64, n as f64, playing as u8 as f64, looping as u8 as f64])
            }
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%editor-open-image id path)` — load PATH (a PNG) into the active buffer
    /// as a view-only image. Returns 1 on success, 0 if it didn't decode.
    "%editor-open-image", fn editor_open_image(id: num, ok: opt_text) {
        let id = id as u32;
        let ok = ok
            .map(|p| crate::editorpane::open_image(id, &p))
            .unwrap_or(false);
        Out::Num(ok as u8 as f64)

    }
}

prim! {
    /// `(%editor-open-markdown id path)` — load PATH (a .md) into the active
    /// buffer as a view-only, source-highlighted markdown buffer. Returns 1 on
    /// success, 0 if the file didn't read.
    "%editor-open-markdown", fn editor_open_markdown(id: num, ok: opt_text) {
        let id = id as u32;
        let ok = ok
            .map(|p| crate::editorpane::open_markdown(id, &p))
            .unwrap_or(false);
        Out::Num(ok as u8 as f64)

    }
}

prim! {
    /// `(%editor-open-markdown-text id src)` — turn the active buffer into a
    /// view-only markdown buffer from the string SRC (a generated doc, no file).
    "%editor-open-markdown-text", fn editor_open_markdown_text(id: num, a1: opt_text) {
        let id = id as u32;
        if let Some(src) = a1 {
            crate::editorpane::open_markdown_text(id, &src);
        }
        Out::Nil

    }
}

prim! {
    /// `(%editor-toggle-markdown id)` — flip the active markdown buffer between
    /// source and reading view. Returns 1 (reading), 0 (source), -1 (not markdown).
    "%editor-toggle-markdown", fn editor_toggle_markdown(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::toggle_markdown_reading(id) as f64)

    }
}

prim! {
    /// `(%editor-markdown? id)` — 1 when the active buffer is a view-only markdown
    /// buffer, else 0.
    "%editor-markdown?", fn editor_markdown_p(id: num) {
        let id = id as u32;
        Out::Num(crate::editorpane::is_markdown(id) as u8 as f64)

    }
}

prim! {
    /// `(%editor-buffer-list id)` → `[[buf-id dirty?] ...]`, storage order. Scheme
    /// joins each id with its name/path (`%buffer-name`, `%buffer-path`).
    "%editor-buffer-list", fn editor_buffer_list(id: num) {
        Out::rows(
            crate::editorpane::buffer_list(id as u32)
                .into_iter()
                .map(|(buf, dirty)| Out::Nums(vec![buf as f64, f64::from(u8::from(dirty))])),
        )
    }
}


/// This module's prims, installed; answers their names.
pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        buffer_mru_prim::install(interp)?,
        editor_buffer_list::install(interp)?,
        editor_cursor::install(interp)?,
        editor_find::install(interp)?,
        editor_cursor_xy::install(interp)?,
        editor_select::install(interp)?,
        editor_replace_all::install(interp)?,
        editor_set_text::install(interp)?,
        editor_text::install(interp)?,
        editor_scroll::install(interp)?,
        editor_act::install(interp)?,
        editor_completions::install(interp)?,
        editor_replace::install(interp)?,
        editor_head::install(interp)?,
        editor_diagnostics::install(interp)?,
        editor_set_language::install(interp)?,
        buffer_language::install(interp)?,
        editor_set_vi::install(interp)?,
        editor_vi_mode::install(interp)?,
        editor_goto_line::install(interp)?,
        editor_select_line::install(interp)?,
        editor_undo::install(interp)?,
        editor_redo::install(interp)?,
        editor_dirty::install(interp)?,
        editor_mark_saved::install(interp)?,
        editor_buffer_new::install(interp)?,
        editor_buffer_switch::install(interp)?,
        editor_buffer_close::install(interp)?,
        editor_active_buffer::install(interp)?,
        buffer_path_prim::install(interp)?,
        buffer_set_path_prim::install(interp)?,
        buffer_name_prim::install(interp)?,
        buffer_set_name_prim::install(interp)?,
        buffer_tag_prim::install(interp)?,
        buffer_dialect_prim::install(interp)?,
        buffer_set_dialect_prim::install(interp)?,
        buffer_set_tag_prim::install(interp)?,
        editor_take_owned::install(interp)?,
        editor_own::install(interp)?,
        editor_owned_clear::install(interp)?,
        editor_owners::install(interp)?,
        buffer_by_name_prim::install(interp)?,
        buffer_by_path_prim::install(interp)?,
        buffer_auto_opened_prim::install(interp)?,
        buffer_set_auto_opened_prim::install(interp)?,
        editor_set_name::install(interp)?,
        editor_open_image::install(interp)?,
        editor_open_video::install(interp)?,
        video_control::install(interp)?,
        editor_open_markdown::install(interp)?,
        editor_open_markdown_text::install(interp)?,
        editor_toggle_markdown::install(interp)?,
        editor_markdown_p::install(interp)?,
    ])
}
