//! The child controls of the window, wrapped by what lwfm uses them for.
//!
//! Two kinds of calls come back from a control, both on the UI thread:
//!
//! - *Data* callbacks ([`ListSource::cell`] and friends) may run at any time,
//!   including synchronously inside a method of the control.
//! - *Notification* callbacks (selection changed, text changed, ...) report
//!   what the user did. Changes made through a control's own methods are not
//!   reported back. The one exception is losing the keyboard focus, which is
//!   reported whatever moved the focus, unless the control was hidden.

pub mod edit;
pub mod list;
pub mod status;

pub use edit::{Edit, EditEvent, EditId};
pub use list::{CellBuf, Column, ListId, ListSource, ListView};
pub use status::StatusBar;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, HMENU, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WINDOW_EX_STYLE,
    WINDOW_STYLE,
};
use windows::core::PCWSTR;

use crate::error::Error;
use crate::geom::Rect;
use crate::window::Window;

/// Creates a child control of `window` with zero size.
fn create_child(
    window: &Window,
    class: PCWSTR,
    ex_style: WINDOW_EX_STYLE,
    style: WINDOW_STYLE,
    id: usize,
) -> Result<HWND, Error> {
    let parent = window.state().hwnd;
    // SAFETY: `class` is a NUL-terminated class name of a registered system
    // class; for a child window the menu parameter carries its id.
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class,
            PCWSTR::null(),
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(std::ptr::without_provenance_mut(id))),
            Some(window.module()?),
            None,
        )
    }?;
    Ok(hwnd)
}

fn set_bounds(hwnd: HWND, bounds: Rect) {
    // SAFETY: plain values; the window belongs to this thread.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn has_focus(hwnd: HWND) -> bool {
    // SAFETY: no arguments.
    let focus = unsafe { GetFocus() };
    focus == hwnd
}

fn take_focus(hwnd: HWND) {
    // SAFETY: plain value; the window belongs to this thread.
    let _ = unsafe { SetFocus(Some(hwnd)) };
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use windows::Win32::UI::Controls::{LVIS_SELECTED, LVITEMW, LVM_SETITEMSTATE};
    use windows::Win32::UI::WindowsAndMessaging::SetWindowTextW;
    use windows::core::w;

    use super::*;
    use crate::input::KeyEvent;
    use crate::msg::send_ref;
    use crate::window::testing::pump;
    use crate::window::{Handler, WindowEvent};

    const COLUMNS: [Column; 2] = [
        Column {
            title: "Name",
            width: 200,
            align_right: false,
        },
        Column {
            title: "Size",
            width: 80,
            align_right: true,
        },
    ];

    #[derive(Default)]
    struct Probe {
        cells: RefCell<Vec<(usize, usize)>>,
        selection_changes: Cell<u32>,
        edits: RefCell<Vec<(EditId, EditEvent)>>,
    }

    impl ListSource for Probe {
        fn cell(&self, row: usize, column: usize, out: &mut CellBuf<'_>) {
            self.cells.borrow_mut().push((row, column));
            out.push_str(&format!("r{row}c{column}"));
        }

        fn on_selection_changed(&self) {
            self.selection_changes.set(self.selection_changes.get() + 1);
        }
    }

    impl Handler for Probe {
        fn on_event(&self, _: WindowEvent) {}
        fn on_key(&self, _: KeyEvent) -> bool {
            false
        }
        fn list_source(&self, _: ListId) -> &dyn ListSource {
            self
        }
        fn on_edit(&self, id: EditId, event: EditEvent) {
            self.edits.borrow_mut().push((id, event));
        }
    }

    #[test]
    fn list_asks_its_source_for_cell_text() {
        let window = Window::create("test").unwrap();
        let list = ListView::new(&window, ListId(1), &COLUMNS).unwrap();
        let probe = Probe::default();
        window.attached(&probe, || {
            list.set_count(3);
            for row in 0..3 {
                assert_eq!(list.item_text(row, 0), format!("r{row}c0"));
            }
            assert_eq!(list.item_text(2, 1), "r2c1");
        });
        let cells = probe.cells.borrow();
        for row in 0..3 {
            assert!(cells.contains(&(row, 0)), "row {row} not requested");
        }
    }

    #[test]
    fn programmatic_selection_is_not_echoed() {
        let window = Window::create("test").unwrap();
        let list = ListView::new(&window, ListId(1), &COLUMNS).unwrap();
        let probe = Probe::default();
        window.attached(&probe, || {
            list.set_count(5);
            list.select(1);
            list.focus(1);
            list.select_only(2);
            list.clear_selection();
            list.set_count(4);
            pump();
            assert_eq!(probe.selection_changes.get(), 0);
            assert_eq!(list.selected_count(), 0);

            list.select(3);
            assert_eq!(list.selected(), [3]);
            assert_eq!(probe.selection_changes.get(), 0);

            // The same change arriving from outside the wrapper, as a click
            // or a key press does, is reported.
            let item = LVITEMW {
                state: LVIS_SELECTED,
                stateMask: LVIS_SELECTED,
                ..Default::default()
            };
            send_ref(list.hwnd(), LVM_SETITEMSTATE, 0, &item);
            assert!(probe.selection_changes.get() > 0);
        });
    }

    #[test]
    fn programmatic_text_is_not_echoed() {
        let window = Window::create("test").unwrap();
        let edit = Edit::new(&window, EditId(2), true).unwrap();
        let probe = Probe::default();
        window.attached(&probe, || {
            edit.set_text("abc");
            assert_eq!(edit.text(), "abc");
            assert!(probe.edits.borrow().is_empty());

            // Text changing from outside the wrapper, as typing does, is reported.
            // SAFETY: the literal is NUL-terminated.
            unsafe { SetWindowTextW(edit.hwnd(), w!("typed")) }.unwrap();
            assert_eq!(*probe.edits.borrow(), [(EditId(2), EditEvent::Changed)]);
            assert_eq!(edit.text(), "typed");
        });
    }

    #[test]
    fn hidden_edit_reports_nothing_and_shows_again() {
        let window = Window::create("test").unwrap();
        let edit = Edit::new(&window, EditId(2), false).unwrap();
        let probe = Probe::default();
        window.attached(&probe, || {
            assert!(!edit.is_visible());
            edit.show_on_top();
            edit.hide();
            assert!(probe.edits.borrow().is_empty());
        });
    }

    #[test]
    fn status_bar_has_a_height() {
        let window = Window::create("test").unwrap();
        let status = StatusBar::new(&window, 3).unwrap();
        status.set_part(0, "3 items");
        assert!(status.layout(280) > 0);
    }
}
