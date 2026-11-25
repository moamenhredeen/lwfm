//! A virtual list in report view: the control holds no rows, it asks its
//! [`ListSource`] for whatever it needs to draw.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT};
use windows::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows::Win32::UI::Controls::{
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NEWFONT,
    CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT, LIST_VIEW_ITEM_STATE_FLAGS,
    LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCFMT_LEFT, LVCFMT_RIGHT, LVCOLUMNW, LVIF_TEXT,
    LVIR_LABEL, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_ENSUREVISIBLE, LVM_GETCOLUMNWIDTH,
    LVM_GETITEMRECT, LVM_GETNEXTITEM, LVM_GETSELECTEDCOUNT, LVM_INSERTCOLUMNW, LVM_SETCOLUMNWIDTH,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVN_COLUMNCLICK,
    LVN_GETDISPINFOW, LVN_ITEMCHANGED, LVN_ODSTATECHANGED, LVNI_FOCUSED, LVNI_SELECTED,
    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS,
    LVSICF_NOSCROLL, NM_CUSTOMDRAW, NM_DBLCLK, NMCUSTOMDRAW_DRAW_STAGE, NMITEMACTIVATE, NMLISTVIEW,
    NMLVCUSTOMDRAW, NMLVDISPINFOW, SetWindowTheme, WC_LISTVIEWW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, PWSTR, w};

use super::{create_child, has_focus, set_bounds, take_focus};
use crate::error::Error;
use crate::gdi::{self, Brush, Painter};
use crate::geom::{Color, Rect, rescale};
use crate::msg::{param_mut, param_ref, send, send_mut, send_ref};
use crate::wide::wide;
use crate::window::{Window, WindowState};

/// Highlight behind the matched part of a name outside high-contrast themes.
const HIGHLIGHT: Color = Color::rgb(255, 230, 0);
/// Left margin of the text inside a label rectangle at 96 DPI, as the list
/// control draws it.
const LABEL_MARGIN: i32 = 2;
/// Room for a cell text that is drawn by this module: the longest file name
/// plus a suffix.
const SCRATCH_UNITS: usize = 520;

/// Identifies a list among the window's controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ListId(pub usize);

/// A column of a list.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub title: &'static str,
    /// Initial width in pixels at 96 DPI.
    pub width: i32,
    pub align_right: bool,
}

/// Where a [`ListSource`] writes the text of one cell. Text that does not
/// fit is cut off.
pub struct CellBuf<'a> {
    buf: &'a mut [u16],
    len: usize,
}

impl<'a> CellBuf<'a> {
    fn new(buf: &'a mut [u16]) -> CellBuf<'a> {
        CellBuf { buf, len: 0 }
    }

    pub fn push_unit(&mut self, unit: u16) {
        // One unit stays free for the terminator.
        if self.len + 1 < self.buf.len() {
            self.buf[self.len] = unit;
            self.len += 1;
        }
    }

    /// Appends UTF-16 text.
    pub fn push_units(&mut self, units: &[u16]) {
        let room = self.buf.len().saturating_sub(self.len + 1);
        let n = units.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&units[..n]);
        self.len += n;
    }

    pub fn push_str(&mut self, text: &str) {
        for unit in text.encode_utf16() {
            self.push_unit(unit);
        }
    }

    /// Terminates the text and returns its length in units.
    fn finish(self) -> usize {
        if let Some(end) = self.buf.get_mut(self.len) {
            *end = 0;
        }
        self.len
    }
}

