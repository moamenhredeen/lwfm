//! Turns keyboard events into chords.
//!
//! Named keys and ctrl/alt/win combinations are resolved from the key on
//! key-down. Everything else waits for the character event, so shift, `AltGr`
//! and dead keys are already applied by the keyboard layout.

use lwfm_win::input::{KeyCode, ModState, NamedKey, VirtualKey};

use super::chord::{Chord, Key, Mods, Named};

fn named(key: NamedKey) -> Named {
    match key {
        NamedKey::F(n) => Named::F(n),
        NamedKey::Enter => Named::Enter,
        NamedKey::Esc => Named::Esc,
        NamedKey::Tab => Named::Tab,
        NamedKey::Backspace => Named::Backspace,
        NamedKey::Delete => Named::Delete,
        NamedKey::Insert => Named::Insert,
        NamedKey::Home => Named::Home,
        NamedKey::End => Named::End,
        NamedKey::PageUp => Named::PageUp,
        NamedKey::PageDown => Named::PageDown,
        NamedKey::Up => Named::Up,
        NamedKey::Down => Named::Down,
        NamedKey::Left => Named::Left,
        NamedKey::Right => Named::Right,
        NamedKey::Space => Named::Space,
    }
}

/// Chord for a key-down, or `None` if the key must be resolved from the
/// following character event (or is a lone modifier). `layout_char` maps a
/// key to its unshifted character on the active layout.
pub fn from_keydown(
    key: KeyCode,
    st: ModState,
    layout_char: &dyn Fn(VirtualKey) -> Option<char>,
) -> Option<Chord> {
    // With AltGr, the ctrl+alt state belongs to AltGr, not to the chord.
    let (ctrl, alt) = if st.altgr {
        (false, false)
    } else {
        (st.ctrl, st.alt)
    };
    let mut mods = Mods::NONE;
    for (on, m) in [
        (ctrl, Mods::CTRL),
        (alt, Mods::ALT),
        (st.shift, Mods::SHIFT),
        (st.win, Mods::WIN),
    ] {
        if on {
            mods = mods.with(m);
        }
    }

    let vk = match key {
        KeyCode::Named(n) => return Some(Chord::new(mods, Key::Named(named(n)))),
        KeyCode::Other(vk) => vk,
    };
    if !(ctrl || alt || st.win) {
        return None;
    }
    // Letters and digits are the same keys on every layout; punctuation is not.
    let c = match vk.ascii_alphanumeric() {
        Some(c) => c,
        None => layout_char(vk)?.to_lowercase().next()?,
    };
    Some(Chord::new(mods, Key::Char(c)))
}

/// Chord for a typed character. Control characters and space are not
/// character chords (they arrive as named keys or ctrl combinations).
pub fn from_char(c: char) -> Option<Chord> {
    if c.is_control() || c == ' ' {
        return None;
    }
    Some(Chord::new(Mods::NONE, Key::Char(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Win32 virtual-key codes of the keys used below.
    const VK_BACK: u16 = 0x08;
    const VK_CONTROL: u16 = 0x11;
    const VK_LEFT: u16 = 0x25;
    const VK_8: u16 = 0x38;
    const VK_J: u16 = 0x4A;
    const VK_N: u16 = 0x4E;
    const VK_F5: u16 = 0x74;
    const VK_OEM_COMMA: u16 = 0xBC;

    fn no_char(_: VirtualKey) -> Option<char> {
        None
    }

    fn st(ctrl: bool, alt: bool, shift: bool) -> ModState {
        ModState {
            ctrl,
            alt,
            shift,
            ..ModState::default()
        }
    }

    fn key(vk: u16) -> KeyCode {
        VirtualKey::from_raw(vk).into()
    }

    fn down(vk: u16, s: ModState) -> Option<String> {
        from_keydown(key(vk), s, &no_char).map(|c| c.to_string())
    }

    #[test]
    fn ctrl_shift_letter() {
        assert_eq!(down(VK_N, st(true, false, true)).unwrap(), "ctrl+shift+n");
    }

    #[test]
    fn named_keys_carry_modifiers() {
        assert_eq!(down(VK_LEFT, st(false, true, false)).unwrap(), "alt+left");
        assert_eq!(down(VK_F5, st(false, false, false)).unwrap(), "f5");
        assert_eq!(
            down(VK_BACK, st(false, false, true)).unwrap(),
            "shift+backspace"
        );
    }

    #[test]
    fn plain_and_shifted_letters_wait_for_char() {
        assert_eq!(down(VK_J, st(false, false, true)), None);
        assert_eq!(down(VK_J, st(false, false, false)), None);
        assert_eq!(from_char('J').unwrap().to_string(), "J");
    }

    #[test]
    fn altgr_produces_character_chord() {
        let altgr = ModState {
            ctrl: true,
            alt: true,
            altgr: true,
            ..ModState::default()
        };
        // AltGr+8 on a German layout: no chord on key-down ...
        assert_eq!(down(VK_8, altgr), None);
        // ... the layout then sends the character '['.
        assert_eq!(from_char('[').unwrap().to_string(), "[");
    }

    #[test]
    fn punctuation_via_layout() {
        let comma = |vk: VirtualKey| (vk == VirtualKey::from_raw(VK_OEM_COMMA)).then_some(',');
        let c = from_keydown(key(VK_OEM_COMMA), st(true, false, false), &comma).unwrap();
        assert_eq!(c.to_string(), "ctrl+,");
    }

    #[test]
    fn lone_modifiers_and_control_chars() {
        assert_eq!(down(VK_CONTROL, st(true, false, false)), None);
        assert_eq!(from_char('\u{8}'), None);
        assert_eq!(from_char(' '), None);
    }
}
