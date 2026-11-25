//! Locale-aware text helpers: name ordering and local time.

use std::cmp::Ordering;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Globalization::{
    CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringEx, NORM_IGNORECASE, SORT_DIGITSASNUMBERS,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::core::PCWSTR;

/// Natural, case-insensitive, locale-aware order of two UTF-16 names
/// (`file2` < `file10`). Names the locale considers equal are ordered by
/// their code units, so the order is total.
#[must_use]
pub fn compare_natural(a: &[u16], b: &[u16]) -> Ordering {
    // SAFETY: both strings are passed as slices with their lengths; a null
    // locale name selects the user's default locale.
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
        _ => a.cmp(b),
    }
}

/// A calendar date and time of day in the user's time zone.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

/// Converts a FILETIME value (100 ns ticks since 1601, UTC) to local time.
/// `None` for values outside the representable range.
#[must_use]
pub fn to_local(filetime: i64) -> Option<LocalTime> {
    let [b0, b1, b2, b3, b4, b5, b6, b7] = filetime.to_le_bytes();
    let ft = FILETIME {
        dwLowDateTime: u32::from_le_bytes([b0, b1, b2, b3]),
        dwHighDateTime: u32::from_le_bytes([b4, b5, b6, b7]),
    };
    let mut utc = SYSTEMTIME::default();
    // SAFETY: `ft` is readable and `utc` writable for the call.
    unsafe { FileTimeToSystemTime(&raw const ft, &raw mut utc) }.ok()?;
    let mut local = SYSTEMTIME::default();
    // SAFETY: `utc` is readable and `local` writable for the call; no time
    // zone means the currently active one.
    unsafe { SystemTimeToTzSpecificLocalTime(None, &raw const utc, &raw mut local) }.ok()?;
    Some(LocalTime {
        year: local.wYear,
        month: local.wMonth,
        day: local.wDay,
        hour: local.wHour,
        minute: local.wMinute,
        second: local.wSecond,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn natural_order() {
        assert_eq!(compare_natural(&w("file2"), &w("file10")), Ordering::Less);
        assert_eq!(compare_natural(&w("b"), &w("A")), Ordering::Greater);
        assert_eq!(compare_natural(&w("a"), &w("a")), Ordering::Equal);
        // Equal for the locale, still a total order.
        assert_ne!(compare_natural(&w("a"), &w("A")), Ordering::Equal);
    }

    #[test]
    fn local_time_fields() {
        // 2026-11-23 12:00:00 UTC; every time zone keeps the year.
        let t = to_local(134_399_088_000_000_000).unwrap();
        assert_eq!(t.year, 2026);
        assert!((1..=12).contains(&t.month) && (1..=31).contains(&t.day));
        assert!(to_local(-1).is_none());
    }
}
