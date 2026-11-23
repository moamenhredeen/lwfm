//! Listing model: all entries of a directory plus the sorted, filtered view
//! the list displays. Re-sorting and hidden toggling only rebuild the view.

use std::cmp::Ordering;
use std::collections::HashSet;

use windows::Win32::Globalization::{
    CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringEx, CompareStringOrdinal,
    NORM_IGNORECASE, SORT_DIGITSASNUMBERS,
};
use windows::core::PCWSTR;

use super::entry::Entry;

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

    pub fn from_column(c: i32) -> Option<SortKey> {
        [SortKey::Name, SortKey::Size, SortKey::Mtime]
            .get(usize::try_from(c).ok()?)
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
    let r = unsafe {
        CompareStringEx(
            PCWSTR::null(),
            NORM_IGNORECASE | SORT_DIGITSASNUMBERS,
            a,
            b,
            None,
            None,
            None,
        )
    };
    match r {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_GREATER_THAN => Ordering::Greater,
        CSTR_EQUAL => a.cmp(b),
        _ => a.cmp(b),
    }
}

/// Case-insensitive prefix test, used for type-to-find.
pub fn has_prefix_ignore_case(name: &[u16], prefix: &[u16]) -> bool {
    name.len() >= prefix.len()
        && unsafe { CompareStringOrdinal(&name[..prefix.len()], prefix, true) } == CSTR_EQUAL
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
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN};

    fn w(s: &str) -> Box<[u16]> {
        s.encode_utf16().collect()
    }

    fn file(name: &str, size: u64, mtime: i64) -> Entry {
        Entry {
            name: w(name),
            attrs: 0,
            size,
            mtime,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: w(name),
            attrs: FILE_ATTRIBUTE_DIRECTORY.0,
            size: 0,
            mtime: 0,
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
            attrs: FILE_ATTRIBUTE_HIDDEN.0,
            size: 0,
            mtime: 0,
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

    #[test]
    fn prefix_match() {
        assert!(has_prefix_ignore_case(&w("Documents"), &w("doc")));
        assert!(!has_prefix_ignore_case(&w("Do"), &w("doc")));
        assert!(!has_prefix_ignore_case(&w("Music"), &w("doc")));
    }
}
