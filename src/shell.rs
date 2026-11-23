//! External commands: `$f`/`$fs`/`$d` expansion, opener patterns, and
//! launching command lines through `cmd.exe` off the UI thread.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    GetExitCodeProcess, INFINITE, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

/// Runs `line` through `cmd.exe /d /s /c` in `dir` on a new thread. Without a
/// console the process is waited for and a non-zero exit code is reported;
/// launch failures are reported too.
pub fn launch(line: String, dir: PathBuf, console: bool, report: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut cmdline: Vec<u16> = format!("\"{comspec}\" /d /s /c \"{line}\"")
            .encode_utf16()
            .chain([0])
            .collect();
        let dir_w: Vec<u16> = dir
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain([0])
            .collect();
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        let flags = CREATE_UNICODE_ENVIRONMENT
            | if console {
                CREATE_NEW_CONSOLE
            } else {
                CREATE_NO_WINDOW
            };
        let created = unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                None,
                PCWSTR(dir_w.as_ptr()),
                &startup,
                &mut info,
            )
        };
        if let Err(e) = created {
            report(format!("Cannot run `{line}`: {}", e.message()));
            return;
        }
        unsafe {
            let _ = CloseHandle(info.hThread);
            if !console {
                WaitForSingleObject(info.hProcess, INFINITE);
                let mut code = 0u32;
                if GetExitCodeProcess(info.hProcess, &mut code).is_ok() && code != 0 {
                    report(format!("`{line}` exited with code {code}"));
                }
            }
            let _ = CloseHandle(info.hProcess);
        }
    });
}

/// What a command line can refer to.
pub struct Context<'a> {
    pub dir: &'a Path,
    pub focused: Option<&'a Path>,
    pub selection: &'a [PathBuf],
}

fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

/// Substitutes `$f`, `$fs`, `$d` and `$$` in a command line.
pub fn expand(line: &str, ctx: &Context) -> Result<String, String> {
    let no_focus = || "no entry is focused".to_string();
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (text, len) = if after.starts_with('$') {
            ("$".to_string(), 1)
        } else if after.starts_with("fs") {
            let paths: Vec<String> = if ctx.selection.is_empty() {
                vec![quoted(ctx.focused.ok_or_else(no_focus)?)]
            } else {
                ctx.selection.iter().map(|p| quoted(p)).collect()
            };
            (paths.join(" "), 2)
        } else if after.starts_with('f') {
            (quoted(ctx.focused.ok_or_else(no_focus)?), 1)
        } else if after.starts_with('d') {
            (quoted(ctx.dir), 1)
        } else {
            ("$".to_string(), 0)
        };
        out.push_str(&text);
        rest = &after[len..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Case-insensitive wildcard match of a file name (`*` any run, `?` one char).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Index of the opener to use for `name`: the matching pattern with the most
/// characters (ties go to the first).
pub fn best_opener<'a>(patterns: impl IntoIterator<Item = &'a str>, name: &str) -> Option<usize> {
    patterns
        .into_iter()
        .enumerate()
        .filter(|(_, p)| glob_match(p, name))
        .fold(None, |best: Option<(usize, usize)>, (i, p)| {
            let len = p.chars().count();
            match best {
                Some((_, l)) if l >= len => best,
                _ => Some((i, len)),
            }
        })
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(focused: Option<&'a Path>, selection: &'a [PathBuf]) -> Context<'a> {
        Context {
            dir: Path::new(r"C:\w"),
            focused,
            selection,
        }
    }

    #[test]
    fn expands_variables() {
        let sel = [PathBuf::from(r"C:\w\a b.txt"), PathBuf::from(r"C:\w\c.txt")];
        let c = ctx(Some(Path::new(r"C:\w\c.txt")), &sel);
        assert_eq!(
            expand("7z a out.7z $fs", &c).unwrap(),
            r#"7z a out.7z "C:\w\a b.txt" "C:\w\c.txt""#
        );
        assert_eq!(expand("code $d", &c).unwrap(), r#"code "C:\w""#);
        assert_eq!(expand("nvim $f", &c).unwrap(), r#"nvim "C:\w\c.txt""#);
        assert_eq!(expand("echo $$HOME", &c).unwrap(), "echo $HOME");
        assert_eq!(expand("echo $x and $", &c).unwrap(), "echo $x and $");
    }

    #[test]
    fn selection_defaults_to_focused() {
        let c = ctx(Some(Path::new(r"C:\w\c.txt")), &[]);
        assert_eq!(expand("x $fs", &c).unwrap(), r#"x "C:\w\c.txt""#);
    }

    #[test]
    fn nothing_focused() {
        let c = ctx(None, &[]);
        assert_eq!(expand("notepad $f", &c).unwrap_err(), "no entry is focused");
        assert!(expand("x $fs", &c).is_err());
        assert!(expand("x $d", &c).is_ok());
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.md", "README.MD"));
        assert!(glob_match("*.log.txt", "app.log.txt"));
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file10.txt"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("*.md", "notes.txt"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
    }

    #[test]
    fn most_specific_opener() {
        let pats = ["*.txt", "*.log.txt"];
        assert_eq!(best_opener(pats, "app.log.txt"), Some(1));
        assert_eq!(best_opener(pats, "notes.txt"), Some(0));
        assert_eq!(best_opener(pats, "photo.jpg"), None);
    }
}
