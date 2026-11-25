//! The top-level window: creation, the window procedure and the message
//! loop. Everything except [`WindowToken`] and [`UiWaker`] is bound to the
//! thread that created the window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE,
    EN_KILLFOCUS, GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, IDC_ARROW,
    IDI_APPLICATION, IsChild, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, LoadIconW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOWDEFAULT, SWP_NOACTIVATE, SWP_NOZORDER,
    SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP, WM_COMMAND, WM_DESTROY, WM_DPICHANGED,
    WM_NCDESTROY, WM_NOTIFY, WM_SETFOCUS, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE,
    WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, w};

use crate::controls::edit::{EditEvent, EditId};
use crate::controls::list::{self, ListId, ListSource, ListState};
use crate::error::Error;
use crate::gdi::{self, MessageFont};
use crate::geom::{Dpi, Size};
use crate::input::{self, KeyEvent};
use crate::msg::{param_ref, send};
use crate::num::{hiword, loword, size_u32};
use crate::wide::wide;

const CLASS: PCWSTR = w!("lwfm.main");
const WM_APP_WAKE: u32 = WM_APP + 1;

/// Identifies a timer of a window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimerId(pub usize);

/// Something that happened to the window. The mechanical part is already
/// done when the event arrives: after `DpiChanged` the window has its new
/// size and all controls their new font, for example.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowEvent {
    /// The client area changed size.
    Resized,
    /// The window moved to a monitor with another scale factor.
    DpiChanged {
        old: Dpi,
        new: Dpi,
    },
    Timer(TimerId),
    /// A [`UiWaker`] was triggered from another thread.
    Wake,
    /// System colors, fonts or themes changed.
    SettingsChanged,
    /// The window is going away; the message loop ends after this.
    Destroyed,
}

/// Receives everything the window and its controls report. All methods run
/// on the UI thread and may be called while a method of the window or a
/// control is still running (see the notes on the controls), so
/// implementations keep their state in `Cell`/`RefCell`.
pub trait Handler {
    fn on_event(&self, event: WindowEvent);

    /// Called for keyboard messages before they are dispatched. Returning
    /// `true` consumes the message.
    fn on_key(&self, key: KeyEvent) -> bool;

    /// The data source of the list with this id.
    fn list_source(&self, id: ListId) -> &dyn ListSource;

    fn on_edit(&self, id: EditId, event: EditEvent);
}

/// Identifies a window from any thread, for calls that accept a window
/// handle off the UI thread (owning a shell dialog).
#[derive(Clone, Copy, Debug)]
pub struct WindowToken(isize);

impl WindowToken {
    /// Temporary bridge while the application still creates its own window.
    #[doc(hidden)]
    #[must_use]
    pub fn from_raw(hwnd: isize) -> WindowToken {
        WindowToken(hwnd)
    }

    fn of(hwnd: HWND) -> WindowToken {
        WindowToken(hwnd.0.addr().cast_signed())
    }

    pub(crate) fn hwnd(self) -> HWND {
        HWND(std::ptr::with_exposed_provenance_mut(
            self.0.cast_unsigned(),
        ))
    }
}

/// Wakes the UI thread from any thread: the window's handler receives
/// [`WindowEvent::Wake`]. Wake-ups requested before the event is delivered
/// are merged into one.
#[derive(Clone, Debug)]
pub struct UiWaker {
    window: WindowToken,
    pending: Arc<AtomicBool>,
}

impl UiWaker {
    pub fn wake(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        // SAFETY: posting takes a window handle from any thread and plain
        // parameters; it fails if the window is gone.
        let posted =
            unsafe { PostMessageW(Some(self.window.hwnd()), WM_APP_WAKE, WPARAM(0), LPARAM(0)) };
        if posted.is_err() {
            self.pending.store(false, Ordering::Release);
        }
    }
}

/// State shared by the window procedure, the [`Window`] and its controls.
pub(crate) struct WindowState {
    pub(crate) hwnd: HWND,
    dpi: Cell<u32>,
    font: RefCell<Option<MessageFont>>,
    /// Child controls, which share the window's font.
    children: RefCell<Vec<HWND>>,
    lists: RefCell<Vec<Rc<ListState>>>,
    edits: RefCell<Vec<(HWND, EditId)>>,
    /// Depth of running control methods whose effects are not reported back.
    quiet: Cell<u32>,
    /// The child that had the focus when the window was deactivated.
    last_focus: Cell<Option<HWND>>,
    handler: Cell<Option<*const dyn Handler>>,
    wake_pending: Arc<AtomicBool>,
    high_contrast: Cell<bool>,
}

