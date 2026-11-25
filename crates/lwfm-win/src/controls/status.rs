//! The status bar: two text parts along the bottom of the window.

use std::rc::Rc;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Controls::{SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP, STATUSCLASSNAMEW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SIZE, WS_CHILD, WS_VISIBLE,
};

use super::create_child;
use crate::error::Error;
use crate::msg::{send, send_slice};
use crate::wide::wide;
use crate::window::{Window, WindowState};

/// A status bar with two parts: a fixed-width first part and a second part
/// taking the rest.
pub struct StatusBar {
    hwnd: HWND,
    window: Rc<WindowState>,
}

impl StatusBar {
    /// Creates the status bar as a child of `window`.
    ///
    /// # Errors
    ///
    /// Fails when the control cannot be created.
    pub fn new(window: &Window, id: usize) -> Result<StatusBar, Error> {
        let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SBARS_SIZEGRIP);
        let hwnd = create_child(window, STATUSCLASSNAMEW, WINDOW_EX_STYLE(0), style, id)?;
        window.state().adopt(hwnd);
        Ok(StatusBar {
            hwnd,
            window: window.state().clone(),
        })
    }

    /// Re-positions the bar at the bottom of the window and returns its
    /// height. `first_part_width` is in pixels at 96 DPI.
    #[must_use]
    pub fn layout(&self, first_part_width: i32) -> i32 {
        // The bar positions itself when told that its parent was resized.
        send(self.hwnd, WM_SIZE, 0, 0);
        // -1 extends the last part to the right edge.
        let parts = [self.window.dpi().scale(first_part_width), -1];
        send_slice(self.hwnd, SB_SETPARTS, parts.len(), &parts);
        let mut rect = RECT::default();
        // SAFETY: `rect` is writable.
        let _ = unsafe { GetWindowRect(self.hwnd, &raw mut rect) };
        rect.bottom - rect.top
    }

    pub fn set_part(&self, part: usize, text: &str) {
        let text = wide(text);
        // The control copies the NUL-terminated text.
        send_slice(self.hwnd, SB_SETTEXTW, part, &text);
    }
}
