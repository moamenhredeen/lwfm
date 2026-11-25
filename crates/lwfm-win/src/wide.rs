//! UTF-16 encoding for system calls. Nothing here loses information: names
//! that are not valid Unicode pass through unchanged.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// Paths at least this long get the `\\?\` prefix. Lower than 260 so that a
/// `\*` search suffix still fits for the `FindFirstFileExW` fallback.
const LONG_PATH: usize = 240;

/// NUL-terminated UTF-16.
pub(crate) fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

/// NUL-terminated UTF-16 for file APIs, with `\\?\` or `\\?\UNC\` for long
/// paths. `suffix` is appended before termination (e.g. `\*`).
pub(crate) fn api_path(p: &Path, suffix: &str) -> Vec<u16> {
    let s: Vec<u16> = p.as_os_str().encode_wide().collect();
    let mut out = Vec::with_capacity(s.len() + suffix.len() + 9);
    let already = s.starts_with(&units(r"\\?\"));
    if s.len() + suffix.len() >= LONG_PATH && !already {
        if let Some(unc) = s.strip_prefix(&units(r"\\")[..]) {
            out.extend(units(r"\\?\UNC\"));
            out.extend_from_slice(unc);
        } else {
            out.extend(units(r"\\?\"));
            out.extend_from_slice(&s);
        }
    } else {
        out.extend_from_slice(&s);
    }
    out.extend(OsStr::new(suffix).encode_wide());
    out.push(0);
    out
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(p: &str) -> String {
        let w = api_path(Path::new(p), "");
        String::from_utf16(&w[..w.len() - 1]).unwrap()
    }

    #[test]
    fn long_path_prefix() {
        assert_eq!(api(r"C:\short"), r"C:\short");
        let long = format!(r"C:\{}", "a".repeat(300));
        assert_eq!(api(&long), format!(r"\\?\{long}"));
        let unc = format!(r"\\server\share\{}", "b".repeat(300));
        assert_eq!(
            api(&unc),
            format!(r"\\?\UNC\server\share\{}", "b".repeat(300))
        );
        let pre = format!(r"\\?\C:\{}", "c".repeat(300));
        assert_eq!(api(&pre), pre);
        let w = api_path(Path::new(r"C:\x"), r"\*");
        assert_eq!(String::from_utf16(&w[..w.len() - 1]).unwrap(), r"C:\x\*");
    }
}
