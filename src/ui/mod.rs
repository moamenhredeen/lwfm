pub mod list;
pub mod pathbar;
pub mod searchbox;
pub mod statusbar;
pub mod window;

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

/// NUL-terminated UTF-16.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

pub fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

/// Scales a 96-DPI pixel value to `dpi`.
pub fn scale(px: i32, dpi: u32) -> i32 {
    (px as i64 * dpi as i64 / 96) as i32
}
