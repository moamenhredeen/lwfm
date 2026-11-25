//! Application state and behaviour.
//!
//! All state lives on the UI thread in a few `RefCell` groups. The window
//! and its controls call back into the application, also while one of their
//! own methods runs (the list asks for cell text while its row count is
//! being set, for example). Therefore a borrow of a state group never spans
//! a call into the window or a control: state is reached through the
//! closure-taking accessors below, and nothing inside such a closure calls
//! out.

mod commands;
mod config;
mod keys;
mod list;
mod nav;
mod search;
mod status;

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use lwfm_win::controls::{
    Column, Edit, EditEvent, EditId, ListId, ListSource, ListView, StatusBar,
};
use lwfm_win::input::KeyEvent;
use lwfm_win::window::{Handler, TimerId, UiWaker, Window, WindowEvent};
use lwfm_win::{Rect, process};

use self::config::ConfigState;
use self::keys::KeySeq;
use self::list::{RebuildTimer, TraceState};
use self::nav::{Listing, Nav, NavKind};
use self::search::SearchState;
use crate::config::discover::Env;
use crate::config::{ConfigError, Loaded};
use crate::fs::enumerate::FsMsg;
use crate::fs::path;
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

/// What worker threads report to the UI thread.
enum Event {
    Fs(FsMsg),
    Message(String),
    /// New content of the watched config file (or a read error).
    ConfigChanged(PathBuf, Result<String, ConfigError>),
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

pub struct App {
    ui: Controls,
    home: PathBuf,
    config_env: Env,

    config: RefCell<ConfigState>,
    keys: RefCell<KeySeq>,
    listing: RefCell<Listing>,
    search: RefCell<SearchState>,
    /// The latest status message; a pending key sequence's hint replaces it.
    message: RefCell<String>,
    rebuild_timer: Cell<RebuildTimer>,
    trace: Cell<TraceState>,

    /// Number of the latest navigation; results of older ones are dropped.
    generation: Arc<AtomicU64>,
    waker: Waker,
    rx: Receiver<Event>,
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
        let listing = Listing::new(config.settings.show_hidden, config.settings.sort);
        let app = App {
            ui,
            home: path::home(),
            config_env: env,
            config: RefCell::new(ConfigState::new(config, config_file.clone())),
            keys: RefCell::new(KeySeq::default()),
            listing: RefCell::new(listing),
            search: RefCell::new(SearchState::default()),
            message: RefCell::new(String::new()),
            rebuild_timer: Cell::new(RebuildTimer::default()),
            trace: Cell::new(TraceState::default()),
            generation: Arc::new(AtomicU64::new(0)),
            waker,
            rx,
        };
        if let Some(file) = config_file {
            app.watch_config(file);
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

    /// Handles everything worker threads have reported since the last wake-up.
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

    /// Opens `file` with its Windows association, off the UI thread.
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

    // ---- state access ----------------------------------------------------
    //
    // The closures must not call into the window or a control (see the
    // module documentation).

    fn config(&self) -> Rc<Loaded> {
        self.config.borrow().loaded.clone()
    }

    fn with_config<R>(&self, f: impl FnOnce(&mut ConfigState) -> R) -> R {
        f(&mut self.config.borrow_mut())
    }

    fn listing<R>(&self, f: impl FnOnce(&Listing) -> R) -> R {
        f(&self.listing.borrow())
    }

    fn listing_mut<R>(&self, f: impl FnOnce(&mut Listing) -> R) -> R {
        f(&mut self.listing.borrow_mut())
    }

    fn search<R>(&self, f: impl FnOnce(&SearchState) -> R) -> R {
        f(&self.search.borrow())
    }

    fn search_mut<R>(&self, f: impl FnOnce(&mut SearchState) -> R) -> R {
        f(&mut self.search.borrow_mut())
    }
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
