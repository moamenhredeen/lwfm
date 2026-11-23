//! Find-as-you-type matching: case-insensitive substring over file names,
//! with the matched range in UTF-16 units for highlighting.

use std::ops::Range;

/// A search text, stored lowercased for matching.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Query {
    text: String,
    lower: Vec<char>,
}

impl Query {
    pub fn new(text: &str) -> Query {
        Query {
            text: text.to_string(),
            lower: text.chars().flat_map(char::to_lowercase).collect(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The first match in the UTF-16 name, as a range of UTF-16 units of the
    /// original name (exact even when lowercasing changes the length).
    pub fn find(&self, name: &[u16]) -> Option<Range<usize>> {
        if self.lower.is_empty() {
            return None;
        }
        // For every lowercased char: the UTF-16 range of the char it came from.
        let mut lower = Vec::with_capacity(name.len());
        let mut spans = Vec::with_capacity(name.len());
        let mut offset = 0;
        for c in char::decode_utf16(name.iter().copied()) {
            let (c, len) = match c {
                Ok(c) => (c, c.len_utf16()),
                Err(_) => ('\u{FFFD}', 1),
            };
            for l in c.to_lowercase() {
                lower.push(l);
                spans.push(offset..offset + len);
            }
            offset += len;
        }
        let n = self.lower.len();
        let i = lower.windows(n).position(|w| w == self.lower.as_slice())?;
        Some(spans[i].start..spans[i + n - 1].end)
    }

    pub fn matches(&self, name: &[u16]) -> bool {
        self.find(name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn substring_ignoring_case() {
        let q = Query::new("rep");
        assert_eq!(q.find(&w("Reports")), Some(0..3));
        assert_eq!(q.find(&w("prep.txt")), Some(1..4));
        assert_eq!(q.find(&w("report.pdf")), Some(0..3));
        assert_eq!(q.find(&w("notes.md")), None);
        assert_eq!(Query::new("PREP").find(&w("prep.txt")), Some(0..4));
        assert!(Query::new("ÄRGER").matches(&w("das ärgernis.txt")));
    }

    #[test]
    fn ranges_survive_length_changing_lowercase() {
        // 'İ' lowercases to two chars ("i̇"); the range must still cover the
        // original characters of the name.
        let name = w("aİbc");
        assert_eq!(Query::new("bc").find(&name), Some(2..4));
        assert_eq!(Query::new("i̇b").find(&name), Some(1..3));
        // Characters outside the BMP take two UTF-16 units.
        let emoji = w("x😀report");
        assert_eq!(Query::new("rep").find(&emoji), Some(3..6));
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert_eq!(Query::default().find(&w("anything")), None);
        assert!(Query::default().is_empty());
    }
}
