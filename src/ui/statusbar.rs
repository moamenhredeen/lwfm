//! The status bar: item counts on the left, the latest message on the right.

use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
use windows::Win32::UI::Controls::{SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP, STATUSCLASSNAMEW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use super::{scale, send, wide};

pub fn create(parent: HWND, hinstance: HINSTANCE, id: usize) -> windows::core::Result<HWND> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            STATUSCLASSNAMEW,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SBARS_SIZEGRIP),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as _)),
            Some(hinstance),
            None,
        )
    }
}

/// Re-positions the bar at the bottom of the parent and returns its height.
pub fn layout(status: HWND, dpi: u32) -> i32 {
    send(status, WM_SIZE, 0, 0);
    let parts: [i32; 2] = [scale(280, dpi), -1];
    send(status, SB_SETPARTS, parts.len(), parts.as_ptr() as isize);
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(status, &mut r);
    }
    r.bottom - r.top
}

pub fn set_part(status: HWND, part: usize, text: &str) {
    let w = wide(text);
    send(status, SB_SETTEXTW, part, w.as_ptr() as isize);
}
