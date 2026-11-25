//! Application state and behaviour. All state lives on the UI thread behind
//! `Cell`/`RefCell`; borrows are kept short and never held across calls into
//! the window or its controls, which may call back into the application
//! (the list asks for cell text while it is being changed, for example).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::ops::Range;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use lwfm_win::controls::{
    CellBuf, Column, Edit, EditEvent, EditId, ListId, ListSource, ListView, StatusBar,
};
use lwfm_win::input::{KeyEvent, VirtualKey};
use lwfm_win::window::{Handler, TimerId, UiWaker, Window, WindowEvent};
use lwfm_win::{Color, Rect, process};

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

const LIST: ListId = ListId(1);
const PATH: EditId = EditId(2);
const STATUS: usize = 3;
const SEARCH: EditId = EditId(4);
const TIMER_REBUILD: TimerId = TimerId(1);
/// While a directory is loading, the list is re-sorted at most this often.
const REBUILD_INTERVAL: Duration = Duration::from_millis(100);

const COLUMNS: [Column; 3] = [
    Column {
        title: "Name",
        width: 360,
        align_right: false,
    },
    Column {
        title: "Size",
        width: 90,
        align_right: true,
    },
    Column {
        title: "Modified",
        width: 140,
        align_right: false,
    },
];

// Layout measures in pixels at 96 DPI.
const PADDING: i32 = 4;
const PATH_HEIGHT: i32 = 26;
const SEARCH_WIDTH: i32 = 240;
const SEARCH_HEIGHT: i32 = 24;
/// Width of the status bar part that shows the item counts.
const STATUS_COUNTS_WIDTH: i32 = 280;

struct Controls {
    window: Window,
    path: Edit,
    list: ListView,
    status: StatusBar,
    /// The find-as-you-type input, hidden while no search is active and
    /// raised above the list whenever it is shown.
    search: Edit,
}

impl Controls {
    fn create() -> Result<Controls, lwfm_win::Error> {
        let window = Window::create("lwfm")?;
        Ok(Controls {
            path: Edit::new(&window, PATH, true)?,
            list: ListView::new(&window, LIST, &COLUMNS)?,
            status: StatusBar::new(&window, STATUS)?,
            search: Edit::new(&window, SEARCH, false)?,
            window,
        })
    }
}

/// What lwfm was started with.
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

/// Sends events from worker threads and wakes the UI thread.
#[derive(Clone)]
struct Waker {
    tx: Sender<Event>,
    ui: UiWaker,
}

impl Waker {
    /// Returns `false` once the receiver is gone.
    fn send(&self, ev: Event) -> bool {
        if self.tx.send(ev).is_err() {
            return false;
        }
        self.ui.wake();
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

    generation: Arc<AtomicU64>,
    waker: Waker,
    rx: Receiver<Event>,

    /// Find-as-you-type: whether the input is open, its current text and
    /// how many visible entries match it.
    search_open: Cell<bool>,
    search: RefCell<Query>,
    search_matches: Cell<usize>,

    startup_traced: Cell<bool>,
    done_at: Cell<Option<Instant>>,
}

/// Creates the window, runs the message loop and returns the exit code.
pub fn run(startup: Startup) -> Result<i32, lwfm_win::Error> {
    let Startup {
        dir,
        config,
        config_file,
        env,
        message,
    } = startup;
    let app = App::new(Controls::create()?, config, config_file, env);
    app.layout();
    app.ui.window.show();
    app.start(dir, message);
    Ok(app.ui.window.run(&app))
}

impl App {
    fn new(ui: Controls, config: Loaded, config_file: Option<PathBuf>, env: Env) -> App {
        let (tx, rx) = channel();
        let waker = Waker {
            tx,
            ui: ui.window.waker(),
        };
        let app = App {
            ui,
            home: path::home(),
            config: RefCell::new(Rc::new(config)),
            config_file: RefCell::new(config_file.clone()),
            config_env: env,
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
            search_open: Cell::new(false),
            search: RefCell::new(Query::default()),
            search_matches: Cell::new(0),
            startup_traced: Cell::new(false),
            done_at: Cell::new(None),
        };
        app.apply_settings();
        if let Some(p) = config_file {
            app.watch_config(p);
        }
        app
    }

    /// Opens the start directory: the argument if given, else home.
    fn start(&self, dir: Option<OsString>, message: Option<String>) {
        self.ui.list.focus_control();
        trace::log(|| {
            format!(
                "window shown after {}",
                trace::ms(trace::since_process_start())
            )
        });
        let mut nav = Nav::new(NavKind::Start);
        nav.message = message;
        let target = match dir {
            Some(a) => {
                nav.fallback_home = true;
                let cwd = std::env::current_dir().unwrap_or_else(|_| self.home.clone());
                path::resolve(&a.to_string_lossy(), &cwd, &self.home)
            }
            None => self.home.clone(),
        };
        self.navigate(target, nav);
    }

