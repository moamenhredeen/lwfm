//! The find-as-you-type input: a hidden single-line Edit shown over the
//! list's bottom-left corner while a search is active.

use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

pub const WIDTH: i32 = 240;
pub const HEIGHT: i32 = 24;

pub fn create(parent: HWND, hinstance: HINSTANCE, id: usize) -> windows::core::Result<HWND> {
    let style = WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
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

pub fn show(edit: HWND) {
    unsafe {
        let _ = SetWindowPos(
            edit,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
    }
}

pub fn hide(edit: HWND) {
    unsafe {
        let _ = ShowWindow(edit, SW_HIDE);
    }
}

pub fn is_visible(edit: HWND) -> bool {
    unsafe { IsWindowVisible(edit).as_bool() }
}
