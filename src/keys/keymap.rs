//! Keymap: per-scope tries of key sequences -> actions. Lookup walks a scope
//! chain (e.g. search -> list -> global) and reports a match, a pending
//! prefix with hints for the next chord, or nothing.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use super::chord::Chord;
use crate::cmd::registry::{self, CmdError, Invocation};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Scope {
    Global,
    List,
    Search,
    Input,
}

impl Scope {
    pub const NAMES: &'static [&'static str] = &["global", "list", "search", "input"];

    pub fn name(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::List => "list",
            Scope::Search => "search",
            Scope::Input => "input",
        }
    }
}

/// The text is not one of [`Scope::NAMES`].
#[derive(Debug, PartialEq, Eq)]
pub struct UnknownScope;

impl FromStr for Scope {
    type Err = UnknownScope;

    fn from_str(s: &str) -> Result<Scope, UnknownScope> {
        match s {
            "global" => Ok(Scope::Global),
            "list" => Ok(Scope::List),
            "search" => Ok(Scope::Search),
            "input" => Ok(Scope::Input),
            _ => Err(UnknownScope),
        }
    }
}

/// A key text or keymap layer that cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub struct KeymapError(String);

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeymapError {}

pub enum Action {
    /// Command strings compiled at load, run in order until one fails.
    Commands(Vec<Invocation>),
    Lua(mlua::Function),
}

pub struct Binding {
    pub action: Action,
    /// Text shown in the hint bar: the `desc`, or the command text.
    pub label: String,
}

