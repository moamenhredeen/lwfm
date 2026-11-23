//! Keymap: (scope, chord) -> command. Lookup walks from the focused scope up
//! to `Global`. The default table is shaped like the future Lua keymap.

use std::collections::HashMap;

use super::chord::Chord;
use crate::cmd::registry::{self, Invocation};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Scope {
    Global,
    List,
    Input,
}

impl Scope {
    fn parent(self) -> Option<Scope> {
        match self {
            Scope::List | Scope::Input => Some(Scope::Global),
            Scope::Global => None,
        }
    }
}

pub const DEFAULT_KEYMAP: &[(Scope, &str, &str)] = &[
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
    (Scope::List, "enter", "open"),
    (Scope::List, "backspace", "up"),
    (Scope::Input, "enter", "path-submit"),
    (Scope::Input, "esc", "path-cancel"),
];

#[derive(Default, Debug)]
pub struct Keymap {
    bindings: HashMap<(Scope, Chord), Invocation>,
}

impl Keymap {
    /// Builds a keymap from (scope, chord text, command string) entries,
    /// failing on the first invalid chord, invalid command or duplicate.
    pub fn from_table(table: &[(Scope, &str, &str)]) -> Result<Keymap, String> {
        let mut bindings = HashMap::new();
        for &(scope, chord, command) in table {
            let parsed: Chord = chord.parse().map_err(|e| format!("{e}"))?;
            let inv = registry::compile(command).map_err(|e| format!("`{chord}`: {e}"))?;
            if bindings.insert((scope, parsed), inv).is_some() {
                return Err(format!("`{parsed}` is bound twice in {scope:?}"));
            }
        }
        Ok(Keymap { bindings })
    }

    pub fn default_map() -> Keymap {
        Keymap::from_table(DEFAULT_KEYMAP).expect("default keymap is valid")
    }

    pub fn lookup(&self, scope: Scope, chord: &Chord) -> Option<&Invocation> {
        let mut s = Some(scope);
        while let Some(scope) = s {
            if let Some(inv) = self.bindings.get(&(scope, *chord)) {
                return Some(inv);
            }
            s = scope.parent();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(km: &Keymap, scope: Scope, chord: &str) -> Option<&'static str> {
        km.lookup(scope, &chord.parse().unwrap())
            .map(|i| i.def.name)
    }

    #[test]
    fn default_keymap_parses_and_resolves() {
        let km = Keymap::default_map();
        assert_eq!(name(&km, Scope::List, "backspace"), Some("up"));
        assert_eq!(name(&km, Scope::Input, "backspace"), None);
        assert_eq!(name(&km, Scope::Input, "ctrl+q"), Some("quit"));
        assert_eq!(name(&km, Scope::List, "enter"), Some("open"));
        assert_eq!(name(&km, Scope::Input, "enter"), Some("path-submit"));
        assert_eq!(name(&km, Scope::List, "alt+left"), Some("back"));
        assert_eq!(name(&km, Scope::List, "down"), None);
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(Keymap::from_table(&[(Scope::List, "ctrl+foo", "up")]).is_err());
        assert!(Keymap::from_table(&[(Scope::List, "x", "cpoy")]).is_err());
        let dup = Keymap::from_table(&[
            (Scope::List, "ctrl+x", "up"),
            (Scope::List, "Ctrl+X", "back"),
        ]);
        assert!(dup.unwrap_err().contains("bound twice"));
    }
}
