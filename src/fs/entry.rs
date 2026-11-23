//! A directory entry and its display formatting.

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// UTF-16 name as returned by the file system, without terminator.
    pub name: Box<[u16]>,
    pub attrs: u32,
    pub size: u64,
    /// Last write time as a FILETIME value (100 ns ticks since 1601, UTC).
    pub mtime: i64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }

    pub fn is_hidden(&self) -> bool {
        self.attrs & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0
    }
}

/// Human-readable size in binary units: `512 B`, `1.5 KB`, `12 KB`, `3.4 MB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
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
pub fn format_time(t: &SYSTEMTIME) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute
    )
}

/// Converts a FILETIME value (UTC) to local time.
pub fn to_local(filetime: i64) -> Option<SYSTEMTIME> {
    let ft = FILETIME {
        dwLowDateTime: filetime as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
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
        let t = SYSTEMTIME {
            wYear: 2026,
            wMonth: 11,
            wDay: 23,
            wHour: 14,
            wMinute: 5,
            ..Default::default()
        };
        assert_eq!(format_time(&t), "2026-11-23 14:05");
    }

    #[test]
    fn attributes() {
        let e = |attrs| Entry {
            name: Box::new([]),
            attrs,
            size: 0,
            mtime: 0,
        };
        assert!(e(FILE_ATTRIBUTE_DIRECTORY.0).is_dir());
        assert!(e(FILE_ATTRIBUTE_HIDDEN.0).is_hidden());
        assert!(e(FILE_ATTRIBUTE_SYSTEM.0).is_hidden());
        assert!(!e(0).is_hidden());
    }
}
