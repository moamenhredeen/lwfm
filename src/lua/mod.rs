//! Lua runtime: state creation, the per-call time limit, and error text.
//! All Lua runs on the UI thread.

pub mod api;

use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, Table, VmState};

/// Limit for bindings, openers and event handlers.
pub const CALL_LIMIT: Duration = Duration::from_millis(50);
/// Limit for evaluating `config.lua`.
pub const CONFIG_LIMIT: Duration = Duration::from_secs(1);

/// Nesting depth of `with_limit` on one state; only the outermost call sets the hook.
struct Depth(u32);

/// Creates a state with the safe standard libraries (no `debug`) and
/// `require` searching `<config dir>\lua`.
pub fn new_state(config_dir: Option<&Path>) -> mlua::Result<Lua> {
    let lua = Lua::new();
    lua.set_app_data(Depth(0));
    let package: Table = lua.globals().get("package")?;
    let path = match config_dir {
        Some(dir) => {
            let d = dir.join("lua");
            format!("{0}\\?.lua;{0}\\?\\init.lua", d.display())
        }
        None => String::new(),
    };
    package.set("path", path)?;
    package.set("cpath", "")?;
    Ok(lua)
}

/// Runs `f` with an instruction-count hook that aborts the script once
/// `limit` has passed.
pub fn with_limit<R>(
    lua: &Lua,
    limit: Duration,
    f: impl FnOnce() -> mlua::Result<R>,
) -> mlua::Result<R> {
    let outer = {
        let mut depth = lua
            .app_data_mut::<Depth>()
            .expect("state created by new_state");
        depth.0 += 1;
        depth.0 == 1
    };
    if outer {
        let deadline = Instant::now() + limit;
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(10_000),
            move |_, _| {
                if Instant::now() > deadline {
                    Err(mlua::Error::runtime(format!(
                        "script took too long (limit {} ms)",
                        limit.as_millis()
                    )))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )?;
    }
    let result = f();
    lua.app_data_mut::<Depth>()
        .expect("state created by new_state")
        .0 -= 1;
    if outer {
        lua.remove_hook();
    }
    result
}

/// A Lua error reduced to one line for the status bar.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptError(String);

impl From<&mlua::Error> for ScriptError {
    fn from(e: &mlua::Error) -> ScriptError {
        ScriptError(short_error(e))
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptError {}

/// One-line description of a Lua error.
pub fn short_error(e: &mlua::Error) -> String {
    use mlua::Error::{
        CallbackError, ExternalError, MemoryError, RuntimeError, SyntaxError, WithContext,
    };
    match e {
        CallbackError { cause, .. } | WithContext { cause, .. } => short_error(cause),
        SyntaxError { message, .. } => first_line(message),
        RuntimeError(m) | MemoryError(m) => first_line(m),
        ExternalError(err) => first_line(&err.to_string()),
        other => first_line(&other.to_string()),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endless_loop_is_aborted() {
        let lua = new_state(None).unwrap();
        let started = Instant::now();
        let err =
            with_limit(&lua, CALL_LIMIT, || lua.load("while true do end").exec()).unwrap_err();
        assert!(
            short_error(&err).contains("took too long"),
            "{}",
            short_error(&err)
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        // The hook is gone afterwards: a later long-ish script is not cut short.
        lua.load("local x = 0 for i = 1, 100000 do x = x + i end")
            .exec()
            .unwrap();
    }

    #[test]
    fn require_finds_config_modules() {
        let dir = std::env::temp_dir().join(format!("lwfm-lua-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lua").join("pkg")).unwrap();
        std::fs::write(
            dir.join("lua").join("bookmarks.lua"),
            "return { home = 'x' }",
        )
        .unwrap();
        std::fs::write(dir.join("lua").join("pkg").join("init.lua"), "return 42").unwrap();
        let lua = new_state(Some(&dir)).unwrap();
        let home: String = lua.load("return require('bookmarks').home").eval().unwrap();
        let pkg: i64 = lua.load("return require('pkg')").eval().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((home.as_str(), pkg), ("x", 42));
    }

    #[test]
    fn no_debug_library_and_error_lines() {
        let lua = new_state(None).unwrap();
        assert!(
            lua.load("return debug")
                .eval::<mlua::Value>()
                .unwrap()
                .is_nil()
        );
        let err = lua
            .load("\n\nerror('boom')")
            .set_name("@config.lua")
            .exec()
            .unwrap_err();
        assert_eq!(short_error(&err), "config.lua:3: boom");
        let err = lua
            .load("x = = 1")
            .set_name("@config.lua")
            .exec()
            .unwrap_err();
        assert!(
            short_error(&err).starts_with("config.lua:1:"),
            "{}",
            short_error(&err)
        );
    }
}
