//! The path bar: a single-line Edit control showing the current directory.

use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use super::{send, wide};

pub fn create(parent: HWND, hinstance: HINSTANCE, id: usize) -> windows::core::Result<HWND> {
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
    unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            windows::Win32::UI::Controls::WC_EDITW,
            PCWSTR::null(),
            style,
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

pub fn text(edit: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(edit).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(edit, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

pub fn set_text(edit: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        let _ = SetWindowTextW(edit, PCWSTR(w.as_ptr()));
    }
}

pub fn select_all(edit: HWND) {
    send(edit, EM_SETSEL, 0, -1);
}
