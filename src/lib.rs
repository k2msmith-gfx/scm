//! **The editor, as a crate** (docs/editor-crate-plan.md). One editor, two
//! hosts: the modeler links it today, and a standalone program will (E4), so
//! an improvement made here reaches both.
//!
//! E1 is the widget: the CPU [`framebuffer`] everything draws into, the glyph
//! painter ([`text`]), the transcript/input/popup panes ([`textpane`]) and
//! the editor buffer itself ([`editorpane`]). They reach the program around
//! them only through [`host::Host`] — dirty flags, a pane's rect, the UI
//! scale, keyboard focus and the colours in [`theme::Theme`] — so nothing in
//! this crate knows what a 3D pane, a scene or a status bar is.

pub mod host;
pub mod theme;
pub mod widget;

pub use widget::{editorpane, framebuffer, text, textpane};

#[cfg(test)]
mod layering;
