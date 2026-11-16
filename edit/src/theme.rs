//! **The colours the editor draws in** — the slots of the modeler's theme
//! that a text pane, the editor and the popup read, and no others. A host
//! hands one over through [`crate::host::Host::theme`] on every draw; the
//! modeler fills it from its own table, which `set-color` writes, so a theme
//! change reaches the editor on the next paint with nothing to sync.
//!
//! The defaults are the modeler's One Dark, slot for slot. Its
//! `modeler/src/theme.rs` documents each; they are kept short here.

/// Read per draw through [`crate::host::theme`].
pub fn get() -> Theme {
    crate::host::theme()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// A text pane's background.
    pub panel_bg: u32,
    /// The editor's background, its own place between panels and stage.
    pub editor_bg: u32,
    pub border_focused: u32,
    /// The editor's mode line.
    pub status_bg: u32,
    pub status_ink: u32,
    /// Body text, de-emphasis, emphasis (prompt, cursor block), errors,
    /// warnings (E8: a name nothing binds, a call with the wrong count).
    pub text_ink: u32,
    pub text_dim: u32,
    pub text_accent: u32,
    pub text_error: u32,
    pub text_warning: u32,
    /// The popup's background, a menu row's label and the key beside it.
    pub popup_bg: u32,
    pub menu_ink: u32,
    pub menu_key_ink: u32,
    /// Syntax inks.
    pub code_comment: u32,
    pub code_string: u32,
    pub code_keyword: u32,
    pub code_number: u32,
    pub code_call: u32,
    pub code_paren: u32,
}

impl Theme {
    /// A slot by its Scheme name (`"panel-bg"` for `:panel-bg`) — the
    /// modeler's names for the same slots, so one theme file serves both
    /// hosts. `None` for a name that is not a slot.
    pub fn slot_mut(&mut self, name: &str) -> Option<&mut u32> {
        Some(match name {
            "panel-bg" => &mut self.panel_bg,
            "editor-bg" => &mut self.editor_bg,
            "border-focused" => &mut self.border_focused,
            "status-bg" => &mut self.status_bg,
            "status-ink" => &mut self.status_ink,
            "text-ink" => &mut self.text_ink,
            "text-dim" => &mut self.text_dim,
            "text-accent" => &mut self.text_accent,
            "text-error" => &mut self.text_error,
            "text-warning" => &mut self.text_warning,
            "popup-bg" => &mut self.popup_bg,
            "menu-ink" => &mut self.menu_ink,
            "menu-key-ink" => &mut self.menu_key_ink,
            "code-comment" => &mut self.code_comment,
            "code-string" => &mut self.code_string,
            "code-keyword" => &mut self.code_keyword,
            "code-number" => &mut self.code_number,
            "code-call" => &mut self.code_call,
            "code-paren" => &mut self.code_paren,
            _ => return None,
        })
    }
}

impl Default for Theme {
    fn default() -> Self {
        // One Dark — the modeler's `Theme::default`, these slots of it.
        Self {
            panel_bg: 0x002f_343e,
            editor_bg: 0x0028_2c34,
            border_focused: 0x003e_4451,
            status_bg: 0x0021_252b,
            status_ink: 0x00ab_b2bf,
            text_ink: 0x00ab_b2bf,
            text_dim: 0x0082_8997,
            text_accent: 0x0061_afef,
            text_error: 0x00e0_6c75,
            text_warning: 0x00e5_c07b,
            popup_bg: 0x003a_3f4b,
            // Near-white, as a desktop menu's (Kevin, 2026-11-14).
            menu_ink: 0x00f5_f7fa,
            menu_key_ink: 0x00c3_c8d0,
            code_comment: 0x005c_6370,
            code_string: 0x0098_c379,
            code_keyword: 0x00c6_78dd,
            code_number: 0x00d1_9a66,
            code_call: 0x0061_afef,
            code_paren: 0x0082_8997,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_write_by_name_and_reject_typos() {
        let mut t = Theme::default();
        *t.slot_mut("code-string").unwrap() = 0x0012_3456;
        assert_eq!(t.code_string, 0x0012_3456);
        assert!(t.slot_mut("code_string").is_none());
        assert!(t.slot_mut("viewport-bg").is_none(), "a modeler-only slot is not the editor's");
    }
}
