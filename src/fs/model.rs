//! Listing model: all entries of a directory plus the sorted, filtered view
//! the list displays. Re-sorting and hidden toggling only rebuild the view.

use std::cmp::Ordering;
use std::collections::HashSet;

use super::entry::Entry;
use crate::search::Query;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Mtime,
}

impl SortKey {
    pub const NAMES: &'static [&'static str] = &["name", "size", "mtime"];

    pub fn from_name(s: &str) -> Option<SortKey> {
        match s {
            "name" => Some(SortKey::Name),
            "size" => Some(SortKey::Size),
            "mtime" => Some(SortKey::Mtime),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Mtime => "mtime",
        }
    }

    pub fn from_column(column: usize) -> Option<SortKey> {
        [SortKey::Name, SortKey::Size, SortKey::Mtime]
            .get(column)
            .copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub reverse: bool,
}

impl Default for Sort {
    fn default() -> Sort {
        Sort {
            key: SortKey::Name,
            reverse: false,
        }
    }
}

#[derive(Default)]
pub struct Model {
    pub entries: Vec<Entry>,
    /// Indices into `entries`, filtered and sorted.
    pub visible: Vec<u32>,
    pub show_hidden: bool,
    pub sort: Sort,
}

/// Natural, case-insensitive, locale-aware name order (`file2` < `file10`).
pub fn compare_names(a: &[u16], b: &[u16]) -> Ordering {
    lwfm_win::text::compare_natural(a, b)
}

