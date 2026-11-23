//! Application state and behaviour. All state lives on the UI thread behind
//! `Cell`/`RefCell`; borrows are kept short and never held across calls that
//! may re-enter the window procedure (e.g. `SendMessageW` to the list).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOWTEXT, CreateFontIndirectW, CreateSolidBrush, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, FillRect, GetSysColor, GetTextExtentPoint32W, HFONT,
    HGDIOBJ, InvalidateRect, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
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

use crate::cmd::registry::{self, CmdError, CmdResult, Target};
use crate::config::discover::Env;
use crate::config::{self, Loaded, watch};
use crate::fs::entry::{self, Entry};
use crate::fs::enumerate::{self, FsMsg};
use crate::fs::model::{Model, Sort, SortKey};
use crate::fs::path;
use crate::keys::chord::{Chord, Key, Mods, Named};
use crate::keys::keymap::{Action, Binding, Lookup, Scope};
use crate::keys::normalize;
use crate::lua::api::{self, Event as LuaEvent, FmState, Host};
use crate::nav::{History, Location};
use crate::search::Query;
use crate::shell;
use crate::trace;
use crate::ui::{list, pathbar, scale, searchbox, send, statusbar, wide};

const WM_APP_WAKE: u32 = WM_APP + 1;
const TIMER_REBUILD: usize = 1;
/// Highlight behind the matched part of names while searching.
const MATCH_COLOR: COLORREF = COLORREF(0x0000_E6FF); // RGB(255, 230, 0)
/// Left margin of Name text inside the label rectangle (at 96 DPI), as the
/// list control draws it.
const LABEL_MARGIN: i32 = 2;
/// While a directory is loading, the list is re-sorted at most this often.
const REBUILD_INTERVAL: Duration = Duration::from_millis(100);

pub struct Controls {
    pub main: HWND,
    pub path: HWND,
    pub list: HWND,
    pub status: HWND,
    /// The find-as-you-type input, hidden while no search is active.
    pub search: HWND,
}

/// What lwfm was started with, besides the window.
pub struct Startup {
    pub dir: Option<OsString>,
    pub config: Loaded,
    /// The config file found at startup, even if it failed to load.
    pub config_file: Option<PathBuf>,
    pub env: Env,
    /// Config problems to show once the first listing appears.
    pub message: Option<String>,
}

enum Event {
    Fs(FsMsg),
    Message(String),
    /// New content of the watched config file (or a read error).
    ConfigChanged(PathBuf, Result<String, String>),
    /// `edit-config` made sure the file exists; `created` if it wrote the starter.
    ConfigReady {
        path: PathBuf,
        created: bool,
    },
}

/// Sends events from worker threads and wakes the UI thread at most once per
/// batch of queued events.
#[derive(Clone)]
struct Waker {
    tx: Sender<Event>,
    pending: Arc<AtomicBool>,
    hwnd: usize,
}

impl Waker {
    fn send(&self, ev: Event) -> bool {
        if self.tx.send(ev).is_err() {
            return false;
        }
        if !self.pending.swap(true, Ordering::AcqRel) {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_APP_WAKE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        true
    }
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

fn join_messages(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} | {b}")),
        (a, b) => a.or(b),
    }
}

pub struct App {
    ui: Controls,
    dpi: Cell<u32>,
    font: Cell<HFONT>,
    home: PathBuf,

    config: RefCell<Rc<Loaded>>,
    /// The config file in use or watched (it may have failed to load).
    config_file: RefCell<Option<PathBuf>>,
    config_env: Env,
    watcher: RefCell<Option<watch::Watcher>>,
    /// Chords of a key sequence typed so far.
    pending: RefCell<Vec<Chord>>,
    /// Hint bar text while a sequence is pending; replaces the message.
    hint: RefCell<Option<String>>,
    /// `$f` for the next `shell` run (the config file for `edit-config`).
    file_override: RefCell<Option<PathBuf>>,
    startup_fired: Cell<bool>,

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
    waker: Waker,
    rx: Receiver<Event>,

    /// Find-as-you-type: the current text (empty when the input is closed)
    /// and how many visible entries match it.
    search: RefCell<Query>,
    search_matches: Cell<usize>,
    /// Set while the app itself changes the search input, so its
    /// notifications do not re-enter.
    search_guard: Cell<bool>,

