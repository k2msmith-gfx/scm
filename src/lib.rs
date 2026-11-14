//! **The editor, as a crate** (docs/editor-crate-plan.md). One editor, two
//! hosts: the modeler links it today, and a standalone program will (E4), so
//! an improvement made here reaches both.
//!
//! E1 is the widget: the CPU [`framebuffer`] everything draws into, the glyph
//! painter ([`text`]), the transcript/input/popup panes ([`textpane`]) and
//! the editor buffer itself ([`editorpane`]). They reach the program around
//! them only through [`host::Host`] — dirty flags, a pane's rect, the UI
//! scale, keyboard focus and the colours in [`theme::Theme`] — so nothing in
//! this crate knows what a 3D pane, a scene or a status bar is. What the
//! editor knows about the text it edits is a [`language::Language`], and
//! what Scheme reaches it through is [`scm_binding`] (E2), the one module
//! that names the VM.

pub mod app;
pub mod host;
pub mod keys;
pub mod language;
pub mod scm_binding;
pub mod theme;
pub mod widget;

pub use widget::{editorpane, framebuffer, termpane, text, textpane};

#[cfg(test)]
mod layering;