/// What a list shows and what the user does with it. See the
/// [module documentation](super) for when these are called.
pub trait ListSource {
    /// Writes the text of a cell. Called for every cell that is painted.
    fn cell(&self, row: usize, column: usize, out: &mut CellBuf<'_>);

    /// Text color of a row; `None` for the default.
    fn row_color(&self, _row: usize) -> Option<Color> {
        None
    }

    /// The part of the first column's text to highlight, as a range of
    /// UTF-16 units. Only asked while highlighting is switched on.
    fn highlight(&self, _row: usize) -> Option<Range<usize>> {
        None
    }

    /// The user double-clicked a row.
    fn on_activate(&self, _row: usize) {}

    /// The user clicked a column header.
    fn on_column_click(&self, _column: usize) {}

    /// The user changed the selection or the focused row.
    fn on_selection_changed(&self) {}
}

/// The part of a list that the window procedure needs.
pub(crate) struct ListState {
    pub(crate) hwnd: HWND,
    pub(crate) id: ListId,
    columns: usize,
    highlighting: Cell<bool>,
    /// The highlight brush with the color it was made for.
    brush: RefCell<Option<(COLORREF, Brush)>>,
}

impl ListState {
    /// Rescales column widths after a DPI change, keeping user-resized widths.
    pub(crate) fn rescale_columns(&self, old_dpi: u32, new_dpi: u32) {
        for column in 0..self.columns {
            let width = send(self.hwnd, LVM_GETCOLUMNWIDTH, column, 0);
            let scaled = rescale(i32::try_from(width).unwrap_or(0), old_dpi, new_dpi);
            send(
                self.hwnd,
                LVM_SETCOLUMNWIDTH,
                column,
                isize::try_from(scaled).unwrap_or(0),
            );
        }
    }

    /// System colors changed: drop what was derived from them and repaint.
    pub(crate) fn colors_changed(&self) {
        self.brush.borrow_mut().take();
        invalidate(self.hwnd);
    }