/// While alive, notifications caused by control methods are not reported.
pub(crate) struct Quiet<'a>(&'a WindowState);

impl Drop for Quiet<'_> {
    fn drop(&mut self) {
        self.0.quiet.set(self.0.quiet.get() - 1);
    }
}

impl WindowState {
    pub(crate) fn quiet(&self) -> Quiet<'_> {
        self.quiet.set(self.quiet.get() + 1);
        Quiet(self)
    }

    pub(crate) fn is_quiet(&self) -> bool {
        self.quiet.get() > 0
    }

    pub(crate) fn dpi(&self) -> Dpi {
        Dpi(self.dpi.get())
    }

    pub(crate) fn high_contrast(&self) -> bool {
        self.high_contrast.get()
    }

    /// Registers a child control and gives it the window's font.
    pub(crate) fn adopt(&self, child: HWND) {
        self.children.borrow_mut().push(child);
        if let Some(font) = self.font.borrow().as_ref() {
            send(child, WM_SETFONT, font.font.raw().0.addr(), 1);
        }
    }

    pub(crate) fn adopt_list(&self, list: Rc<ListState>) {
        self.adopt(list.hwnd);
        self.lists.borrow_mut().push(list);
    }

    pub(crate) fn adopt_edit(&self, edit: HWND, id: EditId) {
        self.adopt(edit);
        self.edits.borrow_mut().push((edit, id));
    }

    fn handler(&self) -> Option<&dyn Handler> {
        let handler = self.handler.get()?;
        // SAFETY: the pointer is only set by `Window::with_handler`, which
        // keeps the handler borrowed until it has cleared the pointer again.
        Some(unsafe { &*handler })
    }

    fn emit(&self, event: WindowEvent) {
        if let Some(handler) = self.handler() {
            handler.on_event(event);
        }
    }

    /// Makes the system message font for the current DPI the font of all
    /// controls, unless that is the font they already have.
    fn refresh_font(&self) {
        let Some(description) = gdi::message_font_description(self.dpi.get()) else {
            return;
        };
        if self
            .font
            .borrow()
            .as_ref()
            .is_some_and(|current| current.description == description)
        {
            return;
        }
        let new = gdi::create_font(description);
        let children = self.children.borrow().clone();
        for child in children {
            send(child, WM_SETFONT, new.font.raw().0.addr(), 1);
        }
        // The old font is deleted here, after every control let go of it.
        let _old = self.font.replace(Some(new));
    }

    fn list(&self, hwnd: HWND) -> Option<Rc<ListState>> {
        self.lists.borrow().iter().find(|l| l.hwnd == hwnd).cloned()
    }

    fn edit(&self, hwnd: HWND) -> Option<EditId> {
        let edits = self.edits.borrow();
        edits.iter().find(|(h, _)| *h == hwnd).map(|(_, id)| *id)
    }

    fn on_message(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => {
                self.emit(WindowEvent::Resized);
                Some(LRESULT(0))
            }
            WM_NOTIFY => {
                // SAFETY: WM_NOTIFY carries a pointer to an NMHDR (the start
                // of the notification's structure), valid while it is handled.
                let header: &NMHDR = unsafe { param_ref(lparam) };
                let list = self.list(header.hwndFrom)?;
                let source = self.handler()?.list_source(list.id);
                list::on_notify(self, &list, source, header.code, lparam)
            }
            WM_COMMAND => {
                let control = HWND(std::ptr::with_exposed_provenance_mut(
                    lparam.0.cast_unsigned(),
                ));
                let id = self.edit(control)?;
                let event = match u32::from(hiword(wparam.0)) {
                    EN_CHANGE => Some(EditEvent::Changed),
                    EN_KILLFOCUS => Some(EditEvent::LostFocus),
                    _ => None,
                };
                if let Some(event) = event
                    && !self.is_quiet()
                    && let Some(handler) = self.handler()
                {
                    handler.on_edit(id, event);
                }
                Some(LRESULT(0))
            }
            WM_APP_WAKE => {
                // Cleared first: a wake-up requested from now on must post again.
                self.wake_pending.store(false, Ordering::Release);
                self.emit(WindowEvent::Wake);
                Some(LRESULT(0))
            }
            WM_TIMER => {
                self.emit(WindowEvent::Timer(TimerId(wparam.0)));
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if u32::from(loword(wparam.0)) == WA_INACTIVE {
                    // SAFETY: no arguments.
                    let focus = unsafe { GetFocus() };
                    // SAFETY: both handles are plain values; invalid ones yield false.
                    if unsafe { IsChild(self.hwnd, focus) }.as_bool() {
                        self.last_focus.set(Some(focus));
                    }
                }
                None
            }
            WM_SETFOCUS => {
                self.restore_focus();
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let new = u32::from(loword(wparam.0));
                let old = self.dpi.replace(new);
                self.refresh_font();
                for list in self.lists.borrow().iter() {
                    list.rescale_columns(old, new);
                }
                // SAFETY: WM_DPICHANGED carries a pointer to the suggested
                // window rectangle, valid while the message is handled.
                let suggested: &RECT = unsafe { param_ref(lparam) };
                // SAFETY: plain values; the window is this thread's.
                let _ = unsafe {
                    SetWindowPos(
                        self.hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                self.emit(WindowEvent::DpiChanged {
                    old: Dpi(old),
                    new: Dpi(new),
                });
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                self.high_contrast.set(gdi::high_contrast());
                self.refresh_font();
                // Common controls only learn about these from their parent.
                let children = self.children.borrow().clone();
                for child in children {
                    // The parameters are forwarded unchanged; a string that
                    // WM_SETTINGCHANGE points to stays valid for this call.
                    send(child, msg, wparam.0, lparam.0);
                }
                for list in self.lists.borrow().iter() {
                    list.colors_changed();
                }
                self.emit(WindowEvent::SettingsChanged);
                None
            }
            WM_DESTROY => {
                self.emit(WindowEvent::Destroyed);
                // SAFETY: no pointer arguments.
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// Gives the focus back to the control that had it, or to the first list.
    fn restore_focus(&self) {
        let visible = |hwnd: &HWND| {
            // SAFETY: plain value; an invalid handle yields false.
            unsafe { IsWindowVisible(*hwnd) }.as_bool()
        };
        let target = self
            .last_focus
            .get()
            .filter(visible)
            .or_else(|| self.lists.borrow().first().map(|l| l.hwnd));
        if let Some(target) = target {
            // SAFETY: plain value; the window belongs to this thread.
            let _ = unsafe { SetFocus(Some(target)) };
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: plain values.
    let default = || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    // SAFETY: reads the pointer-sized user data of a window of this class.
    let user_data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    let state = std::ptr::with_exposed_provenance::<WindowState>(user_data.cast_unsigned());
    if state.is_null() {
        // Before `Window::create` attached the state, or after WM_NCDESTROY.
        return default();
    }
    if msg == WM_NCDESTROY {
        // SAFETY: plain values; detaches the state so it is released once.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        // SAFETY: `state` came from `Rc::into_raw` in `Window::create` and
        // was just detached, so this is the only release of that reference.
        drop(unsafe { Rc::from_raw(state) });
        return default();
    }
    // Hold an own reference for this call: the handler may destroy the
    // window, which releases the reference stored in the user data.
    // SAFETY: the user data holds a live `Rc<WindowState>` (see above).
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: takes over the reference counted just above.
    let state = unsafe { Rc::from_raw(state) };
    state
        .on_message(msg, wparam, lparam)
        .unwrap_or_else(default)
}

fn module() -> Result<HINSTANCE, Error> {
    // SAFETY: a null name asks for the module of the process itself.
    Ok(unsafe { GetModuleHandleW(None) }?.into())
}

fn register_class() -> Result<(), Error> {
    static REGISTERED: OnceLock<Result<(), Error>> = OnceLock::new();
    *REGISTERED.get_or_init(register_class_once)
}

fn register_class_once() -> Result<(), Error> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_u32::<INITCOMMONCONTROLSEX>(),
        dwICC: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
    };
    // SAFETY: `controls` is initialized with its size set.
    let _ = unsafe { InitCommonControlsEx(&raw const controls) };
    // SAFETY: a null module with a predefined id loads a system cursor.
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }?;
    // SAFETY: as above, for a system icon.
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }?;
    let class = WNDCLASSEXW {
        cbSize: size_u32::<WNDCLASSEXW>(),
        lpfnWndProc: Some(wndproc),
        hInstance: module()?,
        hCursor: cursor,
        hIcon: icon,
        // A system color index plus one stands for that color's brush.
        hbrBackground: HBRUSH(std::ptr::without_provenance_mut(
            usize::try_from(COLOR_WINDOW.0 + 1).unwrap_or(0),
        )),
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialized and its strings are static.
    if unsafe { RegisterClassExW(&raw const class) } == 0 {
        return Err(Error::last());
    }
    Ok(())
}

/// The application's top-level window. Dropping it destroys the window.
pub struct Window {
    state: Rc<WindowState>,
}

impl Window {
    /// Creates the window, hidden, with default position and size.
    ///
    /// # Errors
    ///
    /// Fails when the window class or the window cannot be created.
    pub fn create(title: &str) -> Result<Window, Error> {
        register_class()?;
        let title = wide(title);
        // SAFETY: the class is registered, and the class name and title are
        // NUL-terminated and outlive the call.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(module()?),
                None,
            )
        }?;
        // SAFETY: plain value; the window was just created.
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let state = Rc::new(WindowState {
            hwnd,
            dpi: Cell::new(dpi),
            font: RefCell::new(None),
            children: RefCell::new(Vec::new()),
            lists: RefCell::new(Vec::new()),
            edits: RefCell::new(Vec::new()),
            quiet: Cell::new(0),
            last_focus: Cell::new(None),
            handler: Cell::new(None),
            wake_pending: Arc::new(AtomicBool::new(false)),
            high_contrast: Cell::new(gdi::high_contrast()),
        });
        state.refresh_font();
        let user_data = Rc::into_raw(state.clone()).addr().cast_signed();
        // SAFETY: stores a pointer-sized value in the window's user data; the
        // window procedure releases this reference on WM_NCDESTROY.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, user_data) };
        Ok(Window { state })
    }

    pub(crate) fn state(&self) -> &Rc<WindowState> {
        &self.state
    }

    pub(crate) fn module(&self) -> Result<HINSTANCE, Error> {
        module()
    }

    /// Shows the window the way the process was asked to start.
    pub fn show(&self) {
        // SAFETY: plain values.
        let _ = unsafe { ShowWindow(self.state.hwnd, SW_SHOWDEFAULT) };
    }

    pub fn set_title(&self, title: &str) {
        let title = wide(title);
        // SAFETY: `title` is NUL-terminated and outlives the call.
        let _ = unsafe { SetWindowTextW(self.state.hwnd, PCWSTR(title.as_ptr())) };
    }

    #[must_use]
    pub fn dpi(&self) -> Dpi {
        self.state.dpi()
    }

    /// Size of the client area in pixels.
    #[must_use]
    pub fn client_size(&self) -> Size {
        let mut rect = RECT::default();
        // SAFETY: `rect` is writable.
        let _ = unsafe { GetClientRect(self.state.hwnd, &raw mut rect) };
        Size::from(rect)
    }

    /// Starts (or restarts) a timer that raises [`WindowEvent::Timer`]
    /// repeatedly until it is stopped.
    pub fn start_timer(&self, id: TimerId, interval: Duration) {
        let millis = u32::try_from(interval.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: plain values; without a callback the timer posts WM_TIMER.
        unsafe { SetTimer(Some(self.state.hwnd), id.0, millis, None) };
    }

    pub fn stop_timer(&self, id: TimerId) {
        // SAFETY: plain values.
        let _ = unsafe { KillTimer(Some(self.state.hwnd), id.0) };
    }

    /// Destroys the window, which ends the message loop.
    pub fn close(&self) {
        // SAFETY: plain value; the window belongs to this thread.
        let _ = unsafe { DestroyWindow(self.state.hwnd) };
    }

    #[must_use]
    pub fn token(&self) -> WindowToken {
        WindowToken::of(self.state.hwnd)
    }

    #[must_use]
    pub fn waker(&self) -> UiWaker {
        UiWaker {
            window: self.token(),
            pending: self.state.wake_pending.clone(),
        }
    }

    /// Runs `f` with `handler` receiving the window's events.
    fn with_handler<H: Handler + 'static, R>(&self, handler: &H, f: impl FnOnce() -> R) -> R {
        struct Detach<'a>(&'a WindowState);
        impl Drop for Detach<'_> {
            fn drop(&mut self) {
                self.0.handler.set(None);
            }
        }
        let handler: &dyn Handler = handler;
        self.state.handler.set(Some(std::ptr::from_ref(handler)));
        let _detach = Detach(&self.state);
        f()
    }

    /// Runs the message loop with `handler` receiving all events, until the
    /// window is destroyed. Returns the exit code.
    pub fn run<H: Handler + 'static>(&self, handler: &H) -> i32 {
        self.with_handler(handler, || {
            let mut msg = MSG::default();
            loop {
                // SAFETY: `msg` is writable; no filter.
                let got = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
                if got.0 <= 0 {
                    break;
                }
                if let Some(key) = input::key_event(&msg)
                    && handler.on_key(key)
                {
                    continue;
                }
                // SAFETY: `msg` was filled by `GetMessageW`.
                let _ = unsafe { TranslateMessage(&raw const msg) };
                // SAFETY: as above.
                unsafe { DispatchMessageW(&raw const msg) };
            }
            i32::try_from(msg.wParam.0).unwrap_or(0)
        })
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: plain value; an already destroyed window yields false.
        if unsafe { IsWindow(Some(self.state.hwnd)) }.as_bool() {
            self.close();
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Helpers for tests that drive a hidden window without `Window::run`.

    use windows::Win32::UI::WindowsAndMessaging::{PM_REMOVE, PeekMessageW};

    use super::*;

    impl Window {
        /// Runs `f` with `handler` attached, as during `run`.
        pub(crate) fn attached<H: Handler + 'static, R>(
            &self,
            handler: &H,
            f: impl FnOnce() -> R,
        ) -> R {
            self.with_handler(handler, f)
        }
    }

    /// Dispatches the messages queued for this thread.
    pub(crate) fn pump() {
        let mut msg = MSG::default();
        // SAFETY: `msg` is writable; no filter.
        while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: `msg` was filled by `PeekMessageW`.
            unsafe { DispatchMessageW(&raw const msg) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::testing::pump;
    use super::*;
    use crate::controls::list::ListSource;

    #[derive(Default)]
    struct Recorder {
        events: RefCell<Vec<WindowEvent>>,
    }

    struct NoList;
    impl ListSource for NoList {
        fn cell(&self, _: usize, _: usize, _: &mut crate::controls::list::CellBuf<'_>) {}
    }

    impl Handler for Recorder {
        fn on_event(&self, event: WindowEvent) {
            self.events.borrow_mut().push(event);
        }
        fn on_key(&self, _: KeyEvent) -> bool {
            false
        }
        fn list_source(&self, _: ListId) -> &dyn ListSource {
            &NoList
        }
        fn on_edit(&self, _: EditId, _: EditEvent) {}
    }

    #[test]
    fn wake_from_another_thread_reaches_the_handler_once() {
        let window = Window::create("test").unwrap();
        let recorder = Recorder::default();
        let waker = window.waker();
        window.attached(&recorder, || {
            std::thread::spawn(move || {
                waker.wake();
                waker.wake();
            })
            .join()
            .unwrap();
            pump();
        });
        assert_eq!(*recorder.events.borrow(), [WindowEvent::Wake]);
    }

    #[test]
    fn handler_is_detached_afterwards() {
        let window = Window::create("test").unwrap();
        let recorder = Recorder::default();
        window.attached(&recorder, || {});
        window.waker().wake();
        pump();
        assert!(recorder.events.borrow().is_empty());
        assert!(window.state.handler.get().is_none());
    }

    #[test]
    fn timer_event_carries_its_id() {
        let window = Window::create("test").unwrap();
        let recorder = Recorder::default();
        window.attached(&recorder, || {
            send(window.state.hwnd, WM_TIMER, 7, 0);
        });
        assert_eq!(*recorder.events.borrow(), [WindowEvent::Timer(TimerId(7))]);
    }

    #[test]
    fn destroying_the_window_releases_its_state() {
        let window = Window::create("test").unwrap();
        let state = Rc::downgrade(&window.state);
        assert_eq!(Rc::strong_count(&window.state), 2);
        drop(window);
        assert!(state.upgrade().is_none());
    }
}
