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
    /// Body text, de-emphasis, emphasis (prompt, cursor block), errors.
    pub text_ink: u32,
    pub text_dim: u32,
    pub text_accent: u32,
    pub text_error: u32,
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
            popup_bg: 0x003a_3f4b,
            menu_ink: 0x00d7_dae0,
            menu_key_ink: 0x00ab_b2bf,
            code_comment: 0x005c_6370,
            code_string: 0x0098_c379,
            code_keyword: 0x00c6_78dd,
            code_number: 0x00d1_9a66,
            code_call: 0x0061_afef,
            code_paren: 0x0082_8997,
        }
    }
}