impl Model {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.visible.clear();
    }

    pub fn extend(&mut self, batch: Vec<Entry>) {
        self.entries.extend(batch);
    }

    pub fn entry(&self, visible_index: usize) -> Option<&Entry> {
        self.visible
            .get(visible_index)
            .map(|&i| &self.entries[i as usize])
    }

    /// Rebuilds the visible view: filter hidden items, directories first,
    /// then the sort key (ties broken by name).
    pub fn rebuild(&mut self) {
        let entries = &self.entries;
        let show_hidden = self.show_hidden;
        let Sort { key, reverse } = self.sort;
        self.visible.clear();
        self.visible.extend(
            (0..entries.len() as u32).filter(|&i| show_hidden || !entries[i as usize].is_hidden()),
        );
        self.visible.sort_by(|&a, &b| {
            let (a, b) = (&entries[a as usize], &entries[b as usize]);
            b.is_dir().cmp(&a.is_dir()).then_with(|| {
                let by_key = match key {
                    SortKey::Name => compare_names(&a.name, &b.name),
                    SortKey::Size => a
                        .size
                        .cmp(&b.size)
                        .then_with(|| compare_names(&a.name, &b.name)),
                    SortKey::Mtime => a
                        .mtime
                        .cmp(&b.mtime)
                        .then_with(|| compare_names(&a.name, &b.name)),
                };
                if reverse { by_key.reverse() } else { by_key }
            })
        });
    }

    /// The next visible entry matching `q`: after `from` (wrapping around,
    /// and finding `from` itself last), or the first from the top when
    /// `from` is `None`. `forward = false` searches backwards.
    pub fn next_match(&self, q: &Query, from: Option<usize>, forward: bool) -> Option<usize> {
        let n = self.visible.len();
        if n == 0 {
            return None;
        }
        let matches = |i: usize| q.matches(&self.entry(i).unwrap().name);
        match from {
            None => (0..n).find(|&i| matches(i)),
            Some(f) => (1..=n)
                .map(|k| {
                    if forward {
                        (f + k) % n
                    } else {
                        (f + n - k % n) % n
                    }
                })
                .find(|&i| matches(i)),
        }
    }

    pub fn count_matches(&self, q: &Query) -> usize {
        (0..self.visible.len())
            .filter(|&i| q.matches(&self.entry(i).unwrap().name))
            .count()
    }

    pub fn find(&self, name: &[u16]) -> Option<usize> {
        self.visible
            .iter()
            .position(|&i| &*self.entries[i as usize].name == name)
    }

    /// Visible indices of all entries whose name is in `names`, in one pass.
    pub fn find_all(&self, names: &HashSet<Box<[u16]>>) -> Vec<usize> {
        if names.is_empty() {
            return Vec::new();
        }
        self.visible
            .iter()
            .enumerate()
            .filter(|(_, i)| names.contains(&self.entries[**i as usize].name))
            .map(|(v, _)| v)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Box<[u16]> {
        s.encode_utf16().collect()
    }

    fn file(name: &str, size: u64, mtime: i64) -> Entry {
        Entry {
            name: w(name),
            size,
            mtime,
            dir: false,
            hidden: false,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: w(name),
            size: 0,
            mtime: 0,
            dir: true,
            hidden: false,
        }
    }

    fn names(m: &Model) -> Vec<String> {
        m.visible
            .iter()
            .map(|&i| String::from_utf16(&m.entries[i as usize].name).unwrap())
            .collect()
    }

    fn model(entries: Vec<Entry>) -> Model {
        let mut m = Model::default();
        m.extend(entries);
        m.rebuild();
        m
    }

    fn search_fixture() -> Model {
        model(vec![
            file("report.pdf", 0, 0),
            file("prep.txt", 0, 0),
            dir("Reports"),
            file("notes.md", 0, 0),
        ])
    }

    #[test]
    fn matches_do_not_filter() {
        let m = search_fixture();
        let q = Query::new("rep");
        assert_eq!(names(&m), ["Reports", "notes.md", "prep.txt", "report.pdf"]);
        assert_eq!(m.next_match(&q, None, true), Some(0));
        assert_eq!(m.count_matches(&q), 3);
        assert_eq!(m.next_match(&Query::new("PREP"), None, true), Some(2));
        assert_eq!(m.next_match(&Query::new("zzz"), None, true), None);
        assert_eq!(m.count_matches(&Query::new("zzz")), 0);
    }

    #[test]
    fn next_and_previous_match_wrap() {
        let m = search_fixture();
        let q = Query::new("rep");
        assert_eq!(m.next_match(&q, Some(0), true), Some(2));
        assert_eq!(m.next_match(&q, Some(2), true), Some(3));
        assert_eq!(m.next_match(&q, Some(3), true), Some(0));
        assert_eq!(m.next_match(&q, Some(0), false), Some(3));
        assert_eq!(m.next_match(&q, Some(2), false), Some(0));
        // A single match finds itself again.
        assert_eq!(m.next_match(&Query::new("notes"), Some(1), true), Some(1));
    }

    #[test]
    fn hidden_entries_are_not_matched() {
        let secret = Entry {
            name: w("secret-report.txt"),
            size: 0,
            mtime: 0,
            dir: false,
            hidden: true,
        };
        let mut m = model(vec![file("report.txt", 0, 0), secret]);
        assert_eq!(m.count_matches(&Query::new("REPORT")), 1);
        m.show_hidden = true;
        m.rebuild();
        assert_eq!(m.count_matches(&Query::new("REPORT")), 2);
    }

    #[test]
    fn natural_case_insensitive_order() {
        let m = model(vec![
            file("file10.txt", 0, 0),
            file("File2.txt", 0, 0),
            file("file1.txt", 0, 0),
        ]);
        assert_eq!(names(&m), ["file1.txt", "File2.txt", "file10.txt"]);
    }

    #[test]
    fn directories_first_for_every_sort() {
        let mut m = model(vec![file("a.txt", 5, 5), dir("z")]);
        for key in [SortKey::Name, SortKey::Size, SortKey::Mtime] {
            for reverse in [false, true] {
                m.sort = Sort { key, reverse };
                m.rebuild();
                assert_eq!(names(&m)[0], "z", "{key:?} reverse={reverse}");
            }
        }
    }

    #[test]
    fn sort_by_size_and_mtime() {
        let mut m = model(vec![file("a", 30, 1), file("b", 10, 3), file("c", 20, 2)]);
        m.sort = Sort {
            key: SortKey::Size,
            reverse: true,
        };
        m.rebuild();
        assert_eq!(names(&m), ["a", "c", "b"]);
        m.sort = Sort {
            key: SortKey::Mtime,
            reverse: false,
        };
        m.rebuild();
        assert_eq!(names(&m), ["a", "c", "b"]);
    }

    #[test]
    fn hidden_toggle_without_reenumeration() {
        let secret = Entry {
            name: w("secret.txt"),
            size: 0,
            mtime: 0,
            dir: false,
            hidden: true,
        };
        let mut m = model(vec![file("a.txt", 0, 0), secret]);
        assert_eq!(names(&m), ["a.txt"]);
        m.show_hidden = true;
        m.rebuild();
        assert_eq!(names(&m), ["a.txt", "secret.txt"]);
        assert_eq!(m.entries.len(), 2);
    }

    #[test]
    fn find_by_name() {
        let m = model(vec![file("b", 0, 0), file("a", 0, 0), file("c", 0, 0)]);
        assert_eq!(m.find(&w("b")), Some(1));
        assert_eq!(m.find(&w("x")), None);
        let set: HashSet<_> = [w("a"), w("c")].into_iter().collect();
        assert_eq!(m.find_all(&set), [0, 2]);
    }
}
