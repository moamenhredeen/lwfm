//! The status bar: counts or search state on the left, the latest message
//! (or a key-sequence hint) on the right.

use super::App;

impl App {
    pub(super) fn set_message(&self, s: String) {
        *self.message.borrow_mut() = s;
        self.update_status();
    }

    pub(super) fn update_status(&self) {
        let (loading, received, visible) = self.listing(|l| {
            (
                l.load.is_loading(),
                l.load.received(),
                l.model.visible.len(),
            )
        });
        let counts = if loading {
            format!("Loading... {received} items")
        } else {
            let (query, matches) = self.search(|s| (s.query.text().to_string(), s.matches));
            let selected = self.ui.list.selected_count();
            match (query.is_empty(), matches, selected) {
                (false, 0, _) => format!("Search: {query}  no matches"),
                (false, 1, _) => format!("Search: {query}  1 match"),
                (false, m, _) => format!("Search: {query}  {m} matches"),
                (true, _, 0) => format!("{visible} items"),
                (true, _, s) => format!("{visible} items, {s} selected"),
            }
        };
        self.ui.status.set_part(0, &counts);
        let hint = self.keys.borrow().hint.clone();
        let message = hint.unwrap_or_else(|| self.message.borrow().clone());
        self.ui.status.set_part(1, &message);
    }
}
