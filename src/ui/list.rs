//! The virtual (owner-data) ListView showing the listing.

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR, w};

use super::{scale, send, wide};

pub const COLUMNS: [(&str, i32, bool); 3] = [
    ("Name", 360, false),
    ("Size", 90, true),
    ("Modified", 140, false),
];

pub const DIR_COLOR: COLORREF = COLORREF(0x00B4_5F1A); // RGB(0x1A, 0x5F, 0xB4)
pub const HIDDEN_COLOR: COLORREF = COLORREF(0x0080_8080);

pub fn create(
    parent: HWND,
    hinstance: HINSTANCE,
    id: usize,
    dpi: u32,
) -> windows::core::Result<HWND> {
    let style =
        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS;
    let list = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_LISTVIEWW,
            PCWSTR::null(),
            WINDOW_STYLE(style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as _)),
            Some(hinstance),
            None,
        )?
    };
    let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
    send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as isize);
    unsafe {
        let _ = SetWindowTheme(list, w!("Explorer"), PCWSTR::null());
    }
    for (i, (title, width, right)) in COLUMNS.iter().enumerate() {
        let mut text = wide(title);
        let col = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM,
            fmt: if *right { LVCFMT_RIGHT } else { LVCFMT_LEFT },
            cx: scale(*width, dpi),
            pszText: PWSTR(text.as_mut_ptr()),
            iSubItem: i as i32,
            ..Default::default()
        };
        send(list, LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
    }
    Ok(list)
}

pub fn set_count(list: HWND, n: usize) {
    send(list, LVM_SETITEMCOUNT, n, LVSICF_NOSCROLL as isize);
}

fn next(list: HWND, start: isize, flags: u32) -> Option<usize> {
    let r = send(list, LVM_GETNEXTITEM, start as usize, flags as isize).0;
    usize::try_from(r).ok()
}

pub fn focused(list: HWND) -> Option<usize> {
    next(list, -1, LVNI_FOCUSED)
}

pub fn selected(list: HWND) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = -1isize;
    while let Some(i) = next(list, at, LVNI_SELECTED) {
        out.push(i);
        at = i as isize;
    }
    out
}

pub fn selected_count(list: HWND) -> usize {
    send(list, LVM_GETSELECTEDCOUNT, 0, 0).0 as usize
}

fn set_state(
    list: HWND,
    index: isize,
    state: LIST_VIEW_ITEM_STATE_FLAGS,
    mask: LIST_VIEW_ITEM_STATE_FLAGS,
) {
    let item = LVITEMW {
        state,
        stateMask: mask,
        ..Default::default()
    };
    send(
        list,
        LVM_SETITEMSTATE,
        index as usize,
        &item as *const _ as isize,
    );
}

pub fn clear_selection(list: HWND) {
    set_state(
        list,
        -1,
        LIST_VIEW_ITEM_STATE_FLAGS(0),
        LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
    );
}

pub fn select(list: HWND, index: usize) {
    set_state(list, index as isize, LVIS_SELECTED, LVIS_SELECTED);
}

pub fn focus(list: HWND, index: usize) {
    set_state(list, index as isize, LVIS_FOCUSED, LVIS_FOCUSED);
    send(list, LVM_ENSUREVISIBLE, index, 0);
}

/// Rescales column widths after a DPI change, keeping user-resized widths.
pub fn rescale_columns(list: HWND, old_dpi: u32, new_dpi: u32) {
    for i in 0..COLUMNS.len() {
        let w = send(list, LVM_GETCOLUMNWIDTH, i, 0).0 as i64;
        let scaled = (w * new_dpi as i64 / old_dpi.max(1) as i64) as isize;
        send(list, LVM_SETCOLUMNWIDTH, i, scaled);
    }
}

/// Copies `text` into the buffer the ListView supplied in `item`.
pub fn write_text(item: &mut LVITEMW, text: &[u16]) {
    if item.pszText.is_null() || item.cchTextMax <= 0 {
        return;
    }
    let n = text.len().min(item.cchTextMax as usize - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), item.pszText.0, n);
        *item.pszText.0.add(n) = 0;
    }
}
