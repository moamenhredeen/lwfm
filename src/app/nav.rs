//! Navigation: starting an enumeration and turning its results into the
//! current listing.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::{App, Event};
use crate::fs::enumerate::{self, FsMsg};
use crate::fs::model::{Model, Sort};
use crate::fs::path;
use crate::lua::api::Event as LuaEvent;
use crate::nav::{History, Location};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NavKind {
    Start,
    New,
    Back,
    Forward,
    Refresh,
}

/// A request to list a directory and what to do once it is listed.
#[derive(Clone, Debug)]
pub(super) struct Nav {
    pub(super) kind: NavKind,
    /// Entry to select once listed (the directory just left, or history).
    pub(super) select: Option<Box<[u16]>>,
    /// Return focus to the list when the navigation succeeds (path bar).
    pub(super) focus_list: bool,
    /// On failure, open the home directory instead (start argument).
    pub(super) fallback_home: bool,
    /// Status message to show when the navigation commits.
    pub(super) message: Option<String>,
}

impl Nav {
    pub(super) fn new(kind: NavKind) -> Nav {
        Nav {
            kind,
            select: None,
            focus_list: false,
            fallback_home: false,
            message: None,
        }
    }
}

/// Where the latest navigation stands.
#[derive(Debug)]
pub(super) enum LoadState {
    /// Nothing is being enumerated.
    Idle,
    /// Requested; nothing has arrived yet, so the previous listing is still shown.
    Pending { path: PathBuf, nav: Nav },
    /// The first results arrived and replaced the previous listing; more may follow.
    Streaming { kind: NavKind, received: usize },
}

impl LoadState {
    pub(super) fn is_loading(&self) -> bool {
        !matches!(self, LoadState::Idle)
    }

    /// Entries received so far by the navigation in progress.
    pub(super) fn received(&self) -> usize {
        match self {
            LoadState::Streaming { received, .. } => *received,
            LoadState::Idle | LoadState::Pending { .. } => 0,
        }
    }
}

/// A selection by entry name, which survives re-sorting and re-listing.
#[derive(Default, Debug)]
pub(super) struct Selection {
    pub(super) focused: Option<Box<[u16]>>,
    pub(super) selected: HashSet<Box<[u16]>>,
}

/// The current directory and everything about its listing.
///
/// The list reads this while it paints, which can happen inside any call
/// into the list. A mutable borrow must therefore end before the window or
/// a control is called.
pub(super) struct Listing {
    pub(super) cwd: Option<PathBuf>,
    pub(super) model: Model,
    pub(super) history: History,
    pub(super) load: LoadState,
    /// Selection to apply once its entries show up in the listing.
    pub(super) restore: Option<Selection>,
    /// Whether the `startup` event was fired (after the first listing).
    startup_fired: bool,
}

impl Listing {
    pub(super) fn new(show_hidden: bool, sort: Sort) -> Listing {
        Listing {
            cwd: None,
            model: Model {
                show_hidden,
                sort,
                ..Model::default()
            },
            history: History::default(),
            load: LoadState::Idle,
            restore: None,
            startup_fired: false,
        }
    }
}

fn join_messages(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} | {b}")),
        (a, b) => a.or(b),
    }
}

impl App {
    pub(super) fn cwd(&self) -> Option<PathBuf> {
        self.listing(|l| l.cwd.clone())
    }

    /// The current directory, or home before the first listing.
    pub(super) fn base_dir(&self) -> PathBuf {
        self.cwd().unwrap_or_else(|| self.home.clone())
    }

    pub(super) fn cd_with(&self, input: &str, focus_list: bool) {
        let target = path::resolve(input, &self.base_dir(), &self.home);
        let mut nav = Nav::new(NavKind::New);
        nav.focus_list = focus_list;
        self.navigate(target, nav);
    }

    /// Starts listing `path`. The current listing stays until results arrive.
    pub(super) fn navigate(&self, path: PathBuf, nav: Nav) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.listing_mut(|l| {
            l.load = LoadState::Pending {
                path: path.clone(),
                nav,
            };
        });
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

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    pub(super) fn on_fs(&self, msg: FsMsg) {
        match msg {
            FsMsg::Batch {
                generation,
                entries,
            } if self.is_current(generation) => {
                self.commit();
                self.listing_mut(|l| {
                    if let LoadState::Streaming { received, .. } = &mut l.load {
                        *received += entries.len();
                    }
                    l.model.extend(entries);
                });
                self.schedule_rebuild();
            }
            FsMsg::Done { generation } if self.is_current(generation) => {
                self.commit();
                let finished =
                    self.listing_mut(|l| std::mem::replace(&mut l.load, LoadState::Idle));
                let mut trace = self.trace.get();
                trace.done_at = Some(Instant::now());
                self.trace.set(trace);
                self.rebuild();
                self.listing_mut(|l| l.restore = None);
                let entered = matches!(
                    finished,
                    LoadState::Streaming { kind, .. } if kind != NavKind::Refresh
                );
                if entered {
                    let first =
                        self.listing_mut(|l| !std::mem::replace(&mut l.startup_fired, true));
                    if first {
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
                let failed = self.listing_mut(|l| std::mem::replace(&mut l.load, LoadState::Idle));
                match failed {
                    LoadState::Pending { nav, .. } if nav.fallback_home => {
                        let mut home = Nav::new(NavKind::Start);
                        home.message = join_messages(Some(message), nav.message);
                        self.navigate(self.home.clone(), home);
                    }
                    LoadState::Streaming { .. } => {
                        // Failed part-way: show what arrived.
                        self.set_message(message);
                        self.rebuild();
                    }
                    LoadState::Pending { .. } | LoadState::Idle => self.set_message(message),
                }
            }
            _ => {}
        }
    }

    /// Makes the pending navigation current, on its first batch (or on
    /// completion for an empty directory). Until then the old listing stays.
    fn commit(&self) {
        let pending = self.listing_mut(|l| match std::mem::replace(&mut l.load, LoadState::Idle) {
            LoadState::Pending { path, nav } => {
                l.load = LoadState::Streaming {
                    kind: nav.kind,
                    received: 0,
                };
                Some((path, nav))
            }
            other => {
                l.load = other;
                None
            }
        });
        let Some((path, nav)) = pending else { return };

        if let Some(dir) = self.cwd() {
            let current = Location {
                dir,
                selected: self.focused_name(),
            };
            self.listing_mut(|l| match nav.kind {
                NavKind::New => l.history.visit(current),
                NavKind::Back => l.history.went_back(current),
                NavKind::Forward => l.history.went_forward(current),
                NavKind::Refresh | NavKind::Start => {}
            });
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
        self.listing_mut(|l| {
            l.restore = Some(restore);
            l.model.clear();
        });
        // A search belongs to one directory; refresh keeps it.
        if nav.kind != NavKind::Refresh {
            self.close_search();
        }
        // The new listing is shown at once, whenever the old one was last rebuilt.
        let mut timer = self.rebuild_timer.get();
        timer.last = None;
        self.rebuild_timer.set(timer);

        let display = path.display().to_string();
        self.ui.window.set_title(&format!("{display} - lwfm"));
        self.ui.path.set_text(&display);
        self.listing_mut(|l| l.cwd = Some(path));
        *self.message.borrow_mut() = nav.message.unwrap_or_default();
        if nav.focus_list {
            self.ui.list.focus_control();
        }
    }
}
