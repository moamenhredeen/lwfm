//! Turns the table returned by `config.lua` into settings, keymap layers and
//! openers. Wrong values are errors with a dotted path; unknown keys are
//! warnings.

use std::collections::HashMap;

use mlua::{Table, Value};

use super::Settings;
use crate::cmd::registry;
use crate::fs::model::SortKey;
use crate::keys::chord::Chord;
use crate::keys::keymap::{Action, Binding, Entry, Keymap, Scope, parse_seq};

pub struct Read {
    pub settings: Settings,
    pub keymap: Keymap,
    pub openers: Vec<(String, Binding)>,
    pub warnings: Vec<String>,
}

fn key_name(k: &Value) -> Result<String, String> {
    match k {
        Value::String(s) => Ok(s.to_str().map_err(|e| e.to_string())?.to_string()),
        other => Err(format!(
            "expected string keys, got a {} key",
            other.type_name()
        )),
    }
}

/// `(key, value)` pairs of a table, with string keys, in a stable order.
fn pairs(t: &Table, path: &str) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| format!("{path}: {e}"))?;
        let k = key_name(&k).map_err(|e| {
            if path.is_empty() {
                e
            } else {
                format!("{path}: {e}")
            }
        })?;
        out.push((k, v));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn table<'a>(v: &'a Value, path: &str) -> Result<&'a Table, String> {
    match v {
        Value::Table(t) => Ok(t),
        other => Err(format!(
            "{path}: expected a table, got {}",
            other.type_name()
        )),
    }
}

fn string(v: &Value, path: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.to_str().map_err(|e| format!("{path}: {e}"))?.to_string()),
        other => Err(format!(
            "{path}: expected a string, got {}",
            other.type_name()
        )),
    }
}

fn boolean(v: &Value, path: &str) -> Result<bool, String> {
    match v {
        Value::Boolean(b) => Ok(*b),
        other => Err(format!(
            "{path}: expected true or false, got {}",
            other.type_name()
        )),
    }
}

/// `"#rrggbb"` -> COLORREF (0x00BBGGRR).
pub fn parse_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = (rgb >> 16, (rgb >> 8) & 0xFF, rgb & 0xFF);
    Some(r | (g << 8) | (b << 16))
}

