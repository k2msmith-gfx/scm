//! **A key press, as the keymap names it** — `"C-x"`, `"M-r"`, `"return"`.
//! Moved from the modeler's `main.rs` for the standalone program (E4): both
//! hosts hand the shell the same descriptors, so a binding means the same
//! key in each.

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Normalize a key press into an Emacs-flavored descriptor for the Scheme
/// keymap: the base key (character keys lowercased via the shift-free
/// lookup, named keys by name) behind modifier prefixes in fixed order —
/// `C-` control, `M-` option/alt, `S-` shift, `s-` command/super. So ⌥R is
/// `"M-r"`, shift-space is `"S-space"`. Returns None for keys the keymap
/// doesn't speak (bare modifiers, IME keys, exotic named keys).
pub fn descriptor(base: &Key, mods: ModifiersState) -> Option<String> {
    let name = match base {
        Key::Character(c) => {
            let c = c.to_lowercase();
            // A quote or backslash key would break out of the dispatch
            // form's string literal; escape rather than drop.
            match c.as_str() {
                "\"" => "\\\"".to_string(),
                "\\" => "\\\\".to_string(),
                _ => c,
            }
        }
        Key::Named(n) => match n {
            NamedKey::Space => "space".into(),
            NamedKey::Tab => "tab".into(),
            NamedKey::Enter => "return".into(),
            NamedKey::Backspace => "backspace".into(),
            NamedKey::Delete => "delete".into(),
            NamedKey::Home => "home".into(),
            NamedKey::End => "end".into(),
            NamedKey::PageUp => "pageup".into(),
            NamedKey::PageDown => "pagedown".into(),
            NamedKey::ArrowUp => "up".into(),
            NamedKey::ArrowDown => "down".into(),
            NamedKey::ArrowLeft => "left".into(),
            NamedKey::ArrowRight => "right".into(),
            NamedKey::F1 => "f1".into(),
            NamedKey::F2 => "f2".into(),
            NamedKey::F3 => "f3".into(),
            NamedKey::F4 => "f4".into(),
            NamedKey::F5 => "f5".into(),
            NamedKey::F6 => "f6".into(),
            NamedKey::F7 => "f7".into(),
            NamedKey::F8 => "f8".into(),
            NamedKey::F9 => "f9".into(),
            NamedKey::F10 => "f10".into(),
            NamedKey::F11 => "f11".into(),
            NamedKey::F12 => "f12".into(),
            _ => return None,
        },
        _ => return None,
    };
    let mut d = String::new();
    if mods.control_key() {
        d.push_str("C-");
    }
    if mods.alt_key() {
        d.push_str("M-");
    }
    if mods.shift_key() {
        d.push_str("S-");
    }
    if mods.super_key() {
        d.push_str("s-");
    }
    d.push_str(&name);
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_descriptors_normalize_case_modifiers_and_names() {
        let none = ModifiersState::empty();
        let ch = |s: &str| Key::Character(s.into());
        assert_eq!(descriptor(&ch("r"), none).unwrap(), "r");
        assert_eq!(descriptor(&ch("R"), none).unwrap(), "r");
        assert_eq!(
            descriptor(&ch("r"), ModifiersState::ALT).unwrap(),
            "M-r"
        );
        assert_eq!(
            descriptor(&ch("r"), ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT)
                .unwrap(),
            "C-M-S-r"
        );
        assert_eq!(descriptor(&Key::Named(NamedKey::Space), none).unwrap(), "space");
        assert_eq!(
            descriptor(&Key::Named(NamedKey::F5), ModifiersState::SUPER).unwrap(),
            "s-f5"
        );
        // Quote/backslash keys can't break out of the dispatch string.
        assert_eq!(descriptor(&ch("\""), none).unwrap(), "\\\"");
        assert_eq!(descriptor(&ch("\\"), none).unwrap(), "\\\\");
        // Bare modifiers and exotic keys don't dispatch.
        assert!(descriptor(&Key::Named(NamedKey::Shift), none).is_none());
    }
}