    startup_traced: Cell<bool>,
    done_at: Cell<Option<Instant>>,
}

impl App {
    pub fn new(ui: Controls, dpi: u32, startup: &mut Startup) -> App {
        let (tx, rx) = channel();
        let waker = Waker {
            tx,
            pending: Arc::new(AtomicBool::new(false)),
            hwnd: ui.main.0 as usize,
        };
        let loaded = std::mem::replace(&mut startup.config, config::defaults());
        let app = App {
            last_focus: Cell::new(ui.list),
            ui,
            dpi: Cell::new(dpi),
            font: Cell::new(HFONT::default()),
            home: path::home(),
            config: RefCell::new(Rc::new(loaded)),
            config_file: RefCell::new(startup.config_file.clone()),
            config_env: startup.env.clone(),
            watcher: RefCell::new(None),
            pending: RefCell::new(Vec::new()),
            hint: RefCell::new(None),
            file_override: RefCell::new(None),
            startup_fired: Cell::new(false),
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
            waker,
            rx,
            search: RefCell::new(Query::default()),
            search_matches: Cell::new(0),
            search_guard: Cell::new(false),
            startup_traced: Cell::new(false),
            done_at: Cell::new(None),
        };
        app.apply_font();
        app.apply_settings();
        if let Some(p) = startup.config_file.clone() {
            app.watch_config(p);
        }
        app
    }

