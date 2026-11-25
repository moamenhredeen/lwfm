//! Configuration from `config.lua`: discovery, loading and reloading. A load
//! produces a complete `Loaded` value that replaces the previous one at once.

pub mod discover;
pub mod schema;
pub mod watch;

use std::fmt;
use std::path::{Path, PathBuf};

use lwfm_win::Color;
use mlua::{Lua, Value};

use crate::cmd::registry::{self, Invocation};
use crate::fs::model::Sort;
use crate::keys::keymap::{Binding, Keymap};
use crate::lua::{self, ScriptError, api};

pub use schema::SettingError;

pub const STARTER: &str = include_str!("starter.lua");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub dir: Color,
    pub hidden: Color,
}

pub struct Settings {
    pub show_hidden: bool,
    pub sort: Sort,
    pub colors: Colors,
    /// Run by `edit-config` with `$f` set to the config file.
    pub editor: Invocation,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            show_hidden: false,
            sort: Sort::default(),
            colors: Colors {
                dir: Color::rgb(0x1a, 0x5f, 0xb4),
                hidden: Color::rgb(0x80, 0x80, 0x80),
            },
            editor: registry::compile(r#"shell "notepad $f""#).expect("valid default editor"),
        }
    }
}

pub struct Loaded {
    /// Owns every Lua function referenced below.
    pub lua: Lua,
    pub settings: Settings,
    pub keymap: Keymap,
    pub openers: Vec<(String, Binding)>,
    pub handlers: api::Handlers,
    pub warnings: Vec<String>,
}

/// Why a configuration could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The config file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Lua failed to evaluate the script.
    Script(ScriptError),
    /// The script ran, but what it returned is not a valid configuration.
    Invalid { file: String, source: SettingError },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
            ConfigError::Script(e) => e.fmt(f),
            ConfigError::Invalid { file, source } => write!(f, "{file}: {source}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Read { source, .. } => Some(source),
            ConfigError::Script(e) => Some(e),
            ConfigError::Invalid { source, .. } => Some(source),
        }
    }
}

// By value, so that it can be passed to `map_err` directly.
#[allow(clippy::needless_pass_by_value)]
fn short(e: mlua::Error) -> ConfigError {
    ConfigError::Script(ScriptError::from(&e))
}

/// Built-in defaults: the explorer preset with no config file.
pub fn defaults() -> Loaded {
    load(None, None).expect("built-in preset is valid")
}

