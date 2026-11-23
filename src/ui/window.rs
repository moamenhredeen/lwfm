//! Top-level window: class registration, window procedure and message loop.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use super::{list, pathbar, searchbox, statusbar};
use crate::app::{App, Controls, Startup};

const CLASS: PCWSTR = w!("lwfm.main");
const ID_LIST: usize = 1;
const ID_PATH: usize = 2;
const ID_STATUS: usize = 3;
const ID_SEARCH: usize = 4;

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(app) = current()
        && let Some(r) = app.on_message(msg, wparam, lparam)
    {
        return r;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Creates the window, runs the message loop and returns the exit code.
pub fn run(mut startup: Startup) -> windows::core::Result<i32> {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);

        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as _),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("lwfm"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(hinstance),
            None,
        )?;
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd);
        let controls = Controls {
            main: hwnd,
            path: pathbar::create(hwnd, hinstance, ID_PATH)?,
            list: list::create(hwnd, hinstance, ID_LIST, dpi)?,
            status: statusbar::create(hwnd, hinstance, ID_STATUS)?,
            search: searchbox::create(hwnd, hinstance, ID_SEARCH)?,
        };

        let app = Rc::new(App::new(controls, dpi, &mut startup));
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
        app.layout();
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        app.start(&mut startup);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if app.translate_key(&msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|a| a.borrow_mut().take());
        Ok(msg.wParam.0 as i32)
    }
}
