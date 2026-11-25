//! GDI resources and drawing, private to the crate. All of it is UI-thread
//! only: the owners hold raw GDI handles and are therefore not `Send`.

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOWTEXT, CreateFontIndirectW, CreateSolidBrush,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, FillRect,
    GetSysColor, GetTextExtentPoint32W, HBRUSH, HDC, HFONT, HGDIOBJ, LOGFONTW, SYS_COLOR_INDEX,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETHIGHCONTRAST, SPI_GETNONCLIENTMETRICS,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};

use crate::num::size_u32;

/// A font deleted when dropped.
pub(crate) struct Font(HFONT);

impl Font {
    pub(crate) fn raw(&self) -> HFONT {
        self.0
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: the font is valid and owned by `self`. Callers replace the
        // font of every control before dropping the old one.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

/// A solid brush deleted when dropped.
pub(crate) struct Brush(HBRUSH);

impl Brush {
    pub(crate) fn solid(color: COLORREF) -> Brush {
        // SAFETY: no pointer arguments.
        Brush(unsafe { CreateSolidBrush(color) })
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        // SAFETY: the brush is valid, owned by `self` and selected nowhere.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

/// The system's message font at a DPI, with the description it was made from.
pub(crate) struct MessageFont {
    pub(crate) description: LOGFONTW,
    pub(crate) font: Font,
}

/// The description of the system message font scaled for `dpi`.
pub(crate) fn message_font_description(dpi: u32) -> Option<LOGFONTW> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_u32::<NONCLIENTMETRICSW>(),
        ..Default::default()
    };
    // SAFETY: `metrics` is a writable NONCLIENTMETRICSW with its size set,
    // which is what SPI_GETNONCLIENTMETRICS fills.
    unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some((&raw mut metrics).cast()),
            0,
            dpi,
        )
    }
    .ok()?;
    Some(metrics.lfMessageFont)
}

pub(crate) fn create_font(description: LOGFONTW) -> MessageFont {
    // SAFETY: `description` is a valid LOGFONTW for the duration of the call.
    let font = unsafe { CreateFontIndirectW(&raw const description) };
    MessageFont {
        description,
        font: Font(font),
    }
}

/// Whether a high-contrast theme is active.
pub(crate) fn high_contrast() -> bool {
    let mut info = HIGHCONTRASTW {
        cbSize: size_u32::<HIGHCONTRASTW>(),
        ..Default::default()
    };
    // SAFETY: `info` is a writable HIGHCONTRASTW with its size set, which is
    // what SPI_GETHIGHCONTRAST fills.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some((&raw mut info).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && info.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0
}

fn system_color(index: SYS_COLOR_INDEX) -> COLORREF {
    // SAFETY: no pointer arguments.
    COLORREF(unsafe { GetSysColor(index) })
}

pub(crate) fn window_text_color() -> COLORREF {
    system_color(COLOR_WINDOWTEXT)
}

pub(crate) fn highlight_color() -> COLORREF {
    system_color(COLOR_HIGHLIGHT)
}

pub(crate) fn highlight_text_color() -> COLORREF {
    system_color(COLOR_HIGHLIGHTTEXT)
}

/// Draws on a device context that a control handed out for the duration of
/// a paint notification.
pub(crate) struct Painter {
    hdc: HDC,
}

impl Painter {
    /// # Safety
    ///
    /// `hdc` must be a valid device context for as long as the painter lives.
    pub(crate) unsafe fn new(hdc: HDC) -> Painter {
        Painter { hdc }
    }

    pub(crate) fn transparent_background(&self) {
        // SAFETY: the device context is valid (see `new`).
        unsafe { SetBkMode(self.hdc, TRANSPARENT) };
    }

    pub(crate) fn set_text_color(&self, color: COLORREF) {
        // SAFETY: the device context is valid (see `new`).
        unsafe { SetTextColor(self.hdc, color) };
    }

    /// Extent of `text` in the selected font.
    pub(crate) fn measure(&self, text: &[u16]) -> SIZE {
        let mut size = SIZE::default();
        // SAFETY: the device context is valid and `size` is writable.
        let _ = unsafe { GetTextExtentPoint32W(self.hdc, text, &raw mut size) };
        size
    }

    /// Draws `text` on one line, vertically centered, from the left edge of `rect`.
    pub(crate) fn text(&self, text: &mut [u16], mut rect: RECT) {
        let format = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
        // SAFETY: the device context is valid and `rect` is writable; these
        // flags do not make the call modify `text`.
        unsafe { DrawTextW(self.hdc, text, &raw mut rect, format) };
    }

    pub(crate) fn fill(&self, rect: RECT, brush: &Brush) {
        // SAFETY: the device context and the brush are valid.
        unsafe { FillRect(self.hdc, &raw const rect, brush.0) };
    }
}
