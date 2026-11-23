//! Application state and behaviour. All state lives on the UI thread behind
//! `Cell`/`RefCell`; borrows are kept short and never held across calls that
//! may re-enter the window procedure (e.g. `SendMessageW` to the list).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, HFONT, HGDIOBJ, InvalidateRect, UpdateWindow,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::cmd::registry::{self, CmdResult, Invocation, Target};
use crate::fs::entry::{self, Entry};
use crate::fs::enumerate::{self, FsMsg};
use crate::fs::model::{self, Model, Sort, SortKey};
use crate::fs::path;
use crate::keys::keymap::{Keymap, Scope};
use crate::keys::normalize;
use crate::nav::{History, Location};
use crate::trace;
use crate::ui::{list, pathbar, scale, send, statusbar, wide};

const WM_APP_WAKE: u32 = WM_APP + 1;
const TIMER_REBUILD: usize = 1;
/// While a directory is loading, the list is re-sorted at most this often.
const REBUILD_INTERVAL: Duration = Duration::from_millis(100);

pub struct Controls {
    pub main: HWND,
    pub path: HWND,
    pub list: HWND,
    pub status: HWND,
}

enum Event {
    Fs(FsMsg),
    Message(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NavKind {
    Start,
    New,
    Back,
    Forward,
    Refresh,
}

#[derive(Clone, Debug)]
struct Nav {
    kind: NavKind,
    /// Entry to select once listed (the directory just left, or history).
    select: Option<Box<[u16]>>,
    /// Return focus to the list when the navigation succeeds (path bar).
    focus_list: bool,
    /// On failure, open the home directory instead (start argument).
    fallback_home: bool,
    /// Status message to show when the navigation commits.
    message: Option<String>,
}

impl Nav {
    fn new(kind: NavKind) -> Nav {
        Nav {
            kind,
            select: None,
            focus_list: false,
            fallback_home: false,
            message: None,
        }
    }
}

struct Load {
    path: PathBuf,
    nav: Nav,
    committed: bool,
}

#[derive(Default, Debug)]
struct Selection {
    focused: Option<Box<[u16]>>,
    selected: HashSet<Box<[u16]>>,
}

pub struct App {
    ui: Controls,
    dpi: Cell<u32>,
    font: Cell<HFONT>,
    home: PathBuf,
    keymap: Keymap,

    cwd: RefCell<Option<PathBuf>>,
    model: RefCell<Model>,
    history: RefCell<History>,
    load: RefCell<Option<Load>>,
    loading: Cell<bool>,
    loaded: Cell<usize>,
    /// Selection to apply once its entries show up in the listing.
    restore: RefCell<Option<Selection>>,
    message: RefCell<String>,
    last_rebuild: Cell<Option<Instant>>,
    timer_armed: Cell<bool>,
    last_focus: Cell<HWND>,

    generation: Arc<AtomicU64>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// Set while a wake message is queued, so workers post at most one.
    wake_pending: Arc<AtomicBool>,

    startup_traced: Cell<bool>,
    done_at: Cell<Option<Instant>>,
}

impl App {
    pub fn new(ui: Controls, dpi: u32) -> App {
        let (tx, rx) = channel();
        let app = App {
            last_focus: Cell::new(ui.list),
            ui,
            dpi: Cell::new(dpi),
            font: Cell::new(HFONT::default()),
            home: path::home(),
            keymap: Keymap::default_map(),
            cwd: RefCell::new(None),
            model: RefCell::new(Model::default()),
            history: RefCell::new(History::default()),
            load: RefCell::new(None),
            loading: Cell::new(false),
            loaded: Cell::new(0),
            restore: RefCell::new(None),
            message: RefCell::new(String::new()),
            last_rebuild: Cell::new(None),
            timer_armed: Cell::new(false),
            generation: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            wake_pending: Arc::new(AtomicBool::new(false)),
            startup_traced: Cell::new(false),
            done_at: Cell::new(None),
        };
        app.apply_font();
        app
    }

    /// Opens the start directory: the argument if given, else home.
    pub fn start(&self, arg: Option<OsString>) {
        unsafe {
            let _ = SetFocus(Some(self.ui.list));
        }
        trace::log(|| {
            format!(
                "window shown after {}",
                trace::ms(trace::since_process_start())
            )
        });
        let mut nav = Nav::new(NavKind::Start);
        let target = match arg {
            Some(a) => {
                nav.fallback_home = true;
                let cwd = std::env::current_dir().unwrap_or_else(|_| self.home.clone());
                path::resolve(&a.to_string_lossy(), &cwd, &self.home)
            }
            None => self.home.clone(),
        };
        self.navigate(target, nav);
    }

    // ---- window messages -------------------------------------------------

    pub fn on_message(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_NOTIFY => {
                let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
                if hdr.hwndFrom == self.ui.list {
                    self.on_list_notify(hdr.code, lparam)
                } else {
                    None
                }
            }
            WM_APP_WAKE => {
                self.drain_events();
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == TIMER_REBUILD => {
                self.rebuild();
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if wparam.0 & 0xFFFF == WA_INACTIVE as usize {
                    let f = unsafe { GetFocus() };
                    if f == self.ui.list || f == self.ui.path {
                        self.last_focus.set(f);
                    }
                }
                None
            }
            WM_SETFOCUS => {
                unsafe {
                    let _ = SetFocus(Some(self.last_focus.get()));
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let old = self.dpi.replace((wparam.0 & 0xFFFF) as u32);
                trace::log(|| format!("dpi changed {old} -> {}", self.dpi.get()));
                self.apply_font();
                list::rescale_columns(self.ui.list, old, self.dpi.get());
                let r = unsafe { &*(lparam.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        self.ui.main,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                self.layout();
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    pub fn layout(&self) {
        let dpi = self.dpi.get();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.ui.main, &mut rc);
        }
        let status_h = statusbar::layout(self.ui.status, dpi);
        let pad = scale(4, dpi);
        let path_h = scale(26, dpi);
        let list_y = path_h + 2 * pad;
        unsafe {
            let _ = MoveWindow(self.ui.path, pad, pad, rc.right - 2 * pad, path_h, true);
            let _ = MoveWindow(
                self.ui.list,
                0,
                list_y,
                rc.right,
                (rc.bottom - list_y - status_h).max(0),
                true,
            );
        }
    }

    fn apply_font(&self) {
        let mut ncm = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let font = unsafe {
            let ok = SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                ncm.cbSize,
                Some(&mut ncm as *mut _ as *mut _),
                0,
                self.dpi.get(),
            );
            if ok.is_err() {
                return;
            }
            CreateFontIndirectW(&ncm.lfMessageFont)
        };
        for hwnd in [self.ui.path, self.ui.list, self.ui.status] {
            send(hwnd, WM_SETFONT, font.0 as usize, 1);
        }
        let old = self.font.replace(font);
        if !old.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
    }

    fn on_list_notify(&self, code: u32, lparam: LPARAM) -> Option<LRESULT> {
        match code {
            LVN_GETDISPINFOW => {
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                if info.item.mask.0 & LVIF_TEXT.0 != 0
                    && let Ok(model) = self.model.try_borrow()
                    && let Some(e) = model.entry(info.item.iItem as usize)
                {
                    let text = cell_text(e, info.item.iSubItem);
                    list::write_text(&mut info.item, &text);
                }
                Some(LRESULT(0))
            }
            NM_CUSTOMDRAW => {
                let cd = unsafe { &mut *(lparam.0 as *mut NMLVCUSTOMDRAW) };
                let r = match cd.nmcd.dwDrawStage {
                    CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW,
                    CDDS_ITEMPREPAINT => {
                        if let Ok(model) = self.model.try_borrow()
                            && let Some(e) = model.entry(cd.nmcd.dwItemSpec)
                        {
                            if e.is_hidden() {
                                cd.clrText = list::HIDDEN_COLOR;
                            } else if e.is_dir() {
                                cd.clrText = list::DIR_COLOR;
                            }
                        }
                        CDRF_NEWFONT
                    }
                    _ => CDRF_DODEFAULT,
                };
                Some(LRESULT(r as isize))
            }
            LVN_ODFINDITEMW => {
                let find = unsafe { &*(lparam.0 as *const NMLVFINDITEMW) };
                Some(LRESULT(self.find_prefix(find).map_or(-1, |i| i as isize)))
            }
            LVN_COLUMNCLICK => {
                let nm = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
                if let Some(key) = SortKey::from_column(nm.iSubItem) {
                    let cur = self.model.borrow().sort;
                    let reverse = cur.key == key && !cur.reverse;
                    let cmd = format!(
                        "sort {}{}",
                        key.name(),
                        if reverse { " --reverse" } else { "" }
                    );
                    self.run_str(&cmd);
                }
                Some(LRESULT(0))
            }
            NM_DBLCLK => {
                let nm = unsafe { &*(lparam.0 as *const NMITEMACTIVATE) };
                if nm.iItem >= 0 {
                    self.run_str("open");
                }
                Some(LRESULT(0))
            }
            LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
                self.update_status();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// Type-to-find over the visible entries, starting at `iStart` and wrapping.
    fn find_prefix(&self, find: &NMLVFINDITEMW) -> Option<usize> {
        if find.lvfi.flags.0 & LVFI_STRING.0 == 0 || find.lvfi.psz.is_null() {
            return None;
        }
        let prefix = unsafe { find.lvfi.psz.as_wide() };
        let model = self.model.try_borrow().ok()?;
        let n = model.visible.len();
        let start = usize::try_from(find.iStart).unwrap_or(0);
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&i| model::has_prefix_ignore_case(&model.entry(i).unwrap().name, prefix))
    }

    // ---- keyboard --------------------------------------------------------

    /// Resolves keyboard messages against the keymap before any control sees
    /// them. Returns `true` when the message was consumed.
    pub fn translate_key(&self, msg: &MSG) -> bool {
        let chord = match msg.message {
            WM_KEYDOWN | WM_SYSKEYDOWN => normalize::from_keydown(
                msg.wParam.0 as u16,
                normalize::ModState::current(),
                &normalize::layout_char,
            ),
            // WM_SYSCHAR (alt+key) is deliberately ignored: its chord was
            // already looked up on key-down with the alt modifier.
            WM_CHAR => normalize::from_char(msg.wParam.0 as u16),
            _ => None,
        };
        let Some(chord) = chord else { return false };
        let Some(inv) = self.keymap.lookup(self.focus_scope(), &chord) else {
            return false;
        };
        self.run(inv);
        true
    }

    fn focus_scope(&self) -> Scope {
        let f = unsafe { GetFocus() };
        if f == self.ui.path {
            Scope::Input
        } else if f == self.ui.list {
            Scope::List
        } else {
            Scope::Global
        }
    }

    fn run(&self, inv: &Invocation) {
        if let Err(e) = inv.run(self) {
            self.set_message(e.to_string());
        }
    }

    fn run_str(&self, s: &str) {
        if let Err(e) = registry::run_str(self, s) {
            self.set_message(e.to_string());
        }
    }

    // ---- navigation ------------------------------------------------------

    fn navigate(&self, path: PathBuf, nav: Nav) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.load.borrow_mut() = Some(Load {
            path: path.clone(),
            nav,
            committed: false,
        });
        self.loading.set(true);
        self.loaded.set(0);
        self.update_status();

        let tx = self.tx.clone();
        let wake = self.wake_pending.clone();
        let hwnd = self.ui.main.0 as usize;
        enumerate::spawn(path, generation, self.generation.clone(), move |m| {
            if tx.send(Event::Fs(m)).is_err() {
                return false;
            }
            if !wake.swap(true, Ordering::AcqRel) {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        WM_APP_WAKE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            true
        });
    }

    fn drain_events(&self) {
        self.wake_pending.store(false, Ordering::Release);
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Fs(m) => self.on_fs(m),
                Event::Message(s) => self.set_message(s),
            }
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn on_fs(&self, msg: FsMsg) {
        match msg {
            FsMsg::Batch {
                generation,
                entries,
            } if self.is_current(generation) => {
                self.commit();
                self.loaded.set(self.loaded.get() + entries.len());
                self.model.borrow_mut().extend(entries);
                self.schedule_rebuild();
            }
            FsMsg::Done { generation } if self.is_current(generation) => {
                self.commit();
                self.loading.set(false);
                self.done_at.set(Some(Instant::now()));
                self.rebuild();
                self.restore.borrow_mut().take();
            }
            FsMsg::Error {
                generation,
                message,
            } if self.is_current(generation) => {
                self.loading.set(false);
                let load = self.load.borrow_mut().take();
                match load {
                    Some(l) if !l.committed && l.nav.fallback_home => {
                        let mut nav = Nav::new(NavKind::Start);
                        nav.message = Some(message);
                        self.navigate(self.home.clone(), nav);
                    }
                    Some(l) if l.committed => {
                        self.set_message(message);
                        self.rebuild();
                    }
                    _ => self.set_message(message),
                }
            }
            _ => {}
        }
    }

    /// Makes the pending navigation current, on its first batch (or on
    /// completion for an empty directory). Until then the old listing stays.
    fn commit(&self) {
        let (path, nav) = {
            let mut load = self.load.borrow_mut();
            let Some(l) = load.as_mut().filter(|l| !l.committed) else {
                return;
            };
            l.committed = true;
            (l.path.clone(), l.nav.clone())
        };

        let old = self.cwd.borrow().clone();
        if let Some(dir) = old {
            let current = Location {
                dir,
                selected: self.focused_name(),
            };
            let mut h = self.history.borrow_mut();
            match nav.kind {
                NavKind::New => h.visit(current),
                NavKind::Back => h.went_back(current),
                NavKind::Forward => h.went_forward(current),
                NavKind::Refresh | NavKind::Start => {}
            }
        }

        let restore = if nav.kind == NavKind::Refresh {
            self.save_selection()
        } else {
            let mut s = Selection::default();
            if let Some(name) = nav.select {
                s.selected.insert(name.clone());
                s.focused = Some(name);
            }
            s
        };
        *self.restore.borrow_mut() = Some(restore);
        self.model.borrow_mut().clear();
        self.last_rebuild.set(None);

        let display = path.display().to_string();
        let title = wide(format!("{display} - lwfm"));
        unsafe {
            let _ = SetWindowTextW(self.ui.main, PCWSTR(title.as_ptr()));
        }
        pathbar::set_text(self.ui.path, &display);
        *self.cwd.borrow_mut() = Some(path);
        *self.message.borrow_mut() = nav.message.unwrap_or_default();
        if nav.focus_list {
            unsafe {
                let _ = SetFocus(Some(self.ui.list));
            }
        }
    }

    fn schedule_rebuild(&self) {
        let due = self
            .last_rebuild
            .get()
            .is_none_or(|t| t.elapsed() >= REBUILD_INTERVAL);
        if due {
            self.rebuild();
        } else if !self.timer_armed.replace(true) {
            unsafe {
                SetTimer(
                    Some(self.ui.main),
                    TIMER_REBUILD,
                    REBUILD_INTERVAL.as_millis() as u32,
                    None,
                )
            };
        }
    }

    /// Re-sorts the view and refreshes the list, keeping the selection on
    /// the same entries (or applying a pending restore target).
    fn rebuild(&self) {
        if self.timer_armed.replace(false) {
            unsafe {
                let _ = KillTimer(Some(self.ui.main), TIMER_REBUILD);
            }
        }
        let pending_restore = self.restore.borrow().is_some();
        let keep = (!pending_restore).then(|| self.save_selection());

        let count = {
            let mut m = self.model.borrow_mut();
            m.rebuild();
            m.visible.len()
        };
        list::set_count(self.ui.list, count);

        if let Some(sel) = keep {
            self.apply_selection(&sel);
        } else {
            let target = self.restore.borrow_mut().take().unwrap_or_default();
            if !self.apply_selection(&target) {
                // Not listed yet: keep waiting while loading, show a cursor meanwhile.
                if count > 0 {
                    list::select(self.ui.list, 0);
                    list::focus(self.ui.list, 0);
                }
                if self.loading.get() {
                    *self.restore.borrow_mut() = Some(target);
                }
            }
        }

        unsafe {
            let _ = InvalidateRect(Some(self.ui.list), None, false);
            let _ = UpdateWindow(self.ui.list);
        }
        self.last_rebuild.set(Some(Instant::now()));
        self.update_status();
        self.trace_paint(count);
    }

    fn trace_paint(&self, count: usize) {
        if !trace::enabled() {
            return;
        }
        if !self.startup_traced.replace(true) {
            trace::log(|| {
                format!(
                    "startup -> first painted listing: {} ({count} items)",
                    trace::ms(trace::since_process_start())
                )
            });
        }
        if let Some(t) = self.done_at.take() {
            trace::log(|| {
                format!(
                    "enumeration complete -> listing painted: {} ({count} items)",
                    trace::ms(t.elapsed())
                )
            });
        }
    }

    fn focused_name(&self) -> Option<Box<[u16]>> {
        let i = list::focused(self.ui.list)?;
        self.model.borrow().entry(i).map(|e| e.name.clone())
    }

    fn save_selection(&self) -> Selection {
        let selected = list::selected(self.ui.list);
        let focused = list::focused(self.ui.list);
        let model = self.model.borrow();
        Selection {
            focused: focused.and_then(|i| model.entry(i)).map(|e| e.name.clone()),
            selected: selected
                .into_iter()
                .filter_map(|i| model.entry(i))
                .map(|e| e.name.clone())
                .collect(),
        }
    }

    /// Applies a selection by name. Returns whether the focused entry was found.
    fn apply_selection(&self, sel: &Selection) -> bool {
        let (indices, focus) = {
            let model = self.model.borrow();
            (
                model.find_all(&sel.selected),
                sel.focused.as_deref().and_then(|n| model.find(n)),
            )
        };
        list::clear_selection(self.ui.list);
        for i in indices {
            list::select(self.ui.list, i);
        }
        if let Some(i) = focus {
            list::focus(self.ui.list, i);
        }
        focus.is_some()
    }

    // ---- status ----------------------------------------------------------

    fn set_message(&self, s: String) {
        *self.message.borrow_mut() = s;
        self.update_status();
    }

    fn update_status(&self) {
        let counts = if self.loading.get() {
            format!("Loading... {} items", self.loaded.get())
        } else {
            let n = self
                .model
                .try_borrow()
                .map(|m| m.visible.len())
                .unwrap_or(0);
            match list::selected_count(self.ui.list) {
                0 => format!("{n} items"),
                s => format!("{n} items, {s} selected"),
            }
        };
        statusbar::set_part(self.ui.status, 0, &counts);
        let msg = self.message.borrow().clone();
        statusbar::set_part(self.ui.status, 1, &msg);
    }

    // ---- helpers ---------------------------------------------------------

    fn base_dir(&self) -> PathBuf {
        self.cwd
            .borrow()
            .clone()
            .unwrap_or_else(|| self.home.clone())
    }

    fn cd_with(&self, input: &str, focus_list: bool) {
        let target = path::resolve(input, &self.base_dir(), &self.home);
        let mut nav = Nav::new(NavKind::New);
        nav.focus_list = focus_list;
        self.navigate(target, nav);
    }

    fn shell_open(&self, file: PathBuf, dir: PathBuf) {
        let tx = self.tx.clone();
        let wake = self.wake_pending.clone();
        let hwnd = self.ui.main.0 as usize;
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let file_w = wide(&file);
            let dir_w = wide(&dir);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOASYNC,
                hwnd: HWND(hwnd as *mut _),
                lpFile: PCWSTR(file_w.as_ptr()),
                lpDirectory: PCWSTR(dir_w.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            if let Err(e) = ShellExecuteExW(&mut info)
                && e.code() != ERROR_CANCELLED.to_hresult()
            {
                let msg = format!("Cannot open {}: {}", file.display(), e.message());
                if tx.send(Event::Message(msg)).is_ok() && !wake.swap(true, Ordering::AcqRel) {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        WM_APP_WAKE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            CoUninitialize();
        });
    }
}

/// Text of one list cell.
fn cell_text(e: &Entry, column: i32) -> Vec<u16> {
    match column {
        0 => {
            let mut v = e.name.to_vec();
            if e.is_dir() {
                v.push(b'\\' as u16);
            }
            v
        }
        1 if e.is_dir() => Vec::new(),
        1 => entry::format_size(e.size).encode_utf16().collect(),
        2 => entry::to_local(e.mtime)
            .map(|t| entry::format_time(&t))
            .unwrap_or_default()
            .encode_utf16()
            .collect(),
        _ => Vec::new(),
    }
}

fn os_name(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

impl Target for App {
    fn cd(&self, input: &str) -> CmdResult {
        self.cd_with(input, false);
        Ok(())
    }

    fn up(&self) -> CmdResult {
        let Some(cwd) = self.cwd.borrow().clone() else {
            return Ok(());
        };
        let Some(parent) = path::parent(&cwd) else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::New);
        nav.select = cwd
            .file_name()
            .map(|n| wide(n)[..].split_last().unwrap().1.into());
        self.navigate(parent, nav);
        Ok(())
    }

    fn back(&self) -> CmdResult {
        let Some(target) = self.history.borrow().back_target().cloned() else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::Back);
        nav.select = target.selected;
        self.navigate(target.dir, nav);
        Ok(())
    }

    fn forward(&self) -> CmdResult {
        let Some(target) = self.history.borrow().forward_target().cloned() else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::Forward);
        nav.select = target.selected;
        self.navigate(target.dir, nav);
        Ok(())
    }

    fn open(&self) -> CmdResult {
        let Some(cwd) = self.cwd.borrow().clone() else {
            return Ok(());
        };
        let Some(i) = list::focused(self.ui.list) else {
            return Ok(());
        };
        let Some((name, is_dir)) = self
            .model
            .borrow()
            .entry(i)
            .map(|e| (os_name(&e.name), e.is_dir()))
        else {
            return Ok(());
        };
        let target = cwd.join(name);
        if is_dir {
            self.navigate(target, Nav::new(NavKind::New));
        } else {
            self.shell_open(target, cwd);
        }
        Ok(())
    }

    fn refresh(&self) -> CmdResult {
        if let Some(cwd) = self.cwd.borrow().clone() {
            self.navigate(cwd, Nav::new(NavKind::Refresh));
        }
        Ok(())
    }

    fn toggle_hidden(&self) -> CmdResult {
        {
            let mut m = self.model.borrow_mut();
            m.show_hidden = !m.show_hidden;
        }
        self.rebuild();
        Ok(())
    }

    fn sort(&self, sort: Sort) -> CmdResult {
        self.model.borrow_mut().sort = sort;
        self.rebuild();
        Ok(())
    }

    fn focus_path(&self) -> CmdResult {
        unsafe {
            let _ = SetFocus(Some(self.ui.path));
        }
        pathbar::select_all(self.ui.path);
        Ok(())
    }

    fn focus_list(&self) -> CmdResult {
        unsafe {
            let _ = SetFocus(Some(self.ui.list));
        }
        Ok(())
    }

    fn path_submit(&self) -> CmdResult {
        let text = pathbar::text(self.ui.path);
        if text.trim().is_empty() {
            return Ok(());
        }
        self.cd_with(&text, true);
        Ok(())
    }

    fn path_cancel(&self) -> CmdResult {
        let cwd = self
            .cwd
            .borrow()
            .as_deref()
            .map(Path::display)
            .map(|d| d.to_string());
        pathbar::set_text(self.ui.path, &cwd.unwrap_or_default());
        self.focus_list()
    }

    fn quit(&self) -> CmdResult {
        unsafe {
            let _ = DestroyWindow(self.ui.main);
        }
        Ok(())
    }
}
