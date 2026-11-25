//! The top-level window.

use windows::Win32::Foundation::HWND;

/// Identifies a window from any thread, for calls that accept a window
/// handle off the UI thread (posting a wake-up, owning a shell dialog).
#[derive(Clone, Copy, Debug)]
pub struct WindowToken(isize);

impl WindowToken {
    /// Temporary bridge while the application still creates its own window.
    #[doc(hidden)]
    #[must_use]
    pub fn from_raw(hwnd: isize) -> WindowToken {
        WindowToken(hwnd)
    }

    pub(crate) fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}
