//! The command registry: every user action is a named command with a fixed
//! argument signature. Keys, mouse, the path bar (and later Lua and IPC)
//! compile command strings into `Invocation`s and run them against a `Target`.

use std::fmt;

use super::parse::{self, Parsed};
use crate::fs::model::{Sort, SortKey};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CmdError(pub String);

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<parse::ParseError> for CmdError {
    fn from(e: parse::ParseError) -> CmdError {
        CmdError(e.0)
    }
}

pub type CmdResult = Result<(), CmdError>;

/// What commands act on. Implemented by the application; mocked in tests.
pub trait Target {
    fn cd(&self, path: &str) -> CmdResult;
    fn up(&self) -> CmdResult;
    fn back(&self) -> CmdResult;
    fn forward(&self) -> CmdResult;
    fn open(&self) -> CmdResult;
    fn refresh(&self) -> CmdResult;
    fn toggle_hidden(&self) -> CmdResult;
    fn sort(&self, sort: Sort) -> CmdResult;
    fn focus_path(&self) -> CmdResult;
    fn focus_list(&self) -> CmdResult;
    fn path_submit(&self) -> CmdResult;
    fn path_cancel(&self) -> CmdResult;
    fn quit(&self) -> CmdResult;
    fn shell(&self, command: &str, console: bool) -> CmdResult;
    fn search(&self, text: &str) -> CmdResult;
    fn search_next(&self) -> CmdResult;
    fn search_prev(&self) -> CmdResult;
    fn search_close(&self) -> CmdResult;
    fn prompt(&self) -> CmdResult;
    fn edit_config(&self) -> CmdResult;
    fn reload_config(&self) -> CmdResult;
}

pub enum ArgKind {
    Path,
    Text,
    OneOf(&'static [&'static str]),
}

pub struct ArgSpec {
    pub name: &'static str,
    pub kind: ArgKind,
}

pub struct FlagSpec {
    pub name: &'static str,
    pub takes_value: bool,
}

pub struct CommandDef {
    pub name: &'static str,
    pub args: &'static [ArgSpec],
    pub flags: &'static [FlagSpec],
    #[cfg_attr(not(test), allow(dead_code))]
    pub desc: &'static str,
    run: fn(&dyn Target, &Args) -> CmdResult,
}

// Listing support for help, completion and scripting bindings in later changes.
#[cfg_attr(not(test), allow(dead_code))]
impl CommandDef {
    /// Argument signature for help and listings, e.g. `<name|size|mtime> [--reverse]`.
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = self
            .args
            .iter()
            .map(|a| match a.kind {
                ArgKind::Path | ArgKind::Text => format!("<{}>", a.name),
                ArgKind::OneOf(choices) => format!("<{}>", choices.join("|")),
            })
            .collect();
        parts.extend(self.flags.iter().map(|f| {
            if f.takes_value {
                format!("[--{}=<value>]", f.name)
            } else {
                format!("[--{}]", f.name)
            }
        }));
        parts.join(" ")
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    pub fn arg(&self, i: usize) -> &str {
        &self.positional[i]
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }
}

pub struct Invocation {
    pub def: &'static CommandDef,
    pub args: Args,
}

impl Invocation {
    pub fn run(&self, target: &dyn Target) -> CmdResult {
        (self.def.run)(target, &self.args)
    }
}

impl fmt::Debug for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invocation({} {:?})", self.def.name, self.args)
    }
}

const SORT_ARGS: &[ArgSpec] = &[ArgSpec {
    name: "key",
    kind: ArgKind::OneOf(SortKey::NAMES),
}];

static COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "cd",
        args: &[ArgSpec {
            name: "path",
            kind: ArgKind::Path,
        }],
        flags: &[],
        desc: "Go to a directory (relative paths and ~ allowed)",
        run: |t, a| t.cd(a.arg(0)),
    },
    CommandDef {
        name: "up",
        args: &[],
        flags: &[],
        desc: "Go to the parent directory",
        run: |t, _| t.up(),
    },
    CommandDef {
        name: "back",
        args: &[],
        flags: &[],
        desc: "Go back in history",
        run: |t, _| t.back(),
    },
    CommandDef {
        name: "forward",
        args: &[],
        flags: &[],
        desc: "Go forward in history",
        run: |t, _| t.forward(),
    },
    CommandDef {
        name: "open",
        args: &[],
        flags: &[],
        desc: "Enter the focused directory or open the focused file",
        run: |t, _| t.open(),
    },
    CommandDef {
        name: "refresh",
        args: &[],
        flags: &[],
        desc: "Re-read the current directory",
        run: |t, _| t.refresh(),
    },
    CommandDef {
        name: "toggle-hidden",
        args: &[],
        flags: &[],
        desc: "Show or hide hidden and system items",
        run: |t, _| t.toggle_hidden(),
    },
    CommandDef {
        name: "sort",
        args: SORT_ARGS,
        flags: &[FlagSpec {
            name: "reverse",
            takes_value: false,
        }],
        desc: "Sort the list by a column",
        run: |t, a| {
            let key = SortKey::from_name(a.arg(0)).expect("validated by compile");
            t.sort(Sort {
                key,
                reverse: a.flag("reverse"),
            })
        },
    },
    CommandDef {
        name: "focus-path",
        args: &[],
        flags: &[],
        desc: "Focus the path bar",
        run: |t, _| t.focus_path(),
    },
    CommandDef {
        name: "focus-list",
        args: &[],
        flags: &[],
        desc: "Focus the file list",
        run: |t, _| t.focus_list(),
    },
    CommandDef {
        name: "path-submit",
        args: &[],
        flags: &[],
        desc: "Go to the directory typed in the path bar",
        run: |t, _| t.path_submit(),
    },
    CommandDef {
        name: "path-cancel",
        args: &[],
        flags: &[],
        desc: "Discard path bar edits and focus the list",
        run: |t, _| t.path_cancel(),
    },
    CommandDef {
        name: "quit",
        args: &[],
        flags: &[],
        desc: "Exit lwfm",
        run: |t, _| t.quit(),
    },
    CommandDef {
        name: "shell",
        args: &[ArgSpec {
            name: "command",
            kind: ArgKind::Text,
        }],
        flags: &[FlagSpec {
            name: "console",
            takes_value: false,
        }],
        desc: "Run a command line ($f, $fs, $d substituted) without blocking",
        run: |t, a| t.shell(a.arg(0), a.flag("console")),
    },
    CommandDef {
        name: "search",
        args: &[ArgSpec {
            name: "text",
            kind: ArgKind::Text,
        }],
        flags: &[],
        desc: "Open the search input with text and select the first match",
        run: |t, a| t.search(a.arg(0)),
    },
    CommandDef {
        name: "search-next",
        args: &[],
        flags: &[],
        desc: "Select the next entry matching the search",
        run: |t, _| t.search_next(),
    },
    CommandDef {
        name: "search-prev",
        args: &[],
        flags: &[],
        desc: "Select the previous entry matching the search",
        run: |t, _| t.search_prev(),
    },
    CommandDef {
        name: "search-close",
        args: &[],
        flags: &[],
        desc: "Close the search input and keep the selection",
        run: |t, _| t.search_close(),
    },
    CommandDef {
        name: "prompt",
        args: &[],
        flags: &[],
        desc: "Type a command in the path bar",
        run: |t, _| t.prompt(),
    },
    CommandDef {
        name: "edit-config",
        args: &[],
        flags: &[],
        desc: "Open config.lua in the editor (creates a starter config)",
        run: |t, _| t.edit_config(),
    },
    CommandDef {
        name: "reload-config",
        args: &[],
        flags: &[],
        desc: "Reload config.lua",
        run: |t, _| t.reload_config(),
    },
];

pub fn commands() -> &'static [CommandDef] {
    COMMANDS
}

pub fn lookup(name: &str) -> Option<&'static CommandDef> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Parses and validates a command string without running it.
pub fn compile(s: &str) -> Result<Invocation, CmdError> {
    let Parsed {
        name,
        positional,
        flags,
    } = parse::parse(s)?;
    compile_parts(&name, positional, flags)
}

/// Validates an already split command (as built from Lua method calls).
pub fn compile_parts(
    name: &str,
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
) -> Result<Invocation, CmdError> {
    let def = lookup(name).ok_or_else(|| CmdError(format!("unknown command `{name}`")))?;

    if positional.len() < def.args.len() {
        let missing = &def.args[positional.len()];
        return Err(CmdError(format!(
            "`{name}`: missing argument <{}>",
            missing.name
        )));
    }
    if positional.len() > def.args.len() {
        return Err(CmdError(format!(
            "`{name}`: unexpected argument `{}`",
            positional[def.args.len()]
        )));
    }
    for (spec, value) in def.args.iter().zip(&positional) {
        if let ArgKind::OneOf(choices) = spec.kind
            && !choices.contains(&value.as_str())
        {
            return Err(CmdError(format!(
                "`{name}`: `{value}` is not a valid {} (expected {})",
                spec.name,
                choices.join(", ")
            )));
        }
    }
    for (flag, value) in &flags {
        let spec = def
            .flags
            .iter()
            .find(|f| f.name == flag)
            .ok_or_else(|| CmdError(format!("`{name}`: unknown option `--{flag}`")))?;
        match (spec.takes_value, value) {
            (true, None) => return Err(CmdError(format!("`{name}`: `--{flag}` needs a value"))),
            (false, Some(_)) => {
                return Err(CmdError(format!("`{name}`: `--{flag}` takes no value")));
            }
            _ => {}
        }
    }
    Ok(Invocation {
        def,
        args: Args { positional, flags },
    })
}

pub fn run_str(target: &dyn Target, s: &str) -> CmdResult {
    compile(s)?.run(target)
}

/// A `Target` that records every call, for tests of commands, keymaps and Lua.
#[cfg(test)]
pub mod testing {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Recorder(pub RefCell<Vec<String>>);

    impl Recorder {
        pub fn calls(&self) -> Vec<String> {
            self.0.borrow().clone()
        }