    /// Opens the start directory: the argument if given, else home.
    pub fn start(&self, startup: &mut Startup) {
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
        nav.message = startup.message.take();
        let target = match startup.dir.take() {
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
            WM_COMMAND if lparam.0 == self.ui.search.0 as isize => {
                match ((wparam.0 >> 16) & 0xFFFF) as u32 {
                    EN_CHANGE if !self.search_guard.get() => self.on_search_changed(),
                    EN_KILLFOCUS if !self.search_guard.get() => self.close_search(),
                    _ => {}
                }
                Some(LRESULT(0))
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
        let list_h = (rc.bottom - list_y - status_h).max(0);
        let (search_w, search_h) = (scale(searchbox::WIDTH, dpi), scale(searchbox::HEIGHT, dpi));
        unsafe {
            let _ = MoveWindow(self.ui.path, pad, pad, rc.right - 2 * pad, path_h, true);
            let _ = MoveWindow(self.ui.list, 0, list_y, rc.right, list_h, true);
            // Over the list's bottom-left corner, above the list in z-order.
            let _ = SetWindowPos(
                self.ui.search,
                Some(HWND_TOP),
                pad,
                list_y + list_h - search_h - pad,
                search_w.min(rc.right - 2 * pad),
                search_h,
                SWP_NOACTIVATE,
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
        for hwnd in [self.ui.path, self.ui.list, self.ui.status, self.ui.search] {
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
                let searching = !self.search.borrow().is_empty();
                let subitem = NMCUSTOMDRAW_DRAW_STAGE(CDDS_ITEMPREPAINT.0 | CDDS_SUBITEM.0);
                let r = match cd.nmcd.dwDrawStage {
                    CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW,
                    CDDS_ITEMPREPAINT => {
                        self.set_row_color(cd);
                        // While searching, Name cells with a match are drawn by hand.
                        CDRF_NEWFONT | if searching { CDRF_NOTIFYSUBITEMDRAW } else { 0 }
                    }
                    stage if stage == subitem => {
                        self.set_row_color(cd);
                        if cd.iSubItem == 0 && searching && self.paint_match(cd) {
                            CDRF_SKIPDEFAULT
                        } else {
                            CDRF_NEWFONT
                        }
                    }
                    _ => CDRF_DODEFAULT,
                };
                Some(LRESULT(r as isize))
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

    /// Directory and hidden-item text colors from the config.
    fn set_row_color(&self, cd: &mut NMLVCUSTOMDRAW) {
        let colors = self.config.borrow().settings.colors;
        if let Ok(model) = self.model.try_borrow()
            && let Some(e) = model.entry(cd.nmcd.dwItemSpec)
        {
            if e.is_hidden() {
                cd.clrText = COLORREF(colors.hidden);
            } else if e.is_dir() {
                cd.clrText = COLORREF(colors.dir);
            }
        }
    }

    /// Draws the Name cell of a row whose name matches the search, with the
    /// match on a highlight background. Returns `false` (default drawing)
    /// when the name does not match.
    fn paint_match(&self, cd: &NMLVCUSTOMDRAW) -> bool {
        let row = cd.nmcd.dwItemSpec;
        let Some((mut text, range)) = self.model.try_borrow().ok().and_then(|m| {
            let e = m.entry(row)?;
            let range = self.search.borrow().find(&e.name)?;
            Some((cell_text(e, 0), range))
        }) else {
            return false;
        };
        let hdc = cd.nmcd.hdc;
        let rc = list::label_rect(self.ui.list, row);
        let color = if cd.clrText.0 & 0xFF00_0000 != 0 {
            COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) })
        } else {
            cd.clrText
        };
        let width = |s: &[u16]| {
            let mut size = SIZE::default();
            unsafe {
                let _ = GetTextExtentPoint32W(hdc, s, &mut size);
            }
            size
        };
        let flags = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
        let mut x = rc.left + scale(LABEL_MARGIN, self.dpi.get());
        let draw = |segment: &mut [u16], x: i32| {
            let mut r = RECT { left: x, ..rc };
            unsafe { DrawTextW(hdc, segment, &mut r, flags) };
        };
        unsafe {
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, color);
        }
        let (head, rest) = text.split_at_mut(range.start);
        let (hit, tail) = rest.split_at_mut(range.end - range.start);
        draw(head, x);
        x += width(head).cx;
        let size = width(hit);
        let top = rc.top + (rc.bottom - rc.top - size.cy) / 2;
        let mark = RECT {
            left: x,
            top,
            right: (x + size.cx).min(rc.right),
            bottom: top + size.cy,
        };
        unsafe {
            let brush = CreateSolidBrush(MATCH_COLOR);
            FillRect(hdc, &mark, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        draw(hit, x);
        x += size.cx;
        draw(tail, x);
        true
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
        let chain = self.scope_chain();
        let mut seq = self.pending.borrow().clone();
        let pending = !seq.is_empty();
        if pending && chord == Chord::new(Mods::NONE, Key::Named(Named::Esc)) {
            self.end_sequence();
            return true;
        }
        seq.push(chord);
        let config = self.config.borrow().clone();
        match config.keymap.lookup(&chain, &seq) {
            Lookup::Match(b) => {
                self.end_sequence();
                self.run_binding(&config, b);
                true
            }
            Lookup::Prefix(hints) => {
                let keys: Vec<String> = seq.iter().map(Chord::to_string).collect();
                let items: Vec<String> = hints.iter().map(|(c, l)| format!("{c} {l}")).collect();
                *self.hint.borrow_mut() = Some(format!(
                    "{} ...   {}   (esc cancels)",
                    keys.join(" "),
                    items.join("   ")
                ));
                *self.pending.borrow_mut() = seq;
                self.update_status();
                true
            }
            Lookup::None if pending => {
                // A chord that continues no binding ends the sequence and is dropped.
                self.end_sequence();
                true
            }
            Lookup::None => {
                // Typing in the list opens the search input.
                if msg.message == WM_CHAR
                    && chain.contains(&Scope::List)
                    && let Key::Char(c) = chord.key
                {
                    let inv = registry::compile_parts("search", vec![c.to_string()], vec![]);
                    if let Err(e) = inv.and_then(|inv| inv.run(self)) {
                        self.set_message(e.0);
                    }
                    return true;
                }
                false
            }
        }
    }

    fn scope_chain(&self) -> Vec<Scope> {
        let f = unsafe { GetFocus() };
        if f == self.ui.path {
            vec![Scope::Input, Scope::Global]
        } else if f == self.ui.search {
            vec![Scope::Search, Scope::Global]
        } else if f == self.ui.list {
            vec![Scope::List, Scope::Global]
        } else {
            vec![Scope::Global]
        }
    }

    fn end_sequence(&self) {
        self.pending.borrow_mut().clear();
        if self.hint.borrow_mut().take().is_some() {
            self.update_status();
        }
    }

    /// Runs a key binding or opener. `config` keeps the Lua state alive even
    /// if the action reloads the configuration.
    fn run_binding(&self, config: &Rc<Loaded>, b: &Binding) {
        match &b.action {
            Action::Commands(invs) => {
                for inv in invs {
                    if let Err(e) = inv.run(self) {
                        self.set_message(e.0);
                        break;
                    }
                }
            }
            Action::Lua(f) => {
                let state = self.fm_state();
                if let Err(e) = api::call(&config.lua, self, &state, f, vec![]) {
                    self.set_message(e);
                }
            }
        }
    }

    fn run_str(&self, s: &str) {
        if let Err(e) = registry::run_str(self, s) {
            self.set_message(e.to_string());
        }
    }

    fn fire(&self, event: LuaEvent, extra: Vec<String>) {
        let config = self.config.borrow().clone();
        let Some(handlers) = config.handlers.get(&event) else {
            return;
        };
        for f in handlers {
            let state = self.fm_state();
            if let Err(e) = api::call(&config.lua, self, &state, f, extra.clone()) {
                self.set_message(e);
            }
        }
    }

    // ---- state for Lua and shell commands ----------------------------------

    fn entry_path(&self, i: usize) -> Option<PathBuf> {
        let cwd = self.cwd.borrow().clone()?;
        let name = self.model.borrow().entry(i).map(|e| os_name(&e.name))?;
        Some(cwd.join(name))
    }

    fn focused_path(&self) -> Option<PathBuf> {
        self.entry_path(list::focused(self.ui.list)?)
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        list::selected(self.ui.list)
            .into_iter()
            .filter_map(|i| self.entry_path(i))
            .collect()
    }

    fn fm_state(&self) -> FmState {
        let text = |p: PathBuf| p.display().to_string();
        FmState {
            dir: text(self.base_dir()),
            file: self.focused_path().map(text),
            selection: self.selected_paths().into_iter().map(text).collect(),
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

        let waker = self.waker.clone();
        enumerate::spawn(path, generation, self.generation.clone(), move |m| {
            waker.send(Event::Fs(m))
        });
    }

    fn drain_events(&self) {
        self.waker.pending.store(false, Ordering::Release);
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Fs(m) => self.on_fs(m),
                Event::Message(s) => self.set_message(s),
                Event::ConfigChanged(path, source) => self.on_config_changed(path, source),
                Event::ConfigReady { path, created } => self.on_config_ready(path, created),
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
                let kind = self.load.borrow().as_ref().map(|l| l.nav.kind);
                if kind.is_some_and(|k| k != NavKind::Refresh) {
                    if !self.startup_fired.replace(true) {
                        self.fire(LuaEvent::Startup, vec![]);
                    }
                    self.fire(
                        LuaEvent::DirEnter,
                        vec![self.base_dir().display().to_string()],
                    );
                }
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
                        nav.message = join_messages(Some(message), l.nav.message);
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
        // A search belongs to one directory; refresh keeps it.
        if nav.kind != NavKind::Refresh {
            self.close_search();
        }
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

    /// Re-sorts and re-filters the view and refreshes the list, keeping the
    /// selection on the same entries (or applying a pending restore target).
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

        let found = match keep {
            Some(sel) => self.apply_selection(&sel),
            None => {
                let target = self.restore.borrow_mut().take().unwrap_or_default();
                let found = self.apply_selection(&target);
                if !found && self.loading.get() {
                    // Not listed yet: keep waiting while loading.
                    *self.restore.borrow_mut() = Some(target);
                }
                found
            }
        };
        if !found && count > 0 {
            // Always show a cursor; it lands on the first entry.
            list::select(self.ui.list, 0);
            list::focus(self.ui.list, 0);
        }

        unsafe {
            let _ = InvalidateRect(Some(self.ui.list), None, false);
            let _ = UpdateWindow(self.ui.list);
        }
        self.last_rebuild.set(Some(Instant::now()));
        if !self.search.borrow().is_empty() {
            self.count_matches();
        }
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

    // ---- search ----------------------------------------------------------

    fn count_matches(&self) {
        let n = self.model.borrow().count_matches(&self.search.borrow());
        self.search_matches.set(n);
    }

    /// The search input's text changed: select the first match.
    fn on_search_changed(&self) {
        let text = pathbar::text(self.ui.search);
        if text.is_empty() {
            self.close_search();
            return;
        }
        let started = Instant::now();
        *self.search.borrow_mut() = Query::new(&text);
        let first = self
            .model
            .borrow()
            .next_match(&self.search.borrow(), None, true);
        if let Some(i) = first {
            list::select_only(self.ui.list, i);
        }
        self.count_matches();
        unsafe {
            let _ = InvalidateRect(Some(self.ui.list), None, false);
        }
        self.update_status();
        trace::log(|| {
            format!(
                "search: {} ({} matches)",
                trace::ms(started.elapsed()),
                self.search_matches.get()
            )
        });
    }

    fn step_match(&self, forward: bool) {
        if self.search.borrow().is_empty() {
            return;
        }
        let from = list::focused(self.ui.list);
        let next = self
            .model
            .borrow()
            .next_match(&self.search.borrow(), from, forward);
        if let Some(i) = next {
            list::select_only(self.ui.list, i);
        }
    }

    /// Hides the search input and its highlights; the selection stays.
    fn close_search(&self) {
        if self.search.borrow().is_empty() && !searchbox::is_visible(self.ui.search) {
            return;
        }
        self.search_guard.set(true);
        *self.search.borrow_mut() = Query::default();
        if unsafe { GetFocus() } == self.ui.search {
            unsafe {
                let _ = SetFocus(Some(self.ui.list));
            }
        }
        pathbar::set_text(self.ui.search, "");
        searchbox::hide(self.ui.search);
        self.search_guard.set(false);
        unsafe {
            let _ = InvalidateRect(Some(self.ui.list), None, false);
        }
        self.update_status();
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
            let query = self.search.borrow().text().to_string();
            let selected = list::selected_count(self.ui.list);
            match (query.is_empty(), self.search_matches.get(), selected) {
                (false, 0, _) => format!("Search: {query}  no matches"),
                (false, 1, _) => format!("Search: {query}  1 match"),
                (false, m, _) => format!("Search: {query}  {m} matches"),
                (true, _, 0) => format!("{n} items"),
                (true, _, s) => format!("{n} items, {s} selected"),
            }
        };
        statusbar::set_part(self.ui.status, 0, &counts);
        let msg = match self.hint.borrow().as_ref() {
            Some(h) => h.clone(),
            None => self.message.borrow().clone(),
        };
        statusbar::set_part(self.ui.status, 1, &msg);
    }

    // ---- configuration ---------------------------------------------------

    fn apply_settings(&self) {
        let (show_hidden, sort) = {
            let c = self.config.borrow();
            (c.settings.show_hidden, c.settings.sort)
        };
        {
            let mut m = self.model.borrow_mut();
            m.show_hidden = show_hidden;
            m.sort = sort;
        }
    }

    fn watch_config(&self, path: PathBuf) {
        if self
            .watcher
            .borrow()
            .as_ref()
            .is_some_and(|w| w.path == path)
        {
            return;
        }
        let waker = self.waker.clone();
        let w = watch::watch(path, move |p, source| {
            waker.send(Event::ConfigChanged(p, source));
        });
        *self.watcher.borrow_mut() = w;
    }

    fn on_config_changed(&self, path: PathBuf, source: Result<String, String>) {
        let started = Instant::now();
        let loaded = source.and_then(|src| config::load(Some(&path), Some(&src)));
        trace::log(|| {
            format!(
                "config loaded in {} ({})",
                trace::ms(started.elapsed()),
                path.display()
            )
        });
        match loaded {
            Ok(l) => {
                let msg = if l.warnings.is_empty() {
                    "Config reloaded".to_string()
                } else {
                    format!("Config reloaded with warnings: {}", l.warnings.join("; "))
                };
                *self.config_file.borrow_mut() = Some(path.clone());
                *self.config.borrow_mut() = Rc::new(l);
                self.end_sequence();
                self.apply_settings();
                self.rebuild();
                self.watch_config(path);
                self.set_message(msg);
            }
            Err(e) => self.set_message(e),
        }
    }

    fn on_config_ready(&self, path: PathBuf, created: bool) {
        if created {
            self.on_config_changed(path.clone(), Ok(config::STARTER.to_string()));
            self.set_message(format!("Created {}", path.display()));
        }
        let editor = self.config.borrow().settings.editor.def.name;
        *self.file_override.borrow_mut() = Some(path);
        let config = self.config.borrow().clone();
        let result = config.settings.editor.run(self);
        self.file_override.borrow_mut().take();
        if let Err(e) = result {
            self.set_message(format!("editor ({editor}): {e}"));
        }
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
        let waker = self.waker.clone();
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
                waker.send(Event::Message(msg));
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

impl Host for App {
    fn notify(&self, msg: &str) {
        self.set_message(msg.to_string());
    }
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
        let target = cwd.join(&name);
        if is_dir {
            self.navigate(target, Nav::new(NavKind::New));
            return Ok(());
        }
        let config = self.config.borrow().clone();
        let file_name = name.to_string_lossy();
        let opener = shell::best_opener(config.openers.iter().map(|(p, _)| p.as_str()), &file_name);
        match opener {
            Some(k) => self.run_binding(&config, &config.openers[k].1),
            None => self.shell_open(target, cwd),
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
        if let Some(command) = text.trim_start().strip_prefix(':') {
            if command.trim().is_empty() {
                return self.path_cancel();
            }
            registry::run_str(self, command)?;
            // Success: show the directory again (unless the command moved focus there).
            if unsafe { GetFocus() } == self.ui.path {
                self.path_cancel()?;
            }
            return Ok(());
        }
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

    fn shell(&self, command: &str, console: bool) -> CmdResult {
        let dir = self.base_dir();
        let focused = self
            .file_override
            .borrow()
            .clone()
            .or_else(|| self.focused_path());
        let selection = if self.file_override.borrow().is_some() {
            Vec::new()
        } else {
            self.selected_paths()
        };
        let ctx = shell::Context {
            dir: &dir,
            focused: focused.as_deref(),
            selection: &selection,
        };
        let line = shell::expand(command, &ctx).map_err(CmdError)?;
        let waker = self.waker.clone();
        shell::launch(line, dir.clone(), console, move |msg| {
            waker.send(Event::Message(msg));
        });
        Ok(())
    }

    fn search(&self, text: &str) -> CmdResult {
        searchbox::show(self.ui.search);
        unsafe {
            let _ = SetFocus(Some(self.ui.search));
        }
        // Setting the text runs the jump through EN_CHANGE.
        pathbar::set_text(self.ui.search, text);
        let end = text.encode_utf16().count();
        send(self.ui.search, EM_SETSEL, end, end as isize);
        if text.is_empty() {
            self.close_search();
        }
        Ok(())
    }

    fn search_next(&self) -> CmdResult {
        self.step_match(true);
        Ok(())
    }

    fn search_prev(&self) -> CmdResult {
        self.step_match(false);
        Ok(())
    }

    fn search_close(&self) -> CmdResult {
        self.close_search();
        Ok(())
    }

    fn prompt(&self) -> CmdResult {
        unsafe {
            let _ = SetFocus(Some(self.ui.path));
        }
        pathbar::set_text(self.ui.path, ":");
        send(self.ui.path, EM_SETSEL, 1, 1);
        Ok(())
    }

    fn edit_config(&self) -> CmdResult {
        let path = self
            .config_file
            .borrow()
            .clone()
            .or_else(|| self.config_env.default_location())
            .ok_or_else(|| CmdError("no location for a config file (APPDATA is not set)".into()))?;
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let created = !path.exists();
            if created {
                let write = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|_| std::fs::write(&path, config::STARTER));
                if let Err(e) = write {
                    waker.send(Event::Message(format!(
                        "Cannot create {}: {e}",
                        path.display()
                    )));
                    return;
                }
            }
            waker.send(Event::ConfigReady { path, created });
        });
        Ok(())
    }

    fn reload_config(&self) -> CmdResult {
        let known = self.config_file.borrow().clone();
        let env = self.config_env.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let found =
                known.or_else(
                    || match config::discover::discover(&env, &|p| p.is_file()) {
                        config::discover::Found::File(p) => Some(p),
                        _ => None,
                    },
                );
            match found {
                Some(path) => {
                    let source = config::read_file(&path);
                    waker.send(Event::ConfigChanged(path, source))
                }
                None => waker.send(Event::Message("No config file to reload".into())),
            };
        });
        Ok(())
    }
}
