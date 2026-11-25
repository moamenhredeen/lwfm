//! A directory entry and its display formatting.

use lwfm_win::fs::RawEntry;
use lwfm_win::text::LocalTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// UTF-16 name as returned by the file system, without terminator.
    pub name: Box<[u16]>,
    pub size: u64,
    /// Last write time as a FILETIME value (100 ns ticks since 1601, UTC).
    pub mtime: i64,
    pub dir: bool,
    /// Hidden or system.
    pub hidden: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.dir
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
}

impl From<RawEntry<'_>> for Entry {
    fn from(raw: RawEntry<'_>) -> Entry {
        Entry {
            name: raw.name.into(),
            size: raw.size,
            mtime: raw.mtime,
            dir: raw.is_dir,
            hidden: raw.is_hidden,
        }
    }
}

/// Human-readable size in binary units: `512 B`, `1.5 KB`, `12 KB`, `3.4 MB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Rounded to two significant digits below, so the precision of f64 is ample.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let num = if value < 10.0 {
        let s = format!("{value:.1}");
        s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
    } else {
        format!("{value:.0}")
    };
    format!("{num} {}", UNITS[unit])
}

/// `YYYY-MM-DD HH:MM`
pub fn format_time(t: &LocalTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(12288), "12 KB");
        assert_eq!(format_size(3_565_158), "3.4 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5 GB");
    }

    #[test]
    fn time_format() {
        let t = LocalTime {
            year: 2026,
            month: 11,
            day: 23,
            hour: 14,
            minute: 5,
            second: 0,
        };
        assert_eq!(format_time(&t), "2026-11-23 14:05");
    }

    #[test]
    fn from_raw_entry() {
        let name: Vec<u16> = "sub".encode_utf16().collect();
        let e = Entry::from(RawEntry {
            name: &name,
            size: 7,
            mtime: 9,
            is_dir: true,
            is_hidden: false,
        });
        assert_eq!(&*e.name, &name[..]);
        assert_eq!((e.size, e.mtime), (7, 9));
        assert!(e.is_dir() && !e.is_hidden());
    }
}
