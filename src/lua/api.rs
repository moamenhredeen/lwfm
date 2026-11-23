//! The Lua-facing API: the `lwfm` module (`preset`, `on`) and the `fm`
//! object handed to every function lwfm calls. `fm` exposes each registry
//! command as a method, so Lua has no actions of its own.

use std::collections::HashMap;

use mlua::{Function, Lua, Table, Value, Variadic};

use crate::cmd::registry::{self, CmdError, Target};

pub const EXPLORER: &str = include_str!("../config/explorer.lua");
pub const PRESETS: &[&str] = &["explorer"];

const HANDLERS_KEY: &str = "lwfm.handlers";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Event {
    Startup,
    DirEnter,
}

impl Event {
    pub const NAMES: &'static [&'static str] = &["startup", "dir_enter"];

    fn from_name(s: &str) -> Option<Event> {
        match s {
            "startup" => Some(Event::Startup),
            "dir_enter" => Some(Event::DirEnter),
            _ => None,
        }
    }
}

pub type Handlers = HashMap<Event, Vec<Function>>;

/// What Lua code can act on: every command, plus status messages.
pub trait Host: Target {
    fn notify(&self, msg: &str);
}

/// State visible to Lua as `fm.dir`, `fm.file` and `fm.selection`.
#[derive(Default, Clone, Debug)]
pub struct FmState {
    pub dir: String,
    pub file: Option<String>,
    pub selection: Vec<String>,
}

/// Evaluates a built-in preset into a fresh table.
pub fn preset(lua: &Lua, name: &str) -> mlua::Result<Table> {
    match name {
        "explorer" => lua.load(EXPLORER).set_name("=explorer").eval(),
        _ => Err(mlua::Error::runtime(format!(
            "unknown preset `{name}` (available: {})",
            PRESETS.join(", ")
        ))),
    }
}

/// Installs the `lwfm` module so `require("lwfm")` never touches disk.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let handlers = lua.create_table()?;
    for name in Event::NAMES {
        handlers.set(*name, lua.create_table()?)?;
    }
    lua.set_named_registry_value(HANDLERS_KEY, handlers)?;

    let module = lua.create_table()?;
    module.set(
        "preset",
        lua.create_function(|lua, name: String| preset(lua, &name))?,
    )?;
    module.set(
        "on",
        lua.create_function(|lua, (name, f): (String, Function)| {
            if Event::from_name(&name).is_none() {
                return Err(mlua::Error::runtime(format!(
                    "unknown event `{name}` (available: {})",
                    Event::NAMES.join(", ")
                )));
            }
            let handlers: Table = lua.named_registry_value(HANDLERS_KEY)?;
            let list: Table = handlers.get(name)?;
            list.push(f)
        })?,
    )?;
    let loaded: Table = lua.globals().get::<Table>("package")?.get("loaded")?;
    loaded.set("lwfm", module)
}

/// Event handlers registered with `lwfm.on`, in registration order.
pub fn handlers(lua: &Lua) -> mlua::Result<Handlers> {
    let table: Table = lua.named_registry_value(HANDLERS_KEY)?;
    let mut out = Handlers::new();
    for name in Event::NAMES {
        let list: Table = table.get(*name)?;
        let fns = list
            .sequence_values::<Function>()
            .collect::<mlua::Result<Vec<_>>>()?;
        out.insert(Event::from_name(name).unwrap(), fns);
    }
    Ok(out)
}

fn to_lua_err(e: CmdError) -> mlua::Error {
    mlua::Error::runtime(e.0)
}

fn arg_text(v: &Value) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(mlua::Error::runtime(format!(
            "expected a string argument, got {}",
            other.type_name()
        ))),
    }
}

type Parts = (Vec<String>, Vec<(String, Option<String>)>);

/// Method arguments -> positional strings plus flags from a trailing table.
fn command_parts(mut args: Vec<Value>) -> mlua::Result<Parts> {
    let mut flags = Vec::new();
    if let Some(Value::Table(t)) = args.last() {
        for pair in t.pairs::<String, Value>() {
            let (k, v) = pair?;
            match v {
                Value::Boolean(true) => flags.push((k, None)),
                Value::Boolean(false) | Value::Nil => {}
                other => flags.push((k, Some(arg_text(&other)?))),
            }
        }
        flags.sort();
        args.pop();
    }
    let positional = args
        .iter()
        .map(arg_text)
        .collect::<mlua::Result<Vec<_>>>()?;
    Ok((positional, flags))
}

