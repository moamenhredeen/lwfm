//! Find-as-you-type in the list.

use std::time::Instant;

use super::App;
use crate::search::Query;
use crate::trace;

#[derive(Default)]
pub(super) struct SearchState {
    /// Whether the search input is open.
    open: bool,
    /// The input's text; empty while the input is closed.
    pub(super) query: Query,
    /// How many visible entries match the query.
    pub(super) matches: usize,
}

impl App {
    pub(super) fn count_matches(&self) {
        let n = self.listing(|l| self.search(|s| l.model.count_matches(&s.query)));
        self.search_mut(|s| s.matches = n);
    }

    /// Opens the search input with `text`, as if it had been typed.
    pub(super) fn open_search(&self, text: &str) {
        self.search_mut(|s| s.open = true);
        self.ui.search.show_on_top();
        self.ui.search.focus_control();
        self.ui.search.set_text(text);
        self.ui.search.set_caret(text.encode_utf16().count());
        // Setting the text is not reported back, so run the jump here. Empty
        // text closes the input again.
        self.on_search_changed();
    }

    /// The search input's text changed: select the first match.
    pub(super) fn on_search_changed(&self) {
        let text = self.ui.search.text();
        if text.is_empty() {
            self.close_search();
            return;
        }
        let started = Instant::now();
        let query = Query::new(&text);
        let first = self.listing(|l| l.model.next_match(&query, None, true));
        self.search_mut(|s| s.query = query);
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
                self.search(|s| s.matches)
            )
        });
    }

    /// Moves the selection to the next or previous match, wrapping around.
    pub(super) fn step_match(&self, forward: bool) {
        if self.search(|s| s.query.is_empty()) {
            return;
        }
        let from = self.ui.list.focused();
        let next = self.listing(|l| self.search(|s| l.model.next_match(&s.query, from, forward)));
        if let Some(i) = next {
            self.ui.list.select_only(i);
            self.update_status();
        }
    }

    /// Hides the search input and its highlights; the selection stays.
    pub(super) fn close_search(&self) {
        // Moving the focus away below reports the input's focus loss, which
        // lands here again; by then the search is marked closed and that
        // second call returns at once.
        let was_open = self.search_mut(|s| {
            let was_open = s.open;
            s.open = false;
            s.query = Query::default();
            was_open
        });
        if !was_open {
            return;
        }
        if self.ui.search.has_focus() {
            self.ui.list.focus_control();
        }
        self.ui.search.set_text("");
        self.ui.search.hide();
        self.ui.list.set_highlighting(false);
        self.update_status();
    }
}
