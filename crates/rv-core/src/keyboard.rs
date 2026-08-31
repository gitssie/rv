//! Client-side RFB keyboard state.
//!
//! GPUI (especially on macOS) can deliver the same physical key twice: once from
//! `NSKeyDown` and again from the IME `doCommandBySelector:` fallback. RFB treats
//! a second KeyEvent-down as another character, so we remember what is already
//! down and only emit auto-repeat when the OS marks the event as held.

use std::collections::HashMap;

use crate::keysym::{XK_ALT_L, XK_CONTROL_L, XK_SUPER_L, keysym_of};

/// Latin-1 printable range accepted as RFB keysyms (RFC 6143 §7.5.4).
fn latin1_printable(c: char) -> Option<u32> {
    let u = c as u32;
    if (0x20..=0xFF).contains(&u) {
        Some(u)
    } else {
        None
    }
}

/// Resolve a GPUI keystroke to an X11 keysym.
///
/// Prefer the produced character when it is a single Latin-1 glyph so layout
/// (Dvorak, AZERTY, …) matches what the user typed. Named keys (`enter`,
/// `left`, …) still go through [`keysym_of`].
pub fn keysym_for_keystroke(key: &str, key_char: Option<&str>, shift: bool) -> Option<u32> {
    if let Some(ch) = key_char {
        let mut chars = ch.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(ks) = latin1_printable(c)
        {
            return Some(ks);
        }
    }
    keysym_of(key, shift)
}

fn is_modifier_name(key: &str) -> bool {
    matches!(
        key,
        "shift"
            | "leftshift"
            | "rightshift"
            | "control"
            | "ctrl"
            | "leftcontrol"
            | "rightcontrol"
            | "rightctrl"
            | "alt"
            | "option"
            | "leftalt"
            | "leftoption"
            | "rightalt"
            | "rightoption"
            | "altgr"
            | "meta"
            | "command"
            | "cmd"
            | "super"
            | "windows"
            | "win"
            | "leftmeta"
            | "rightmeta"
            | "rightcommand"
            | "rightsuper"
            | "function"
            | "platform"
    )
}

/// Tracks keys currently held so each physical press becomes one RFB down/up pair.
#[derive(Default)]
pub struct Keyboard {
    /// Physical key name (lowercased GPUI `keystroke.key`) → last sent keysym.
    down: HashMap<String, u32>,
    control: bool,
    alt: bool,
    super_key: bool,
}

impl Keyboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Emit Ctrl/Alt/Super edges. Call this from both key events and
    /// `ModifiersChanged` so modifiers are not left stuck when the OS only
    /// reports `flagsChanged`.
    pub fn set_modifiers(&mut self, control: bool, alt: bool, super_key: bool) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        edge(&mut out, &mut self.control, control, XK_CONTROL_L);
        edge(&mut out, &mut self.alt, alt, XK_ALT_L);
        edge(&mut out, &mut self.super_key, super_key, XK_SUPER_L);
        out
    }

    /// Key-down. Returns RFB (keysym, down) events to send.
    ///
    /// Duplicate downs for the same physical key are ignored unless `is_held`
    /// (OS auto-repeat), which RFC 6143 represents as another down without up.
    pub fn key_down(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        shift: bool,
        is_held: bool,
    ) -> Vec<(u32, bool)> {
        let key = key.to_ascii_lowercase();
        if is_modifier_name(&key) {
            return Vec::new();
        }
        let Some(ks) = keysym_for_keystroke(&key, key_char, shift) else {
            return Vec::new();
        };
        if is_held {
            return vec![(ks, true)];
        }
        if self.down.contains_key(&key) {
            return Vec::new();
        }
        self.down.insert(key, ks);
        vec![(ks, true)]
    }

    /// Key-up. Releases the keysym that was sent for this physical key.
    pub fn key_up(&mut self, key: &str) -> Vec<(u32, bool)> {
        let key = key.to_ascii_lowercase();
        if is_modifier_name(&key) {
            return Vec::new();
        }
        match self.down.remove(&key) {
            Some(ks) => vec![(ks, false)],
            None => Vec::new(),
        }
    }

    pub fn recognizes(key: &str, key_char: Option<&str>) -> bool {
        let key = key.to_ascii_lowercase();
        if is_modifier_name(&key) {
            return true;
        }
        keysym_for_keystroke(&key, key_char, false).is_some()
            || keysym_for_keystroke(&key, key_char, true).is_some()
    }

    pub fn release_all(&mut self) -> Vec<(u32, bool)> {
        let mut out: Vec<(u32, bool)> = self.down.drain().map(|(_, ks)| (ks, false)).collect();
        out.extend(self.set_modifiers(false, false, false));
        out
    }
}

fn edge(out: &mut Vec<(u32, bool)>, was: &mut bool, now: bool, ks: u32) {
    if *was != now {
        *was = now;
        out.push((ks, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keysym::XK_RETURN;

    #[test]
    fn duplicate_down_is_ignored() {
        let mut kb = Keyboard::new();
        assert_eq!(
            kb.key_down("a", Some("a"), false, false),
            vec![(b'a' as u32, true)]
        );
        assert_eq!(kb.key_down("a", Some("a"), false, false), Vec::new());
        assert_eq!(kb.key_up("a"), vec![(b'a' as u32, false)]);
        assert_eq!(kb.key_up("a"), Vec::new());
    }

    #[test]
    fn held_repeat_sends_another_down() {
        let mut kb = Keyboard::new();
        assert_eq!(
            kb.key_down("a", Some("a"), false, false),
            vec![(b'a' as u32, true)]
        );
        assert_eq!(
            kb.key_down("a", Some("a"), false, true),
            vec![(b'a' as u32, true)]
        );
        assert_eq!(kb.key_up("a"), vec![(b'a' as u32, false)]);
    }

    #[test]
    fn key_char_latin1_preferred() {
        assert_eq!(
            keysym_for_keystroke("oem_1", Some(";"), false),
            Some(u32::from(b';'))
        );
        assert_eq!(
            keysym_for_keystroke("enter", Some("\n"), false),
            Some(XK_RETURN)
        );
    }

    #[test]
    fn modifiers_are_edges() {
        let mut kb = Keyboard::new();
        assert_eq!(
            kb.set_modifiers(true, false, false),
            vec![(XK_CONTROL_L, true)]
        );
        assert_eq!(kb.set_modifiers(true, false, false), Vec::new());
        assert_eq!(
            kb.set_modifiers(false, false, false),
            vec![(XK_CONTROL_L, false)]
        );
    }

    #[test]
    fn modifier_key_name_does_not_emit_keysym() {
        let mut kb = Keyboard::new();
        assert!(kb.key_down("control", None, false, false).is_empty());
        assert!(kb.key_down("shift", None, true, false).is_empty());
    }

    #[test]
    fn up_uses_original_keysym_even_if_shift_changed() {
        let mut kb = Keyboard::new();
        assert_eq!(
            kb.key_down("a", Some("A"), true, false),
            vec![(b'A' as u32, true)]
        );
        assert_eq!(kb.key_up("a"), vec![(b'A' as u32, false)]);
    }
}
