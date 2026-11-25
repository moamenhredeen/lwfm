//! Path handling: user input resolution, `~` expansion and parents. Paths
//! are kept absolute and normalized; the `\\?\` prefix for long paths is
//! added by `lwfm-win` when it calls file APIs.

use std::path::{Component, Path, PathBuf};

/// Resolves user input against the current directory: expands `~`, joins
/// relative paths and removes `.` and `..` components.
pub fn resolve(input: &str, cwd: &Path, home: &Path) -> PathBuf {
    let input = input.trim();
    let expanded = match input.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(input),
    };
    // `C:` alone is drive-relative in Windows; treat it as the drive root.
    let expanded = match expanded.to_str() {
        Some(s) if s.len() == 2 && s.ends_with(':') => PathBuf::from(format!("{s}\\")),
        _ => expanded,
    };
    normalize(&cwd.join(expanded))
}

/// Lexical normalization: drops `.`, applies `..` without going above the
/// root, and yields backslash separators.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if out.file_name().is_some() {
                    out.pop();
                }
            }
            Component::Normal(n) => out.push(n),
        }
    }
    out
}

/// Parent directory, or `None` at a drive root or share root.
pub fn parent(p: &Path) -> Option<PathBuf> {
    p.parent().map(Path::to_path_buf)
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("C:\\"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: &str) -> String {
        resolve(
            input,
            Path::new(r"C:\Users\me\docs"),
            Path::new(r"C:\Users\me"),
        )
        .to_string_lossy()
        .into_owned()
    }

    #[test]
    fn resolves_input() {
        assert_eq!(r(r"C:\Windows"), r"C:\Windows");
        assert_eq!(r("sub"), r"C:\Users\me\docs\sub");
        assert_eq!(r(".."), r"C:\Users\me");
        assert_eq!(r(r"..\..\..\..\.."), r"C:\");
        assert_eq!(r(r".\a\.\b\..\c"), r"C:\Users\me\docs\a\c");
        assert_eq!(r("C:/Windows/System32"), r"C:\Windows\System32");
        assert_eq!(r(r"\Temp"), r"C:\Temp");
        assert_eq!(r("D:"), r"D:\");
        assert_eq!(r(r"\\server\share\x"), r"\\server\share\x");
    }

    #[test]
    fn expands_home() {
        assert_eq!(r("~"), r"C:\Users\me");
        assert_eq!(r("~/Downloads"), r"C:\Users\me\Downloads");
        assert_eq!(r(r"~\Downloads"), r"C:\Users\me\Downloads");
        assert_eq!(r("~x"), r"C:\Users\me\docs\~x");
    }

    #[test]
    fn parents() {
        assert_eq!(
            parent(Path::new(r"C:\Users\me")).unwrap(),
            Path::new(r"C:\Users")
        );
        assert_eq!(parent(Path::new(r"C:\Users")).unwrap(), Path::new(r"C:\"));
        assert_eq!(parent(Path::new(r"C:\")), None);
        assert_eq!(parent(Path::new(r"\\server\share\")), None);
        assert_eq!(
            parent(Path::new(r"\\server\share\x")).unwrap(),
            Path::new(r"\\server\share\")
        );
    }
}