impl Binding {
    pub fn commands(cmds: &[&str], desc: Option<&str>) -> Result<Binding, CmdError> {
        let invs = cmds
            .iter()
            .map(|c| registry::compile(c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Binding {
            action: Action::Commands(invs),
            label: desc.map_or_else(|| cmds.join("; "), str::to_string),
        })
    }
}

/// One binding as written in a config layer. `binding: None` is `noop`.
pub struct Entry {
    pub scope: Scope,
    pub keys: String,
    pub seq: Vec<Chord>,
    pub binding: Option<Binding>,
}

impl Entry {
    pub fn new(scope: Scope, keys: &str, binding: Option<Binding>) -> Result<Entry, KeymapError> {
        Ok(Entry {
            scope,
            keys: keys.to_string(),
            seq: parse_seq(keys)?,
            binding,
        })
    }
}

/// Parses `"ctrl+g d"` into chords.
pub fn parse_seq(keys: &str) -> Result<Vec<Chord>, KeymapError> {
    let seq = keys
        .split_whitespace()
        .map(|c| {
            c.parse::<Chord>()
                .map_err(|e| KeymapError(format!("`{keys}`: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if seq.is_empty() {
        return Err(KeymapError("empty key text".into()));
    }
    Ok(seq)
}

/// Rejects duplicates and prefix conflicts within one layer.
fn check_layer(entries: &[Entry]) -> Result<(), KeymapError> {
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if a.scope != b.scope {
                continue;
            }
            let scope = a.scope.name();
            if a.seq == b.seq {
                return Err(KeymapError(format!(
                    "`{}` and `{}` are the same keys in {scope}",
                    a.keys, b.keys
                )));
            }
            let (short, long) = if a.seq.len() < b.seq.len() {
                (a, b)
            } else {
                (b, a)
            };
            if long.seq.starts_with(&short.seq) {
                return Err(KeymapError(format!(
                    "`{}` is a prefix of `{}` in {scope}; a key sequence cannot also be a binding",
                    short.keys, long.keys
                )));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Node {
    binding: Option<Binding>,
    children: HashMap<Chord, Node>,
}

pub enum Lookup<'a> {
    Match(&'a Binding),
    /// The keys so far start one or more sequences: (next chord, label).
    Prefix(Vec<(String, String)>),
    None,
}

#[derive(Default)]
pub struct Keymap {
    scopes: HashMap<Scope, Node>,
}

impl Keymap {
    /// Merges the preset and user layers: a user binding replaces every
    /// preset binding in the same scope that it equals, extends or prefixes.
    pub fn build(preset: Vec<Entry>, user: Vec<Entry>) -> Result<Keymap, KeymapError> {
        check_layer(&preset)?;
        check_layer(&user)?;
        let mut merged = preset;
        for u in user {
            merged.retain(|p| {
                p.scope != u.scope || !(p.seq.starts_with(&u.seq) || u.seq.starts_with(&p.seq))
            });
            merged.push(u);
        }
        let mut map = Keymap::default();
        for e in merged {
            let Some(binding) = e.binding else { continue };
            let mut node = map.scopes.entry(e.scope).or_default();
            for chord in e.seq {
                node = node.children.entry(chord).or_default();
            }
            node.binding = Some(binding);
        }
        Ok(map)
    }

    pub fn lookup(&self, chain: &[Scope], seq: &[Chord]) -> Lookup<'_> {
        for scope in chain {
            let Some(mut node) = self.scopes.get(scope) else {
                continue;
            };
            let mut found = true;
            for chord in seq {
                if let Some(n) = node.children.get(chord) {
                    node = n;
                } else {
                    found = false;
                    break;
                }
            }
            if !found || seq.is_empty() {
                continue;
            }
            if let Some(b) = &node.binding {
                return Lookup::Match(b);
            }
            let mut hints: Vec<(String, String)> = node
                .children
                .iter()
                .map(|(c, n)| {
                    let label = n
                        .binding
                        .as_ref()
                        .map_or_else(|| "...".to_string(), |b| b.label.clone());
                    (c.to_string(), label)
                })
                .collect();
            hints.sort();
            return Lookup::Prefix(hints);
        }
        Lookup::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(scope: Scope, keys: &str, command: &str) -> Entry {
        let b = (command != "noop").then(|| Binding::commands(&[command], None).unwrap());
        Entry::new(scope, keys, b).unwrap()
    }

    fn seq(s: &str) -> Vec<Chord> {
        parse_seq(s).unwrap()
    }

    fn matched(km: &Keymap, chain: &[Scope], keys: &str) -> Option<String> {
        match km.lookup(chain, &seq(keys)) {
            Lookup::Match(b) => Some(b.label.clone()),
            _ => None,
        }
    }

    const LIST: &[Scope] = &[Scope::List, Scope::Global];
    const SEARCH: &[Scope] = &[Scope::Search, Scope::List, Scope::Global];
    const INPUT: &[Scope] = &[Scope::Input, Scope::Global];

    fn preset() -> Vec<Entry> {
        vec![
            cmd(Scope::Global, "ctrl+q", "quit"),
            cmd(Scope::Global, "ctrl+h", "toggle-hidden"),
            cmd(Scope::List, "backspace", "up"),
            cmd(Scope::Search, "backspace", "search-next"),
        ]
    }

    #[test]
    fn scope_chain() {
        let km = Keymap::build(preset(), vec![]).unwrap();
        assert_eq!(matched(&km, LIST, "backspace").as_deref(), Some("up"));
        assert_eq!(
            matched(&km, SEARCH, "backspace").as_deref(),
            Some("search-next")
        );
        assert_eq!(matched(&km, INPUT, "backspace"), None);
        assert_eq!(matched(&km, INPUT, "ctrl+q").as_deref(), Some("quit"));
    }

    #[test]
    fn sequences_and_hints() {
        let user = vec![
            Entry::new(
                Scope::Global,
                "ctrl+g d",
                Some(Binding::commands(&["cd ~/Downloads"], Some("Downloads")).unwrap()),
            )
            .unwrap(),
            Entry::new(
                Scope::Global,
                "ctrl+g p",
                Some(Binding::commands(&["cd ~/projects"], Some("Projects")).unwrap()),
            )
            .unwrap(),
        ];
        let km = Keymap::build(preset(), user).unwrap();
        match km.lookup(LIST, &seq("ctrl+g")) {
            Lookup::Prefix(h) => assert_eq!(
                h,
                [
                    ("d".into(), "Downloads".into()),
                    ("p".into(), "Projects".into())
                ]
            ),
            _ => panic!("expected prefix"),
        }
        assert_eq!(matched(&km, LIST, "ctrl+g d").as_deref(), Some("Downloads"));
        assert!(matches!(km.lookup(LIST, &seq("ctrl+g x")), Lookup::None));
    }

    #[test]
    fn user_overrides_and_noop() {
        let user = vec![
            cmd(Scope::Global, "ctrl+h", "refresh"),
            cmd(Scope::List, "backspace", "noop"),
        ];
        let km = Keymap::build(preset(), user).unwrap();
        assert_eq!(matched(&km, LIST, "ctrl+h").as_deref(), Some("refresh"));
        assert_eq!(matched(&km, LIST, "backspace"), None);
    }

    #[test]
    fn user_shadows_conflicting_preset_sequences() {
        let user = vec![cmd(Scope::Global, "ctrl+h d", "cd ~/Downloads")];
        let km = Keymap::build(preset(), user).unwrap();
        assert!(matches!(km.lookup(LIST, &seq("ctrl+h")), Lookup::Prefix(_)));
        let user = vec![cmd(Scope::Global, "ctrl+q", "refresh")];
        let km = Keymap::build(vec![cmd(Scope::Global, "ctrl+q x", "quit")], user).unwrap();
        assert_eq!(matched(&km, LIST, "ctrl+q").as_deref(), Some("refresh"));
    }

    #[test]
    fn conflicts_within_user_layer() {
        let dup = Keymap::build(
            vec![],
            vec![
                cmd(Scope::List, "ctrl+x", "up"),
                cmd(Scope::List, "Ctrl+X", "back"),
            ],
        );
        assert!(dup.err().unwrap().to_string().contains("same keys"));
        let prefix = Keymap::build(
            vec![],
            vec![
                cmd(Scope::Global, "ctrl+g", "up"),
                cmd(Scope::Global, "ctrl+g d", "back"),
            ],
        );
        let err = prefix.err().unwrap();
        assert!(
            err.to_string()
                .contains("`ctrl+g` is a prefix of `ctrl+g d`"),
            "{err}"
        );
        // Same keys in different scopes are fine.
        assert!(
            Keymap::build(
                vec![],
                vec![
                    cmd(Scope::List, "ctrl+x", "up"),
                    cmd(Scope::Input, "ctrl+x", "back")
                ]
            )
            .is_ok()
        );
    }

    #[test]
    fn invalid_keys_and_commands() {
        assert!(Entry::new(Scope::List, "ctrl+foo", None).is_err());
        assert!(Binding::commands(&["cpoy"], None).is_err());
        assert!(parse_seq("  ").is_err());
    }
}