        fn log(&self, s: impl Into<String>) -> CmdResult {
            self.0.borrow_mut().push(s.into());
            Ok(())
        }
    }

    impl Target for Recorder {
        fn cd(&self, path: &str) -> CmdResult {
            self.log(format!("cd {path}"))
        }
        fn up(&self) -> CmdResult {
            self.log("up")
        }
        fn back(&self) -> CmdResult {
            self.log("back")
        }
        fn forward(&self) -> CmdResult {
            self.log("forward")
        }
        fn open(&self) -> CmdResult {
            self.log("open")
        }
        fn refresh(&self) -> CmdResult {
            self.log("refresh")
        }
        fn toggle_hidden(&self) -> CmdResult {
            self.log("toggle-hidden")
        }
        fn sort(&self, s: Sort) -> CmdResult {
            self.log(format!("sort {} {}", s.key.name(), s.reverse))
        }
        fn focus_path(&self) -> CmdResult {
            self.log("focus-path")
        }
        fn focus_list(&self) -> CmdResult {
            self.log("focus-list")
        }
        fn path_submit(&self) -> CmdResult {
            self.log("path-submit")
        }
        fn path_cancel(&self) -> CmdResult {
            self.log("path-cancel")
        }
        fn quit(&self) -> CmdResult {
            self.log("quit")
        }
        fn shell(&self, command: &str, console: bool) -> CmdResult {
            self.log(format!("shell {command} console={console}"))
        }
        fn search(&self, text: &str) -> CmdResult {
            self.log(format!("search {text}"))
        }
        fn search_next(&self) -> CmdResult {
            self.log("search-next")
        }
        fn search_prev(&self) -> CmdResult {
            self.log("search-prev")
        }
        fn search_close(&self) -> CmdResult {
            self.log("search-close")
        }
        fn prompt(&self) -> CmdResult {
            self.log("prompt")
        }
        fn edit_config(&self) -> CmdResult {
            self.log("edit-config")
        }
        fn reload_config(&self) -> CmdResult {
            self.log("reload-config")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Recorder;
    use super::*;

    #[test]
    fn runs_commands() {
        let r = Recorder::default();
        run_str(&r, r#"cd "C:\Program Files""#).unwrap();
        run_str(&r, "sort size --reverse").unwrap();
        run_str(&r, "toggle-hidden").unwrap();
        run_str(&r, "search rep").unwrap();
        run_str(&r, r#"shell --console "nvim $f""#).unwrap();
        run_str(&r, "search-next").unwrap();
        assert_eq!(
            r.calls(),
            [
                r"cd C:\Program Files",
                "sort size true",
                "toggle-hidden",
                "search rep",
                "shell nvim $f console=true",
                "search-next"
            ]
        );
    }

    #[test]
    fn unknown_command() {
        let r = Recorder::default();
        assert_eq!(run_str(&r, "cpoy").unwrap_err().0, "unknown command `cpoy`");
        assert!(r.calls().is_empty());
    }

    #[test]
    fn invalid_arguments() {
        let err = |s: &str| compile(s).unwrap_err().0;
        assert!(err("sort color").contains("`color` is not a valid key"));
        assert!(err("cd").contains("missing argument <path>"));
        assert!(err("up now").contains("unexpected argument `now`"));
        assert!(err("sort name --fast").contains("unknown option `--fast`"));
        assert!(err("sort name --reverse=yes").contains("takes no value"));
        assert!(err(r#"cd "C:\x"#).contains("unterminated quote"));
        assert!(err("shell").contains("missing argument <command>"));
    }

    #[test]
    fn compile_parts_validates_like_strings() {
        let r = Recorder::default();
        compile_parts("sort", vec!["size".into()], vec![("reverse".into(), None)])
            .unwrap()
            .run(&r)
            .unwrap();
        assert_eq!(r.calls(), ["sort size true"]);
        assert!(compile_parts("sort", vec!["color".into()], vec![]).is_err());
        assert!(compile_parts("nope", vec![], vec![]).is_err());
    }

    #[test]
    fn lists_all_commands() {
        let names: Vec<_> = commands().iter().map(|c| c.name).collect();
        for required in [
            "cd",
            "up",
            "back",
            "forward",
            "open",
            "refresh",
            "toggle-hidden",
            "sort",
            "focus-path",
            "focus-list",
            "path-submit",
            "path-cancel",
            "quit",
            "shell",
            "search",
            "search-next",
            "search-prev",
            "search-close",
            "prompt",
            "edit-config",
            "reload-config",
        ] {
            assert!(names.contains(&required), "missing {required}");
        }
        for c in commands() {
            assert!(!c.desc.is_empty(), "{} has no description", c.name);
        }
        assert_eq!(
            lookup("sort").unwrap().signature(),
            "<name|size|mtime> [--reverse]"
        );
        assert_eq!(lookup("cd").unwrap().signature(), "<path>");
        assert_eq!(
            lookup("shell").unwrap().signature(),
            "<command> [--console]"
        );
    }
}
