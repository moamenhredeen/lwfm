//! Navigation history: back/forward stacks of locations, each remembering
//! the entry that was focused when the location was left.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub dir: PathBuf,
    pub selected: Option<Box<[u16]>>,
}

#[derive(Default)]
pub struct History {
    back: Vec<Location>,
    forward: Vec<Location>,
}

impl History {
    /// Records leaving `current` for a new location; clears forward history.
    pub fn visit(&mut self, current: Location) {
        self.back.push(current);
        self.forward.clear();
    }

    pub fn back_target(&self) -> Option<&Location> {
        self.back.last()
    }

    pub fn forward_target(&self) -> Option<&Location> {
        self.forward.last()
    }

    /// Applies a completed `back`: `current` becomes the forward target.
    pub fn went_back(&mut self, current: Location) {
        if self.back.pop().is_some() {
            self.forward.push(current);
        }
    }

    /// Applies a completed `forward`: `current` becomes the back target.
    pub fn went_forward(&mut self, current: Location) {
        if self.forward.pop().is_some() {
            self.back.push(current);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(dir: &str, sel: Option<&str>) -> Location {
        Location {
            dir: dir.into(),
            selected: sel.map(|s| s.encode_utf16().collect()),
        }
    }

    #[test]
    fn back_and_forward() {
        let mut h = History::default();
        h.visit(loc("A", None)); // A -> B
        assert_eq!(h.back_target().unwrap().dir, PathBuf::from("A"));
        h.went_back(loc("B", None));
        assert_eq!(h.forward_target().unwrap().dir, PathBuf::from("B"));
        assert!(h.back_target().is_none());
        h.went_forward(loc("A", None));
        assert_eq!(h.back_target().unwrap().dir, PathBuf::from("A"));
        assert!(h.forward_target().is_none());
    }

    #[test]
    fn new_navigation_clears_forward() {
        let mut h = History::default();
        h.visit(loc("A", None)); // A -> B
        h.went_back(loc("B", None)); // back to A
        h.visit(loc("A", None)); // A -> C
        assert!(h.forward_target().is_none());
    }

    #[test]
    fn selection_is_remembered() {
        let mut h = History::default();
        h.visit(loc("A", Some("x.txt")));
        assert_eq!(h.back_target().unwrap(), &loc("A", Some("x.txt")));
    }
}
