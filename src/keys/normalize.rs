//! Turns Win32 keyboard messages into chords.
//!
//! Named keys and ctrl/alt/win combinations are resolved from the virtual key
//! on `WM_KEYDOWN`. Everything else waits for `WM_CHAR`, so shift, AltGr and
//! dead keys are already applied by the keyboard layout.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

use super::chord::{Chord, Key, Mods, Named};

#[derive(Clone, Copy, Default, Debug)]
pub struct ModState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Right Alt with the synthetic left Ctrl that AltGr layouts inject.
    pub altgr: bool,
}

impl ModState {
    /// Reads the current modifier state of the calling thread's keyboard.
    pub fn current() -> ModState {
        let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk.0 as i32) } < 0;
        ModState {
            ctrl: down(VK_CONTROL),
            alt: down(VK_MENU),
            shift: down(VK_SHIFT),
            win: down(VK_LWIN) || down(VK_RWIN),
            altgr: down(VK_RMENU) && down(VK_LCONTROL),
        }
    }
}

fn named_from_vk(vk: u16) -> Option<Named> {
    let vk = VIRTUAL_KEY(vk);
    Some(match vk {
        VK_RETURN => Named::Enter,
        VK_ESCAPE => Named::Esc,
        VK_TAB => Named::Tab,
        VK_BACK => Named::Backspace,
        VK_DELETE => Named::Delete,
        VK_INSERT => Named::Insert,
        VK_HOME => Named::Home,
        VK_END => Named::End,
        VK_PRIOR => Named::PageUp,
        VK_NEXT => Named::PageDown,
        VK_UP => Named::Up,
        VK_DOWN => Named::Down,
        VK_LEFT => Named::Left,
        VK_RIGHT => Named::Right,
        VK_SPACE => Named::Space,
        _ if (VK_F1.0..=VK_F24.0).contains(&vk.0) => Named::F((vk.0 - VK_F1.0 + 1) as u8),
        _ => return None,
    })
}

/// Chord for a key-down, or `None` if the key must be resolved from the
/// following `WM_CHAR` (or is a lone modifier). `vk_char` maps a virtual key
/// to its unshifted character on the active layout.
pub fn from_keydown(vk: u16, st: ModState, vk_char: &dyn Fn(u16) -> Option<char>) -> Option<Chord> {
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

    if let Some(named) = named_from_vk(vk) {
        return Some(Chord::new(mods, Key::Named(named)));
    }
    if !(ctrl || alt || st.win) {
        return None;
    }
    let c = match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_ascii_lowercase(),
        _ => vk_char(vk)?.to_lowercase().next()?,
    };
    Some(Chord::new(mods, Key::Char(c)))
}

/// Chord for a `WM_CHAR` code unit. Control characters and space are not
/// character chords (they arrive as named keys or ctrl combinations).
pub fn from_char(unit: u16) -> Option<Chord> {
    let c = char::from_u32(unit as u32)?;
    if c.is_control() || c == ' ' {
        return None;
    }
    Some(Chord::new(Mods::NONE, Key::Char(c)))
}

/// Unshifted character of a virtual key on the current keyboard layout.
pub fn layout_char(vk: u16) -> Option<char> {
    let r = unsafe { MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_CHAR, Some(GetKeyboardLayout(0))) };
    // The high bit marks dead keys; the low word is still the character.
    char::from_u32(r & 0xFFFF).filter(|c| *c != '\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_char(_: u16) -> Option<char> {
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

    fn down(vk: VIRTUAL_KEY, s: ModState) -> Option<String> {
        from_keydown(vk.0, s, &no_char).map(|c| c.to_string())
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
        assert_eq!(from_char('J' as u16).unwrap().to_string(), "J");
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
        // ... the layout then sends WM_CHAR '['.
        assert_eq!(from_char('[' as u16).unwrap().to_string(), "[");
    }

    #[test]
    fn punctuation_via_layout() {
        let comma = |vk: u16| (vk == VK_OEM_COMMA.0).then_some(',');
        let c = from_keydown(VK_OEM_COMMA.0, st(true, false, false), &comma).unwrap();
        assert_eq!(c.to_string(), "ctrl+,");
    }

    #[test]
    fn lone_modifiers_and_control_chars() {
        assert_eq!(down(VK_CONTROL, st(true, false, false)), None);
        assert_eq!(from_char(0x08), None);
        assert_eq!(from_char(' ' as u16), None);
    }
}
