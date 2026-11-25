//! Keyboard input: what the message loop offers to the application before a
//! control sees it.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, MAPVK_VK_TO_CHAR, MapVirtualKeyExW, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT,
    VK_LCONTROL, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RMENU,
    VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_CHAR, WM_KEYDOWN, WM_SYSKEYDOWN};

use crate::num::loword;

/// Which modifier keys are held.
#[derive(Clone, Copy, Default, Debug)]
pub struct ModState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Right Alt with the synthetic left Ctrl that `AltGr` layouts inject.
    pub altgr: bool,
}

impl ModState {
    /// Reads the modifier state of the calling thread's keyboard, as of the
    /// message being processed.
    #[must_use]
    pub fn current() -> ModState {
        // SAFETY: no pointer arguments.
        let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(vk.0)) } < 0;
        ModState {
            ctrl: down(VK_CONTROL),
            alt: down(VK_MENU),
            shift: down(VK_SHIFT),
            win: down(VK_LWIN) || down(VK_RWIN),
            altgr: down(VK_RMENU) && down(VK_LCONTROL),
        }
    }
}

/// A key that has a name instead of a character.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NamedKey {
    /// F1 to F24.
    F(u8),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Space,
}

/// A key identified by its position-independent virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VirtualKey(u16);

impl VirtualKey {
    /// The key with the given Win32 virtual-key code. Meant for tests and
    /// tables; application logic should not depend on the numbering.
    #[must_use]
    pub const fn from_raw(code: u16) -> VirtualKey {
        VirtualKey(code)
    }

    /// The lowercase letter or digit of a letter or digit key. These keys
    /// have the same code on every layout.
    #[must_use]
    pub fn ascii_alphanumeric(self) -> Option<char> {
        match u8::try_from(self.0) {
            Ok(code @ (b'0'..=b'9' | b'A'..=b'Z')) => Some(char::from(code).to_ascii_lowercase()),
            _ => None,
        }
    }

    /// The unshifted character of this key on the current keyboard layout.
    #[must_use]
    pub fn layout_char(self) -> Option<char> {
        // SAFETY: no pointer arguments; thread 0 is the calling thread.
        let layout = unsafe { GetKeyboardLayout(0) };
        // SAFETY: no pointer arguments.
        let mapped = unsafe { MapVirtualKeyExW(u32::from(self.0), MAPVK_VK_TO_CHAR, Some(layout)) };
        // The high bit marks dead keys; the low word is still the character.
        char::from_u32(mapped & 0xFFFF).filter(|c| *c != '\0')
    }

    fn named(self) -> Option<NamedKey> {
        let vk = VIRTUAL_KEY(self.0);
        Some(match vk {
            VK_RETURN => NamedKey::Enter,
            VK_ESCAPE => NamedKey::Esc,
            VK_TAB => NamedKey::Tab,
            VK_BACK => NamedKey::Backspace,
            VK_DELETE => NamedKey::Delete,
            VK_INSERT => NamedKey::Insert,
            VK_HOME => NamedKey::Home,
            VK_END => NamedKey::End,
            VK_PRIOR => NamedKey::PageUp,
            VK_NEXT => NamedKey::PageDown,
            VK_UP => NamedKey::Up,
            VK_DOWN => NamedKey::Down,
            VK_LEFT => NamedKey::Left,
            VK_RIGHT => NamedKey::Right,
            VK_SPACE => NamedKey::Space,
            _ if (VK_F1.0..=VK_F24.0).contains(&vk.0) => {
                NamedKey::F(u8::try_from(vk.0 - VK_F1.0 + 1).unwrap_or(u8::MAX))
            }
            _ => return None,
        })
    }
}

/// The key of a key-down.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyCode {
    Named(NamedKey),
    /// Any other key, including lone modifiers.
    Other(VirtualKey),
}

impl From<VirtualKey> for KeyCode {
    fn from(vk: VirtualKey) -> KeyCode {
        vk.named().map_or(KeyCode::Other(vk), KeyCode::Named)
    }
}

/// A keyboard message, offered before it is dispatched to a control.
#[derive(Clone, Copy, Debug)]
pub enum KeyEvent {
    /// A key went down (or repeats), with the modifiers held at that time.
    Down { key: KeyCode, mods: ModState },
    /// The keyboard layout produced a character, with shift, `AltGr` and
    /// dead keys already applied. Alt+key characters are not reported: their
    /// key-down was.
    Char(char),
}

/// The key event of a queued message, if it is a keyboard message.
pub(crate) fn key_event(msg: &MSG) -> Option<KeyEvent> {
    match msg.message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyEvent::Down {
            key: VirtualKey(loword(msg.wParam.0)).into(),
            mods: ModState::current(),
        }),
        // WM_SYSCHAR (alt+key) is deliberately ignored.
        WM_CHAR => char::from_u32(u32::from(loword(msg.wParam.0))).map(KeyEvent::Char),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        let key = |vk: VIRTUAL_KEY| KeyCode::from(VirtualKey(vk.0));
        assert_eq!(key(VK_RETURN), KeyCode::Named(NamedKey::Enter));
        assert_eq!(key(VK_F1), KeyCode::Named(NamedKey::F(1)));
        assert_eq!(key(VK_F24), KeyCode::Named(NamedKey::F(24)));
        assert_eq!(key(VK_CONTROL), KeyCode::Other(VirtualKey(VK_CONTROL.0)));
    }

    #[test]
    fn letters_and_digits() {
        assert_eq!(VirtualKey(0x4E).ascii_alphanumeric(), Some('n'));
        assert_eq!(VirtualKey(0x38).ascii_alphanumeric(), Some('8'));
        assert_eq!(VirtualKey(VK_RETURN.0).ascii_alphanumeric(), None);
        assert_eq!(VirtualKey(0x1234).ascii_alphanumeric(), None);
    }
}