/// A key or opener value: command string, list of commands, Lua function,
/// or `{ run = ..., desc = ... }`. `Ok(None)` is `noop`.
fn action(v: &Value, path: &str) -> Result<Option<Binding>, String> {
    let commands = |cmds: Vec<String>, desc: Option<String>| {
        let refs: Vec<&str> = cmds.iter().map(String::as_str).collect();
        Binding::commands(&refs, desc.as_deref())
            .map(Some)
            .map_err(|e| format!("{path}: {e}"))
    };
    match v {
        Value::String(_) => {
            let s = string(v, path)?;
            if s == "noop" {
                Ok(None)
            } else {
                commands(vec![s], None)
            }
        }
        Value::Function(f) => Ok(Some(Binding {
            action: Action::Lua(f.clone()),
            label: "lua function".into(),
        })),
        Value::Table(t) => {
            let run: Value = t.get("run").map_err(|e| format!("{path}: {e}"))?;
            if run.is_nil() {
                let cmds = t
                    .sequence_values::<Value>()
                    .enumerate()
                    .map(|(i, c)| {
                        string(
                            &c.map_err(|e| e.to_string())?,
                            &format!("{path}[{}]", i + 1),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if cmds.is_empty() {
                    return Err(format!(
                        "{path}: expected a command, a list of commands, a function or {{ run = ... }}"
                    ));
                }
                return commands(cmds, None);
            }
            let desc = match t.get::<Value>("desc").map_err(|e| e.to_string())? {
                Value::Nil => None,
                d => Some(string(&d, &format!("{path}.desc"))?),
            };
            let mut b = action(&run, &format!("{path}.run"))?
                .ok_or_else(|| format!("{path}.run: `noop` cannot have a description"))?;
            if let Some(d) = desc {
                b.label = d;
            }
            Ok(Some(b))
        }
        other => Err(format!(
            "{path}: expected a command, a list of commands, a function or {{ run = ... }}, got {}",
            other.type_name()
        )),
    }
}

/// Text used to recognize an unchanged preset binding inside the user's table.
fn fingerprint(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
        Value::Table(t) => {
            let run = fingerprint(&t.get::<Value>("run").ok()?)?;
            let desc = t.get::<Option<String>>("desc").ok()?.unwrap_or_default();
            Some(format!("{run}\u{1}{desc}"))
        }
        _ => None,
    }
}

type Layer = Vec<(Scope, String, Value)>;

fn key_layer(keys: &Value, path: &str) -> Result<Layer, String> {
    let mut out = Vec::new();
    for (scope_name, scope_tbl) in pairs(table(keys, path)?, path)? {
        let scope = Scope::from_name(&scope_name).ok_or_else(|| {
            format!(
                "{path}.{scope_name}: unknown scope (expected {})",
                Scope::NAMES.join(", ")
            )
        })?;
        let spath = format!("{path}.{scope_name}");
        for (keys_text, v) in pairs(table(&scope_tbl, &spath)?, &spath)? {
            out.push((scope, keys_text, v));
        }
    }
    Ok(out)
}

fn entries(layer: Layer) -> Result<Vec<Entry>, String> {
    layer
        .into_iter()
        .map(|(scope, keys, v)| {
            let path = format!("keys.{}[\"{keys}\"]", scope.name());
            Entry::new(scope, &keys, action(&v, &path)?)
                .map_err(|e| format!("keys.{}: {e}", scope.name()))
        })
        .collect()
}

pub fn read(cfg: &Table, preset: &Table) -> Result<Read, String> {
    let mut settings = Settings::default();
    let mut warnings = Vec::new();
    let mut user_keys = Layer::new();
    let mut openers = Vec::new();

    for (key, v) in pairs(cfg, "")? {
        match key.as_str() {
            "show_hidden" => settings.show_hidden = boolean(&v, "show_hidden")?,
            "sort" => {
                for (k, sv) in pairs(table(&v, "sort")?, "sort")? {
                    match k.as_str() {
                        "key" => {
                            let s = string(&sv, "sort.key")?;
                            settings.sort.key = SortKey::from_name(&s).ok_or_else(|| {
                                format!(
                                    "sort.key: expected one of {}, got `{s}`",
                                    SortKey::NAMES.join(", ")
                                )
                            })?;
                        }
                        "reverse" => settings.sort.reverse = boolean(&sv, "sort.reverse")?,
                        _ => warnings.push(format!("unknown setting `sort.{k}`")),
                    }
                }
            }
            "colors" => {
                for (k, cv) in pairs(table(&v, "colors")?, "colors")? {
                    let path = format!("colors.{k}");
                    let target = match k.as_str() {
                        "dir" => &mut settings.colors.dir,
                        "hidden" => &mut settings.colors.hidden,
                        _ => {
                            warnings.push(format!("unknown setting `{path}`"));
                            continue;
                        }
                    };
                    let s = string(&cv, &path)?;
                    *target = parse_color(&s).ok_or_else(|| {
                        format!("{path}: expected a color as \"#rrggbb\", got `{s}`")
                    })?;
                }
            }
            "editor" => {
                let s = string(&v, "editor")?;
                settings.editor = registry::compile(&s).map_err(|e| format!("editor: {e}"))?;
            }
            "keys" => user_keys = key_layer(&v, "keys")?,
            "openers" => {
                for (pattern, ov) in pairs(table(&v, "openers")?, "openers")? {
                    let path = format!("openers[\"{pattern}\"]");
                    let b = action(&ov, &path)?
                        .ok_or_else(|| format!("{path}: `noop` is not an opener"))?;
                    openers.push((pattern, b));
                }
            }
            _ => warnings.push(format!("unknown setting `{key}`")),
        }
    }

    let preset_layer = key_layer(
        &preset.get::<Value>("keys").map_err(|e| e.to_string())?,
        "preset.keys",
    )?;
    let mut unchanged: HashMap<(Scope, Vec<Chord>), String> = HashMap::new();
    for (scope, keys, v) in &preset_layer {
        if let (Ok(seq), Some(fp)) = (parse_seq(keys), fingerprint(v)) {
            unchanged.insert((*scope, seq), fp);
        }
    }
    // Entries identical to the preset belong to the preset layer, not the user's.
    user_keys.retain(|(scope, keys, v)| {
        let same = parse_seq(keys)
            .ok()
            .and_then(|seq| unchanged.get(&(*scope, seq)))
            .is_some_and(|fp| fingerprint(v).as_ref() == Some(fp));
        !same
    });
    let keymap = Keymap::build(entries(preset_layer)?, entries(user_keys)?)?;
    Ok(Read {
        settings,
        keymap,
        openers,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#1a5fb4"), Some(0x00B4_5F1A));
        assert_eq!(parse_color("#FF0000"), Some(0x0000_00FF));
        assert_eq!(parse_color("blue"), None);
        assert_eq!(parse_color("#12345"), None);
    }
}