/// Evaluates `source` (the content of `path`) or, without a source, the
/// explorer preset.
pub fn load(path: Option<&Path>, source: Option<&str>) -> Result<Loaded, ConfigError> {
    let name = path.and_then(Path::file_name).map_or_else(
        || "config.lua".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let lua = lua::new_state(path.and_then(Path::parent)).map_err(short)?;
    api::install(&lua).map_err(short)?;
    let preset = api::preset(&lua, "explorer").map_err(short)?;
    let cfg = match source {
        Some(src) => {
            let value: Value = lua::with_limit(&lua, lua::CONFIG_LIMIT, || {
                lua.load(src).set_name(format!("@{name}")).eval()
            })
            .map_err(short)?;
            match value {
                Value::Table(t) => t,
                other => {
                    return Err(ConfigError::Invalid {
                        file: name,
                        source: SettingError::new(format!(
                            "the script must return a table (got {})",
                            other.type_name()
                        )),
                    });
                }
            }
        }
        None => api::preset(&lua, "explorer").map_err(short)?,
    };
    let read = schema::read(&cfg, &preset)
        .map_err(|source| ConfigError::Invalid { file: name, source })?;
    let handlers = api::handlers(&lua).map_err(short)?;
    Ok(Loaded {
        lua,
        settings: read.settings,
        keymap: read.keymap,
        openers: read.openers,
        handlers,
        warnings: read.warnings,
    })
}

/// Reads a config file as text. Called before the window exists and on the
/// watcher thread, never on the UI thread of a running window.
pub fn read_file(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::model::SortKey;
    use crate::keys::keymap::{Action, Lookup, Scope, parse_seq};

    fn cfg(src: &str) -> Result<Loaded, String> {
        load(Some(Path::new(r"C:\cfg\config.lua")), Some(src)).map_err(|e| e.to_string())
    }

    fn bound(l: &Loaded, scope: Scope, keys: &str) -> Option<String> {
        let chain: Vec<Scope> = match scope {
            Scope::Global => vec![Scope::Global],
            s => vec![s],
        };
        match l.keymap.lookup(&chain, &parse_seq(keys).unwrap()) {
            Lookup::Match(b) => Some(match &b.action {
                Action::Commands(invs) => invs
                    .iter()
                    .map(|i| format!("{} {:?}", i.def.name, i.args))
                    .collect::<Vec<_>>()
                    .join("; "),
                Action::Lua(_) => "lua".into(),
            }),
            _ => None,
        }
    }

    fn name_of(l: &Loaded, scope: Scope, keys: &str) -> Option<String> {
        bound(l, scope, keys).map(|s| s.split(' ').next().unwrap().to_string())
    }

    #[test]
    fn defaults_have_the_explorer_keymap() {
        let d = defaults();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let table = [
            (Scope::Global, "ctrl+q", "quit"),
            (Scope::Global, "ctrl+l", "focus-path"),
            (Scope::Global, "alt+d", "focus-path"),
            (Scope::Global, "f4", "focus-path"),
            (Scope::Global, "alt+left", "back"),
            (Scope::Global, "alt+right", "forward"),
            (Scope::Global, "alt+up", "up"),
            (Scope::Global, "f5", "refresh"),
            (Scope::Global, "ctrl+r", "refresh"),
            (Scope::Global, "ctrl+h", "toggle-hidden"),
            (Scope::Global, "ctrl+,", "edit-config"),
            (Scope::List, "enter", "open"),
            (Scope::List, "backspace", "up"),
            (Scope::List, ":", "prompt"),
            (Scope::Search, "esc", "search-close"),
            (Scope::Search, "enter", "search-close"),
            (Scope::Search, "down", "search-next"),
            (Scope::Search, "up", "search-prev"),
            (Scope::Input, "enter", "path-submit"),
            (Scope::Input, "esc", "path-cancel"),
        ];
        for (scope, keys, cmd) in table {
            assert_eq!(
                name_of(&d, scope, keys).as_deref(),
                Some(cmd),
                "{scope:?} {keys}"
            );
        }
        // Enter in the search input closes it, then opens the selected entry.
        let enter = bound(&d, Scope::Search, "enter").unwrap();
        assert!(
            enter.starts_with("search-close") && enter.contains("; open"),
            "{enter}"
        );
    }

    #[test]
    fn preset_settings_match_defaults() {
        let d = defaults();
        let s = Settings::default();
        assert_eq!(d.settings.show_hidden, s.show_hidden);
        assert_eq!(d.settings.sort, s.sort);
        assert_eq!(d.settings.colors, s.colors);
        assert_eq!(d.settings.editor.def.name, "shell");
    }

    #[test]
    fn minimal_config_and_settings() {
        let l = cfg(
            r#"local cfg = require("lwfm").preset("explorer"); cfg.show_hidden = true; return cfg"#,
        )
        .unwrap();
        assert!(l.settings.show_hidden);
        assert_eq!(
            name_of(&l, Scope::Global, "ctrl+q").as_deref(),
            Some("quit")
        );
        let l = cfg(r##"return { sort = { key = "mtime", reverse = true }, colors = { dir = "#ff0000" } }"##).unwrap();
        assert_eq!(
            l.settings.sort,
            Sort {
                key: SortKey::Mtime,
                reverse: true
            }
        );
        assert_eq!(l.settings.colors.dir, Color::rgb(0xff, 0, 0));
        assert_eq!(l.settings.colors.hidden, Color::rgb(0x80, 0x80, 0x80));
        // Without the preset table the explorer keys still form the base layer.
        assert_eq!(name_of(&l, Scope::List, "enter").as_deref(), Some("open"));
    }

    #[test]
    fn wrong_return_value_and_unknown_preset() {
        let e = cfg("return 42").err().unwrap();
        assert!(e.contains("must return a table"), "{e}");
        let e = cfg(r#"return require("lwfm").preset("vim")"#)
            .err()
            .unwrap();
        assert!(
            e.contains("unknown preset `vim`") && e.contains("explorer"),
            "{e}"
        );
    }

    #[test]
    fn validation_errors_and_warnings() {
        let e = cfg(r#"return { colors = { dir = "blue" } }"#)
            .err()
            .unwrap();
        assert!(e.contains("colors.dir") && e.contains("#rrggbb"), "{e}");
        let e = cfg(r#"return { sort = { key = "color" } }"#).err().unwrap();
        assert!(
            e.contains("sort.key: expected one of name, size, mtime"),
            "{e}"
        );
        let e = cfg(r#"return { show_hidden = "yes" }"#).err().unwrap();
        assert!(e.contains("show_hidden: expected true or false"), "{e}");
        let e = cfg(r#"return { editor = "cpoy" }"#).err().unwrap();
        assert!(e.contains("editor: unknown command `cpoy`"), "{e}");
        let l = cfg(r#"return { show_hiden = true, sort = { kye = "name" } }"#).unwrap();
        assert_eq!(
            l.warnings,
            ["unknown setting `show_hiden`", "unknown setting `sort.kye`"]
        );
    }

    #[test]
    fn errors_carry_line_numbers() {
        let e = cfg("local x = 1\n\n\n\n\n\nlocal = 2\nreturn {}")
            .err()
            .unwrap();
        assert!(e.starts_with("config.lua:7:"), "{e}");
        let e = cfg("\n\nerror('boom')").err().unwrap();
        assert_eq!(e, "config.lua:3: boom");
    }

    #[test]
    fn keys_override_unbind_and_functions() {
        let l = cfg(r#"local cfg = require("lwfm").preset("explorer")
               cfg.keys.global["ctrl+h"] = "refresh"
               cfg.keys.list["backspace"] = "noop"
               cfg.keys.list["ctrl+e"] = function(fm) fm:notify(fm.dir) end
               cfg.keys.global["ctrl+g d"] = { run = "cd ~/Downloads", desc = "Downloads" }
               cfg.keys.global["f9"] = { "refresh", "toggle-hidden" }
               return cfg"#)
        .unwrap();
        assert_eq!(
            name_of(&l, Scope::Global, "ctrl+h").as_deref(),
            Some("refresh")
        );
        assert_eq!(bound(&l, Scope::List, "backspace"), None);
        assert_eq!(bound(&l, Scope::List, "ctrl+e").as_deref(), Some("lua"));
        assert!(
            bound(&l, Scope::Global, "ctrl+g d")
                .unwrap()
                .starts_with("cd ")
        );
        assert_eq!(
            bound(&l, Scope::Global, "f9").unwrap().matches(';').count(),
            1
        );
        match l
            .keymap
            .lookup(&[Scope::Global], &parse_seq("ctrl+g").unwrap())
        {
            Lookup::Prefix(h) => assert_eq!(h, [("d".to_string(), "Downloads".to_string())]),
            _ => panic!("expected prefix"),
        }
    }

    #[test]
    fn keymap_conflicts() {
        let e = cfg(r#"return { keys = { list = { ["ctrl+x"] = "up", ["Ctrl+X"] = "back" } } }"#)
            .err()
            .unwrap();
        assert!(e.contains("same keys"), "{e}");
        let e =
            cfg(r#"return { keys = { global = { ["ctrl+g"] = "up", ["ctrl+g d"] = "back" } } }"#)
                .err()
                .unwrap();
        assert!(e.contains("`ctrl+g` is a prefix of `ctrl+g d`"), "{e}");
        let e = cfg(r"return { keys = { lsit = {} } }").err().unwrap();
        assert!(e.contains("keys.lsit: unknown scope"), "{e}");
        let e = cfg(r#"return { keys = { list = { ["ctrl+foo"] = "up" } } }"#)
            .err()
            .unwrap();
        assert!(e.contains("ctrl+foo"), "{e}");
        // A user sequence under a preset key shadows the preset binding.
        let l = cfg(r#"return { keys = { global = { ["ctrl+h d"] = "cd ~" } } }"#).unwrap();
        assert!(bound(&l, Scope::Global, "ctrl+h d").is_some());
    }

    #[test]
    fn openers_and_handlers() {
        let l = cfg(
            r#"local lwfm = require("lwfm")
               lwfm.on("dir_enter", function(fm, dir) end)
               return { openers = { ["*.md"] = 'shell --console "nvim $f"', ["*.log"] = function(fm) end } }"#,
        )
        .unwrap();
        let pats: Vec<&str> = l.openers.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(pats, ["*.log", "*.md"]);
        assert_eq!(l.handlers[&api::Event::DirEnter].len(), 1);
        let e = cfg(r#"return { openers = { ["*.md"] = "cpoy" } }"#)
            .err()
            .unwrap();
        assert!(e.contains("openers[\"*.md\"]"), "{e}");
    }

    #[test]
    fn starter_config_is_clean() {
        let l = load(Some(Path::new(r"C:\cfg\config.lua")), Some(STARTER)).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(
            name_of(&l, Scope::Global, "ctrl+,").as_deref(),
            Some("edit-config")
        );
    }
}