/// Calls `f(fm, extra...)` with the time limit, where `fm` borrows `host`
/// for the duration of the call. Errors come back as one status-bar line.
pub fn call(
    lua: &Lua,
    host: &dyn Host,
    state: &FmState,
    f: &Function,
    extra: Vec<String>,
) -> Result<(), String> {
    let run = || {
        lua.scope(|scope| {
            let fm = lua.create_table()?;
            fm.set("dir", state.dir.as_str())?;
            fm.set("file", state.file.as_deref())?;
            fm.set(
                "selection",
                lua.create_sequence_from(state.selection.iter().map(String::as_str))?,
            )?;
            fm.set(
                "notify",
                scope.create_function(move |_, (_fm, msg): (Value, String)| {
                    host.notify(&msg);
                    Ok(())
                })?,
            )?;
            fm.set(
                "cmd",
                scope.create_function(move |_, (_fm, s): (Value, String)| {
                    registry::run_str(host, &s).map_err(to_lua_err)
                })?,
            )?;
            for def in registry::commands() {
                let name = def.name;
                let method =
                    scope.create_function(move |_, (_fm, args): (Value, Variadic<Value>)| {
                        let (positional, flags) = command_parts(args.into_iter().collect())?;
                        registry::compile_parts(name, positional, flags)
                            .and_then(|inv| inv.run(host))
                            .map_err(to_lua_err)
                    })?;
                fm.set(name.replace('-', "_"), method)?;
            }
            let mut args: Vec<Value> = vec![Value::Table(fm)];
            for e in &extra {
                args.push(Value::String(lua.create_string(e)?));
            }
            f.call::<()>(Variadic::from_iter(args))
        })
    };
    super::with_limit(lua, super::CALL_LIMIT, run).map_err(|e| super::short_error(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::registry::testing::Recorder;

    impl Host for Recorder {
        fn notify(&self, msg: &str) {
            self.0.borrow_mut().push(format!("notify {msg}"));
        }
    }

    fn state() -> FmState {
        FmState {
            dir: r"C:\work".into(),
            file: Some(r"C:\work\b.txt".into()),
            selection: vec![r"C:\work\a.txt".into(), r"C:\work\b.txt".into()],
        }
    }

    fn run(src: &str) -> (Vec<String>, Result<(), String>) {
        let lua = crate::lua::new_state(None).unwrap();
        install(&lua).unwrap();
        let f: Function = lua.load(src).eval().unwrap();
        let r = Recorder::default();
        let res = call(&lua, &r, &state(), &f, vec![]);
        (r.calls(), res)
    }

    #[test]
    fn reads_state() {
        let (calls, res) = run(
            r#"return function(fm) fm:notify(fm.file .. " " .. #fm.selection .. " " .. fm.dir) end"#,
        );
        res.unwrap();
        assert_eq!(calls, [r"notify C:\work\b.txt 2 C:\work"]);
    }

    #[test]
    fn command_methods() {
        let (calls, res) = run(r#"return function(fm)
                 fm:sort("size", { reverse = true })
                 fm:toggle_hidden()
                 fm:cd("~/Downloads")
                 fm:cmd("search rep")
                 fm:shell("code $d", { console = true })
               end"#);
        res.unwrap();
        assert_eq!(
            calls,
            [
                "sort size true",
                "toggle-hidden",
                "cd ~/Downloads",
                "search rep",
                "shell code $d console=true"
            ]
        );
    }

    #[test]
    fn command_error_is_lua_error() {
        let (calls, res) = run(r#"return function(fm) fm:sort("color"); fm:up() end"#);
        assert!(res.unwrap_err().contains("`color` is not a valid key"));
        assert!(calls.is_empty(), "nothing after the error runs: {calls:?}");
    }

    #[test]
    fn runtime_error_and_timeout() {
        let (_, res) = run(r#"return function(fm) error("boom") end"#);
        assert!(res.unwrap_err().contains("boom"));
        let (_, res) = run("return function(fm) while true do end end");
        assert!(res.unwrap_err().contains("took too long"));
    }

    #[test]
    fn events_registration() {
        let lua = crate::lua::new_state(None).unwrap();
        install(&lua).unwrap();
        lua.load(
            r#"local lwfm = require("lwfm")
               lwfm.on("dir_enter", function() end)
               lwfm.on("dir_enter", function() end)
               lwfm.on("startup", function() end)"#,
        )
        .exec()
        .unwrap();
        let h = handlers(&lua).unwrap();
        assert_eq!(
            (h[&Event::DirEnter].len(), h[&Event::Startup].len()),
            (2, 1)
        );
        let err = lua
            .load(r#"require("lwfm").on("on_copy", function() end)"#)
            .exec()
            .unwrap_err();
        let msg = crate::lua::short_error(&err);
        assert!(
            msg.contains("unknown event `on_copy`") && msg.contains("startup, dir_enter"),
            "{msg}"
        );
    }

    #[test]
    fn unknown_preset() {
        let lua = crate::lua::new_state(None).unwrap();
        install(&lua).unwrap();
        let err = lua
            .load(r#"require("lwfm").preset("vim")"#)
            .exec()
            .unwrap_err();
        let msg = crate::lua::short_error(&err);
        assert!(
            msg.contains("unknown preset `vim`") && msg.contains("explorer"),
            "{msg}"
        );
    }
}
