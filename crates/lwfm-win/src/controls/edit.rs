//! A single-line text input.

use std::rc::Rc;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{EM_SETSEL, WC_EDITW};
use windows::Win32::UI::WindowsAndMessaging::{
    ES_AUTOHSCROLL, GetWindowTextLengthW, GetWindowTextW, HWND_TOP, IsWindowVisible, SW_HIDE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_STYLE,
    WS_CHILD, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::PCWSTR;

use super::{create_child, has_focus, set_bounds, take_focus};
use crate::error::Error;
use crate::geom::Rect;
use crate::msg::send;
use crate::wide::wide;
use crate::window::{Window, WindowState};

/// Identifies an input among the window's controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EditId(pub usize);

/// What happened to an input. See the [module documentation](super) for
/// which changes are reported.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditEvent {
    /// The user changed the text.
    Changed,
    /// The input lost the keyboard focus.
    LostFocus,
}

/// A single-line text input that scrolls horizontally.
pub struct Edit {
    hwnd: HWND,
    window: Rc<WindowState>,
}

impl Edit {
    /// Creates the input as a child of `window`. It lies below the controls
    /// created before it until [`show_on_top`](Edit::show_on_top) raises it.
    ///
    /// # Errors
    ///
    /// Fails when the control cannot be created.
    pub fn new(window: &Window, id: EditId, visible: bool) -> Result<Edit, Error> {
        let auto_scroll = WINDOW_STYLE(u32::try_from(ES_AUTOHSCROLL).unwrap_or(0));
        let mut style = WS_CHILD | WS_TABSTOP | auto_scroll;
        if visible {
            style |= WS_VISIBLE;
        }
        let hwnd = create_child(window, WC_EDITW, WS_EX_CLIENTEDGE, style, id.0)?;
        window.state().adopt_edit(hwnd, id);
        Ok(Edit {
            hwnd,
            window: window.state().clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub fn text(&self) -> String {
        // SAFETY: plain value.
        let len = unsafe { GetWindowTextLengthW(self.hwnd) };
        let mut buf = vec![0u16; usize::try_from(len).unwrap_or(0) + 1];
        // SAFETY: `buf` is writable for its whole length.
        let copied = unsafe { GetWindowTextW(self.hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..usize::try_from(copied).unwrap_or(0)])
    }

    /// Replaces the text. Not reported as [`EditEvent::Changed`].
    pub fn set_text(&self, text: &str) {
        let _quiet = self.window.quiet();
        let text = wide(text);
        // SAFETY: `text` is NUL-terminated and outlives the call.
        let _ = unsafe { SetWindowTextW(self.hwnd, PCWSTR(text.as_ptr())) };
    }

    pub fn select_all(&self) {
        send(self.hwnd, EM_SETSEL, 0, -1);
    }

    /// Places the caret after `units` UTF-16 units, with nothing selected.
    pub fn set_caret(&self, units: usize) {
        send(
            self.hwnd,
            EM_SETSEL,
            units,
            isize::try_from(units).unwrap_or(isize::MAX),
        );
    }

    /// Shows the input above its sibling controls.
    pub fn show_on_top(&self) {
        let _quiet = self.window.quiet();
        // SAFETY: plain values; the window belongs to this thread.
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            )
        };
    }

    /// Hides the input. Losing the focus this way is not reported.
    pub fn hide(&self) {
        let _quiet = self.window.quiet();
        // SAFETY: plain values.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        // SAFETY: plain value.
        unsafe { IsWindowVisible(self.hwnd) }.as_bool()
    }

    #[must_use]
    pub fn has_focus(&self) -> bool {
        has_focus(self.hwnd)
    }

    /// Gives the input the keyboard focus.
    pub fn focus_control(&self) {
        take_focus(self.hwnd);
    }

    pub fn set_bounds(&self, bounds: Rect) {
        set_bounds(self.hwnd, bounds);
    }
}