    fn with_brush(&self, color: COLORREF, f: impl FnOnce(&Brush)) {
        let mut cached = self.brush.borrow_mut();
        let entry = match cached.take() {
            Some(entry) if entry.0 == color => entry,
            _ => (color, Brush::solid(color)),
        };
        f(&cached.insert(entry).1);
    }
}

fn invalidate(hwnd: HWND) {
    // SAFETY: plain values; a null rectangle means the whole client area.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

/// The virtual list control.
pub struct ListView {
    state: Rc<ListState>,
    window: Rc<WindowState>,
}

impl ListView {
    /// Creates the list as a child of `window`.
    ///
    /// # Errors
    ///
    /// Fails when the control cannot be created.
    pub fn new(window: &Window, id: ListId, columns: &[Column]) -> Result<ListView, Error> {
        // Clip siblings so an input shown over the list stays visible.
        let style = WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_CLIPSIBLINGS
            | WINDOW_STYLE(LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS);
        let hwnd = create_child(window, WC_LISTVIEWW, WINDOW_EX_STYLE(0), style, id.0)?;
        let extended = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        send(
            hwnd,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            usize::try_from(extended).unwrap_or(0),
            isize::try_from(extended).unwrap_or(0),
        );
        // SAFETY: the theme name is a NUL-terminated literal.
        let _ = unsafe { SetWindowTheme(hwnd, w!("Explorer"), PCWSTR::null()) };
        let dpi = window.dpi();
        for (index, column) in columns.iter().enumerate() {
            let mut title = wide(column.title);
            let spec = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM,
                fmt: if column.align_right {
                    LVCFMT_RIGHT
                } else {
                    LVCFMT_LEFT
                },
                cx: dpi.scale(column.width),
                pszText: PWSTR(title.as_mut_ptr()),
                iSubItem: i32::try_from(index).unwrap_or(0),
                ..Default::default()
            };
            // `title` outlives the call; the control copies the text.
            send_ref(hwnd, LVM_INSERTCOLUMNW, index, &spec);
        }
        let state = Rc::new(ListState {
            hwnd,
            id,
            columns: columns.len(),
            highlighting: Cell::new(false),
            brush: RefCell::new(None),
        });
        window.state().adopt_list(state.clone());
        Ok(ListView {
            state,
            window: window.state().clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn hwnd(&self) -> HWND {
        self.state.hwnd
    }

    /// Sets the number of rows, keeping the scroll position. The source is
    /// asked for the visible cells when the list repaints.
    pub fn set_count(&self, rows: usize) {
        let _quiet = self.window.quiet();
        send(
            self.state.hwnd,
            LVM_SETITEMCOUNT,
            rows,
            isize::try_from(LVSICF_NOSCROLL).unwrap_or(0),
        );
    }

    fn next(&self, after: Option<usize>, flags: u32) -> Option<usize> {
        // The start index -1 (all bits set) means "from the beginning".
        let start = after.unwrap_or(usize::MAX);
        let found = send(
            self.state.hwnd,
            LVM_GETNEXTITEM,
            start,
            isize::try_from(flags).unwrap_or(0),
        );
        usize::try_from(found).ok()
    }

    /// The row with the keyboard focus.
    #[must_use]
    pub fn focused(&self) -> Option<usize> {
        self.next(None, LVNI_FOCUSED)
    }

    /// The selected rows in ascending order.
    #[must_use]
    pub fn selected(&self) -> Vec<usize> {
        let mut rows = Vec::new();
        let mut at = None;
        while let Some(row) = self.next(at, LVNI_SELECTED) {
            rows.push(row);
            at = Some(row);
        }
        rows
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        usize::try_from(send(self.state.hwnd, LVM_GETSELECTEDCOUNT, 0, 0)).unwrap_or(0)
    }

    /// Changes the state bits in `mask` of one row, or of all rows for `None`.
    fn set_state(
        &self,
        row: Option<usize>,
        state: LIST_VIEW_ITEM_STATE_FLAGS,
        mask: LIST_VIEW_ITEM_STATE_FLAGS,
    ) {
        let _quiet = self.window.quiet();
        let item = LVITEMW {
            state,
            stateMask: mask,
            ..Default::default()
        };
        // The index -1 (all bits set) addresses every row.
        send_ref(
            self.state.hwnd,
            LVM_SETITEMSTATE,
            row.unwrap_or(usize::MAX),
            &item,
        );
    }

    /// Deselects all rows and removes the focus mark.
    pub fn clear_selection(&self) {
        self.set_state(
            None,
            LIST_VIEW_ITEM_STATE_FLAGS(0),
            LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
        );
    }

    /// Adds a row to the selection.
    pub fn select(&self, row: usize) {
        self.set_state(Some(row), LVIS_SELECTED, LVIS_SELECTED);
    }

    /// Makes a row the focused one and scrolls it into view.
    pub fn focus(&self, row: usize) {
        self.set_state(Some(row), LVIS_FOCUSED, LVIS_FOCUSED);
        let _quiet = self.window.quiet();
        send(self.state.hwnd, LVM_ENSUREVISIBLE, row, 0);
    }

    /// Makes `row` the only selected row, focused and scrolled into view.
    pub fn select_only(&self, row: usize) {
        self.clear_selection();
        self.select(row);
        self.focus(row);
    }

    /// Switches match highlighting on or off (see [`ListSource::highlight`])
    /// and repaints.
    pub fn set_highlighting(&self, on: bool) {
        self.state.highlighting.set(on);
        self.invalidate();
    }

    /// Requests a repaint of all rows.
    pub fn invalidate(&self) {
        invalidate(self.state.hwnd);
    }

    /// Repaints now instead of when the message queue is empty.
    pub fn redraw_now(&self) {
        self.invalidate();
        // SAFETY: plain value; the window belongs to this thread.
        let _ = unsafe { UpdateWindow(self.state.hwnd) };
    }

    #[must_use]
    pub fn has_focus(&self) -> bool {
        has_focus(self.state.hwnd)
    }

    /// Gives the list the keyboard focus.
    pub fn focus_control(&self) {
        take_focus(self.state.hwnd);
    }

    pub fn set_bounds(&self, bounds: Rect) {
        set_bounds(self.state.hwnd, bounds);
    }

    /// Text of a cell as the control reports it.
    #[cfg(test)]
    pub(crate) fn item_text(&self, row: usize, column: usize) -> String {
        use windows::Win32::UI::Controls::LVM_GETITEMTEXTW;
        let mut text = [0u16; 64];
        let mut item = LVITEMW {
            iSubItem: i32::try_from(column).unwrap(),
            pszText: PWSTR(text.as_mut_ptr()),
            cchTextMax: i32::try_from(text.len()).unwrap(),
            ..Default::default()
        };
        let len = send_mut(self.state.hwnd, LVM_GETITEMTEXTW, row, &mut item);
        String::from_utf16_lossy(&text[..usize::try_from(len).unwrap()])
    }
}

/// Handles a notification from the list control. `None` leaves it to the
/// default window procedure.
pub(crate) fn on_notify(
    window: &WindowState,
    list: &ListState,
    source: &dyn ListSource,
    code: u32,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match code {
        LVN_GETDISPINFOW => {
            // SAFETY: this notification carries an NMLVDISPINFOW that the
            // handler fills in; nothing else accesses it meanwhile.
            let info: &mut NMLVDISPINFOW = unsafe { param_mut(lparam) };
            let capacity = usize::try_from(info.item.cchTextMax).unwrap_or(0);
            if info.item.mask.0 & LVIF_TEXT.0 != 0 && !info.item.pszText.is_null() && capacity > 0 {
                // SAFETY: with LVIF_TEXT the control supplies a writable
                // buffer of `cchTextMax` units for the duration of the
                // notification.
                let text = unsafe { std::slice::from_raw_parts_mut(info.item.pszText.0, capacity) };
                let mut out = CellBuf::new(text);
                if let (Ok(row), Ok(column)) = (
                    usize::try_from(info.item.iItem),
                    usize::try_from(info.item.iSubItem),
                ) {
                    source.cell(row, column, &mut out);
                }
                out.finish();
            }
            Some(LRESULT(0))
        }
        NM_CUSTOMDRAW => {
            // SAFETY: for a list view this notification carries an
            // NMLVCUSTOMDRAW, whose colors the handler may set.
            let draw: &mut NMLVCUSTOMDRAW = unsafe { param_mut(lparam) };
            let result = custom_draw(window, list, source, draw);
            Some(LRESULT(isize::try_from(result).unwrap_or(0)))
        }
        LVN_COLUMNCLICK => {
            // SAFETY: this notification carries an NMLISTVIEW.
            let click: &NMLISTVIEW = unsafe { param_ref(lparam) };
            if let Ok(column) = usize::try_from(click.iSubItem)
                && !window.is_quiet()
            {
                source.on_column_click(column);
            }
            Some(LRESULT(0))
        }
        NM_DBLCLK => {
            // SAFETY: for a list view this notification carries an NMITEMACTIVATE.
            let click: &NMITEMACTIVATE = unsafe { param_ref(lparam) };
            if let Ok(row) = usize::try_from(click.iItem)
                && !window.is_quiet()
            {
                source.on_activate(row);
            }
            Some(LRESULT(0))
        }
        LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
            if !window.is_quiet() {
                source.on_selection_changed();
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// Runs the custom-draw protocol: rows get their color from the source, and
/// while highlighting is on, first-column cells with a match are drawn here.
fn custom_draw(
    window: &WindowState,
    list: &ListState,
    source: &dyn ListSource,
    draw: &mut NMLVCUSTOMDRAW,
) -> u32 {
    let subitem_prepaint = NMCUSTOMDRAW_DRAW_STAGE(CDDS_ITEMPREPAINT.0 | CDDS_SUBITEM.0);
    let stage = draw.nmcd.dwDrawStage;
    let row = draw.nmcd.dwItemSpec;
    let highlighting = list.highlighting.get();
    if stage == CDDS_PREPAINT {
        CDRF_NOTIFYITEMDRAW
    } else if stage == CDDS_ITEMPREPAINT {
        apply_row_color(source, row, draw);
        if highlighting {
            CDRF_NEWFONT | CDRF_NOTIFYSUBITEMDRAW
        } else {
            CDRF_NEWFONT
        }
    } else if stage == subitem_prepaint {
        apply_row_color(source, row, draw);
        if draw.iSubItem == 0 && highlighting && paint_highlight(window, list, source, draw) {
            CDRF_SKIPDEFAULT
        } else {
            CDRF_NEWFONT
        }
    } else {
        CDRF_DODEFAULT
    }
}

fn apply_row_color(source: &dyn ListSource, row: usize, draw: &mut NMLVCUSTOMDRAW) {
    if let Some(color) = source.row_color(row) {
        draw.clrText = color.colorref();
    }
}

/// Draws the first cell of a row with the matched part on a highlight
/// background. Returns `false` (default drawing) when nothing matches.
fn paint_highlight(
    window: &WindowState,
    list: &ListState,
    source: &dyn ListSource,
    draw: &NMLVCUSTOMDRAW,
) -> bool {
    let row = draw.nmcd.dwItemSpec;
    let Some(range) = source.highlight(row) else {
        return false;
    };
    let mut scratch = [0u16; SCRATCH_UNITS];
    let mut out = CellBuf::new(&mut scratch);
    source.cell(row, 0, &mut out);
    let len = out.finish();
    let text = &mut scratch[..len];
    if range.start >= range.end || range.end > text.len() {
        return false;
    }

    let mut label = RECT {
        left: i32::try_from(LVIR_LABEL).unwrap_or(0),
        ..Default::default()
    };
    send_mut(list.hwnd, LVM_GETITEMRECT, row, &mut label);

    // SAFETY: the control hands out this device context for the duration of
    // the custom-draw notification, which outlives the painter.
    let painter = unsafe { Painter::new(draw.nmcd.hdc) };
    // The control reports "default color" with the high byte set.
    let text_color = if draw.clrText.0 & 0xFF00_0000 != 0 {
        gdi::window_text_color()
    } else {
        draw.clrText
    };
    let (mark_color, hit_color) = if window.high_contrast() {
        (gdi::highlight_color(), gdi::highlight_text_color())
    } else {
        (HIGHLIGHT.colorref(), text_color)
    };

    let (head, rest) = text.split_at_mut(range.start);
    let (hit, tail) = rest.split_at_mut(range.end - range.start);
    let at = |x: i32| RECT { left: x, ..label };

    painter.transparent_background();
    painter.set_text_color(text_color);
    let mut x = label.left + window.dpi().scale(LABEL_MARGIN);
    painter.text(head, at(x));
    x += painter.measure(head).cx;

    let hit_size = painter.measure(hit);
    let top = label.top + (label.bottom - label.top - hit_size.cy) / 2;
    let mark = RECT {
        left: x,
        top,
        right: (x + hit_size.cx).min(label.right),
        bottom: top + hit_size.cy,
    };
    list.with_brush(mark_color, |brush| painter.fill(mark, brush));
    painter.set_text_color(hit_color);
    painter.text(hit, at(x));
    x += hit_size.cx;

    painter.set_text_color(text_color);
    painter.text(tail, at(x));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_text_is_cut_off_and_terminated() {
        let mut raw = [0xFFFFu16; 4];
        let mut out = CellBuf::new(&mut raw);
        out.push_str("ab");
        out.push_units(&[0x63, 0x64, 0x65]);
        out.push_unit(0x66);
        assert_eq!(out.finish(), 3);
        assert_eq!(raw, [0x61, 0x62, 0x63, 0]);
    }

    #[test]
    fn empty_buffer_is_left_alone() {
        let mut raw: [u16; 0] = [];
        let mut out = CellBuf::new(&mut raw);
        out.push_str("x");
        out.push_units(&[1, 2]);
        assert_eq!(out.finish(), 0);
    }
}