    fn layout(&self) {
        let dpi = self.ui.window.dpi();
        let client = self.ui.window.client_size();
        let status_h = self.ui.status.layout(STATUS_COUNTS_WIDTH);
        let pad = dpi.scale(PADDING);
        let path_h = dpi.scale(PATH_HEIGHT);
        let list_y = path_h + 2 * pad;
        let list_h = (client.height - list_y - status_h).max(0);
        let (search_w, search_h) = (dpi.scale(SEARCH_WIDTH), dpi.scale(SEARCH_HEIGHT));
        self.ui
            .path
            .set_bounds(Rect::new(pad, pad, client.width - 2 * pad, path_h));
        self.ui
            .list
            .set_bounds(Rect::new(0, list_y, client.width, list_h));
        // Over the list's bottom-left corner.
        self.ui.search.set_bounds(Rect::new(
            pad,
            list_y + list_h - search_h - pad,
            search_w.min(client.width - 2 * pad),
            search_h,
        ));
    }

    // ---- keyboard --------------------------------------------------------

    /// Resolves keyboard events against the keymap before any control sees
    /// them. Returns `true` when the event was consumed.
    fn translate_key(&self, event: KeyEvent) -> bool {
        let (chord, typed) = match event {
            KeyEvent::Down { key, mods } => (
                normalize::from_keydown(key, mods, &VirtualKey::layout_char),
                false,
            ),
            KeyEvent::Char(c) => (normalize::from_char(c), true),
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
                if typed
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
        if self.ui.path.has_focus() {
            vec![Scope::Input, Scope::Global]
        } else if self.ui.search.has_focus() {
            vec![Scope::Search, Scope::Global]
        } else if self.ui.list.has_focus() {
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
        self.entry_path(self.ui.list.focused()?)
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.ui
            .list
            .selected()
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
        let display = path.display().to_string();
        let spawned = enumerate::spawn(path, generation, self.generation.clone(), move |m| {
            waker.send(Event::Fs(m))
        });
        if let Err(e) = spawned {
            self.on_fs(FsMsg::Error {
                generation,
                message: format!("Cannot list {display}: {e}"),
            });
        }
    }

    fn drain_events(&self) {
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
        self.ui.window.set_title(&format!("{display} - lwfm"));
        self.ui.path.set_text(&display);
        *self.cwd.borrow_mut() = Some(path);
        *self.message.borrow_mut() = nav.message.unwrap_or_default();
        if nav.focus_list {
            self.ui.list.focus_control();
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
            self.ui.window.start_timer(TIMER_REBUILD, REBUILD_INTERVAL);
        }
    }

    /// Re-sorts and re-filters the view and refreshes the list, keeping the
    /// selection on the same entries (or applying a pending restore target).
    fn rebuild(&self) {
        if self.timer_armed.replace(false) {
            self.ui.window.stop_timer(TIMER_REBUILD);
        }
        let pending_restore = self.restore.borrow().is_some();
        let keep = (!pending_restore).then(|| self.save_selection());

        let count = {
            let mut m = self.model.borrow_mut();
            m.rebuild();
            m.visible.len()
        };
        self.ui.list.set_count(count);

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
            self.ui.list.select(0);
            self.ui.list.focus(0);
        }

        self.ui.list.redraw_now();
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
        let i = self.ui.list.focused()?;
        self.model.borrow().entry(i).map(|e| e.name.clone())
    }

    fn save_selection(&self) -> Selection {
        let selected = self.ui.list.selected();
        let focused = self.ui.list.focused();
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
        self.ui.list.clear_selection();
        for i in indices {
            self.ui.list.select(i);
        }
        if let Some(i) = focus {
            self.ui.list.focus(i);
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
        let text = self.ui.search.text();
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
            self.ui.list.select_only(i);
        }
        self.count_matches();
        self.ui.list.set_highlighting(true);
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
        let from = self.ui.list.focused();
        let next = self
            .model
            .borrow()
            .next_match(&self.search.borrow(), from, forward);
        if let Some(i) = next {
            self.ui.list.select_only(i);
            self.update_status();
        }
    }

    /// Hides the search input and its highlights; the selection stays.
    fn close_search(&self) {
        // Moving the focus away below reports the input's focus loss, which
        // lands here again; the flag makes that second call return at once.
        if !self.search_open.replace(false) {
            return;
        }
        *self.search.borrow_mut() = Query::default();
        if self.ui.search.has_focus() {
            self.ui.list.focus_control();
        }
        self.ui.search.set_text("");
        self.ui.search.hide();
        self.ui.list.set_highlighting(false);
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
            let selected = self.ui.list.selected_count();
            match (query.is_empty(), self.search_matches.get(), selected) {
                (false, 0, _) => format!("Search: {query}  no matches"),
                (false, 1, _) => format!("Search: {query}  1 match"),
                (false, m, _) => format!("Search: {query}  {m} matches"),
                (true, _, 0) => format!("{n} items"),
                (true, _, s) => format!("{n} items, {s} selected"),
            }
        };
        self.ui.status.set_part(0, &counts);
        let msg = match self.hint.borrow().as_ref() {
            Some(h) => h.clone(),
            None => self.message.borrow().clone(),
        };
        self.ui.status.set_part(1, &msg);
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
        let owner = self.ui.window.token();
        std::thread::spawn(move || {
            if let Err(e) = process::shell_open(owner, &file, &dir) {
                let msg = format!("Cannot open {}: {e}", file.display());
                waker.send(Event::Message(msg));
            }
        });
    }
}

/// Writes the text of one list cell.
fn write_cell(e: &Entry, column: usize, out: &mut CellBuf<'_>) {
    match column {
        0 => {
            out.push_units(&e.name);
            if e.is_dir() {
                out.push_unit(u16::from(b'\\'));
            }
        }
        1 if !e.is_dir() => out.push_str(&entry::format_size(e.size)),
        2 => {
            if let Some(t) = lwfm_win::text::to_local(e.mtime) {
                out.push_str(&entry::format_time(&t));
            }
        }
        _ => {}
    }
}

fn os_name(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

impl Handler for App {
    fn on_event(&self, event: WindowEvent) {
        match event {
            WindowEvent::Resized | WindowEvent::SettingsChanged => self.layout(),
            WindowEvent::DpiChanged { old, new } => {
                trace::log(|| format!("dpi changed {} -> {}", old.0, new.0));
                self.layout();
            }
            WindowEvent::Timer(TIMER_REBUILD) => self.rebuild(),
            WindowEvent::Wake => self.drain_events(),
            WindowEvent::Timer(_) | WindowEvent::Destroyed => {}
        }
    }

    fn on_key(&self, key: KeyEvent) -> bool {
        self.translate_key(key)
    }

    fn list_source(&self, _id: ListId) -> &dyn ListSource {
        self
    }

    fn on_edit(&self, id: EditId, event: EditEvent) {
        if id != SEARCH {
            return;
        }
        match event {
            EditEvent::Changed => self.on_search_changed(),
            EditEvent::LostFocus => self.close_search(),
        }
    }
}

/// The list reads the model while it paints, which can happen inside any
/// call into the list; hence `try_borrow`: a cell that cannot be read is
/// left empty instead of panicking in the window procedure.
impl ListSource for App {
    fn cell(&self, row: usize, column: usize, out: &mut CellBuf<'_>) {
        if let Ok(model) = self.model.try_borrow()
            && let Some(e) = model.entry(row)
        {
            write_cell(e, column, out);
        }
    }

    /// Directory and hidden-item text colors from the config.
    fn row_color(&self, row: usize) -> Option<Color> {
        let colors = self.config.try_borrow().ok()?.settings.colors;
        let model = self.model.try_borrow().ok()?;
        let e = model.entry(row)?;
        if e.is_hidden() {
            Some(colors.hidden)
        } else if e.is_dir() {
            Some(colors.dir)
        } else {
            None
        }
    }

    fn highlight(&self, row: usize) -> Option<Range<usize>> {
        let model = self.model.try_borrow().ok()?;
        let query = self.search.try_borrow().ok()?;
        query.find(&model.entry(row)?.name)
    }

    fn on_activate(&self, _row: usize) {
        self.run_str("open");
    }

    fn on_column_click(&self, column: usize) {
        let Some(key) = SortKey::from_column(column) else {
            return;
        };
        let cur = self.model.borrow().sort;
        let reverse = cur.key == key && !cur.reverse;
        let cmd = format!(
            "sort {}{}",
            key.name(),
            if reverse { " --reverse" } else { "" }
        );
        self.run_str(&cmd);
    }

    fn on_selection_changed(&self) {
        self.update_status();
    }
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
        nav.select = cwd.file_name().map(|n| n.encode_wide().collect());
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
        let Some(i) = self.ui.list.focused() else {
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
        self.ui.path.focus_control();
        self.ui.path.select_all();
        Ok(())
    }

    fn focus_list(&self) -> CmdResult {
        self.ui.list.focus_control();
        Ok(())
    }

    fn path_submit(&self) -> CmdResult {
        let text = self.ui.path.text();
        if let Some(command) = text.trim_start().strip_prefix(':') {
            if command.trim().is_empty() {
                return self.path_cancel();
            }
            registry::run_str(self, command)?;
            // Success: show the directory again (unless the command moved focus there).
            if self.ui.path.has_focus() {
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
        self.ui.path.set_text(&cwd.unwrap_or_default());
        self.focus_list()
    }

    fn quit(&self) -> CmdResult {
        self.ui.window.close();
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
        self.search_open.set(true);
        self.ui.search.show_on_top();
        self.ui.search.focus_control();
        self.ui.search.set_text(text);
        self.ui.search.set_caret(text.encode_utf16().count());
        // Setting the text is not reported back, so run the jump here. Empty
        // text closes the input again.
        self.on_search_changed();
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
        self.ui.path.focus_control();
        self.ui.path.set_text(":");
        self.ui.path.set_caret(1);
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
