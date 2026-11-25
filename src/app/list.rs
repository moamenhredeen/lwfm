//! The list: what it shows, keeping it in step with the model, and the
//! selection.

use std::ffi::OsString;
use std::ops::Range;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Instant;

use lwfm_win::Color;
use lwfm_win::controls::{CellBuf, ListSource};

use super::nav::{Listing, Selection};
use super::{App, REBUILD_INTERVAL, TIMER_REBUILD};
use crate::fs::entry::{self, Entry};
use crate::fs::model::SortKey;
use crate::lua::api::FmState;
use crate::trace;

/// Throttles re-sorting while a directory is loading.
#[derive(Clone, Copy, Default)]
pub(super) struct RebuildTimer {
    /// When the list was last rebuilt for the current directory.
    pub(super) last: Option<Instant>,
    /// Whether the timer for the next rebuild is running.
    armed: bool,
}

/// What the timing output still has to report.
#[derive(Clone, Copy, Default)]
pub(super) struct TraceState {
    startup_traced: bool,
    /// When the enumeration finished, until the listing is painted.
    pub(super) done_at: Option<Instant>,
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

pub(super) fn os_name(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

impl App {
    /// Reads the listing from a list callback. The list calls back while its
    /// own methods run, so a mutable borrow at this point means the rule in
    /// the module documentation was broken: that fails loudly in debug
    /// builds and leaves the cell empty in release builds, instead of
    /// panicking inside the window procedure.
    fn paint_listing<R>(&self, f: impl FnOnce(&Listing) -> Option<R>) -> Option<R> {
        let listing = self.listing.try_borrow();
        debug_assert!(
            listing.is_ok(),
            "the listing is mutably borrowed across a call into the list"
        );
        let listing = listing.ok()?;
        f(&listing)
    }

    pub(super) fn schedule_rebuild(&self) {
        let mut timer = self.rebuild_timer.get();
        if timer.last.is_none_or(|t| t.elapsed() >= REBUILD_INTERVAL) {
            self.rebuild();
        } else if !timer.armed {
            timer.armed = true;
            self.rebuild_timer.set(timer);
            self.ui.window.start_timer(TIMER_REBUILD, REBUILD_INTERVAL);
        }
    }

    /// Re-sorts and re-filters the view and refreshes the list, keeping the
    /// selection on the same entries (or applying a pending restore target).
    pub(super) fn rebuild(&self) {
        let mut timer = self.rebuild_timer.get();
        if timer.armed {
            self.ui.window.stop_timer(TIMER_REBUILD);
        }
        timer.armed = false;
        self.rebuild_timer.set(timer);

        let pending_restore = self.listing(|l| l.restore.is_some());
        let keep = (!pending_restore).then(|| self.save_selection());

        let count = self.listing_mut(|l| {
            l.model.rebuild();
            l.model.visible.len()
        });
        self.ui.list.set_count(count);

        let found = match keep {
            Some(sel) => self.apply_selection(&sel),
            None => {
                let target = self.listing_mut(|l| l.restore.take()).unwrap_or_default();
                let found = self.apply_selection(&target);
                if !found {
                    // Not listed yet: keep waiting while loading.
                    self.listing_mut(|l| {
                        if l.load.is_loading() {
                            l.restore = Some(target);
                        }
                    });
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
        let mut timer = self.rebuild_timer.get();
        timer.last = Some(Instant::now());
        self.rebuild_timer.set(timer);
        if self.search(|s| !s.query.is_empty()) {
            self.count_matches();
        }
        self.update_status();
        self.trace_paint(count);
    }

    fn trace_paint(&self, count: usize) {
        if !trace::enabled() {
            return;
        }
        let mut state = self.trace.get();
        if !std::mem::replace(&mut state.startup_traced, true) {
            trace::log(|| {
                format!(
                    "startup -> first painted listing: {} ({count} items)",
                    trace::ms(trace::since_process_start())
                )
            });
        }
        if let Some(t) = state.done_at.take() {
            trace::log(|| {
                format!(
                    "enumeration complete -> listing painted: {} ({count} items)",
                    trace::ms(t.elapsed())
                )
            });
        }
        self.trace.set(state);
    }

    // ---- selection -------------------------------------------------------

    pub(super) fn focused_name(&self) -> Option<Box<[u16]>> {
        let i = self.ui.list.focused()?;
        self.listing(|l| l.model.entry(i).map(|e| e.name.clone()))
    }

    pub(super) fn save_selection(&self) -> Selection {
        let selected = self.ui.list.selected();
        let focused = self.ui.list.focused();
        self.listing(|l| Selection {
            focused: focused
                .and_then(|i| l.model.entry(i))
                .map(|e| e.name.clone()),
            selected: selected
                .into_iter()
                .filter_map(|i| l.model.entry(i))
                .map(|e| e.name.clone())
                .collect(),
        })
    }

    /// Applies a selection by name. Returns whether the focused entry was found.
    fn apply_selection(&self, sel: &Selection) -> bool {
        let (indices, focus) = self.listing(|l| {
            (
                l.model.find_all(&sel.selected),
                sel.focused.as_deref().and_then(|n| l.model.find(n)),
            )
        });
        self.ui.list.clear_selection();
        for i in indices {
            self.ui.list.select(i);
        }
        if let Some(i) = focus {
            self.ui.list.focus(i);
        }
        focus.is_some()
    }

    // ---- what Lua and shell commands see ---------------------------------

    fn entry_path(&self, i: usize) -> Option<PathBuf> {
        self.listing(|l| {
            let name = os_name(&l.model.entry(i)?.name);
            Some(l.cwd.as_ref()?.join(name))
        })
    }

    pub(super) fn focused_path(&self) -> Option<PathBuf> {
        self.entry_path(self.ui.list.focused()?)
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        self.ui
            .list
            .selected()
            .into_iter()
            .filter_map(|i| self.entry_path(i))
            .collect()
    }

    pub(super) fn fm_state(&self) -> FmState {
        let text = |p: PathBuf| p.display().to_string();
        FmState {
            dir: text(self.base_dir()),
            file: self.focused_path().map(text),
            selection: self.selected_paths().into_iter().map(text).collect(),
        }
    }
}

impl ListSource for App {
    fn cell(&self, row: usize, column: usize, out: &mut CellBuf<'_>) {
        self.paint_listing(|l| {
            write_cell(l.model.entry(row)?, column, out);
            Some(())
        });
    }

    /// Directory and hidden-item text colors from the config.
    fn row_color(&self, row: usize) -> Option<Color> {
        let colors = self.config.try_borrow().ok()?.loaded.settings.colors;
        self.paint_listing(|l| {
            let e = l.model.entry(row)?;
            if e.is_hidden() {
                Some(colors.hidden)
            } else if e.is_dir() {
                Some(colors.dir)
            } else {
                None
            }
        })
    }

    fn highlight(&self, row: usize) -> Option<Range<usize>> {
        let search = self.search.try_borrow().ok()?;
        self.paint_listing(|l| search.query.find(&l.model.entry(row)?.name))
    }

    fn on_activate(&self, _row: usize) {
        self.run_str("open");
    }

    fn on_column_click(&self, column: usize) {
        let Some(key) = SortKey::from_column(column) else {
            return;
        };
        let current = self.listing(|l| l.model.sort);
        let reverse = current.key == key && !current.reverse;
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
