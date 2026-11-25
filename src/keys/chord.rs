//! Key chords: a set of modifiers plus a named key or a character, with a
//! canonical text form (`ctrl+shift+f5`, `alt+left`, `J`, `plus`).

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const SHIFT: Mods = Mods(4);
    pub const WIN: Mods = Mods(8);

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn with(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    pub fn without(self, other: Mods) -> Mods {
        Mods(self.0 & !other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Named {
    F(u8),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Space,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Named(Named),
    Char(char),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

impl Chord {
    pub fn new(mods: Mods, key: Key) -> Chord {
        Chord { mods, key }
    }
}

const NAMED: &[(&str, Named)] = &[
    ("enter", Named::Enter),
    ("esc", Named::Esc),
    ("tab", Named::Tab),
    ("backspace", Named::Backspace),
    ("delete", Named::Delete),
    ("insert", Named::Insert),
    ("home", Named::Home),
    ("end", Named::End),
    ("pageup", Named::PageUp),
    ("pagedown", Named::PageDown),
    ("up", Named::Up),
    ("down", Named::Down),
    ("left", Named::Left),
    ("right", Named::Right),
    ("space", Named::Space),
];

/// Accepted alternative spellings, never produced when formatting.
const ALIASES: &[(&str, Named)] = &[
    ("escape", Named::Esc),
    ("return", Named::Enter),
    ("del", Named::Delete),
    ("ins", Named::Insert),
    ("pgup", Named::PageUp),
    ("pgdn", Named::PageDown),
];

fn named_name(n: Named) -> String {
    match n {
        Named::F(i) => format!("f{i}"),
        _ => NAMED.iter().find(|(_, k)| *k == n).unwrap().0.to_string(),
    }
}

fn parse_named(s: &str) -> Option<Named> {
    let lower = s.to_ascii_lowercase();
    if let Some(num) = lower.strip_prefix('f')
        && let Ok(i) = num.parse::<u8>()
        && (1..=24).contains(&i)
    {
        return Some(Named::F(i));
    }
    NAMED
        .iter()
        .chain(ALIASES)
        .find(|(name, _)| *name == lower)
        .map(|(_, k)| *k)
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (m, name) in [
            (Mods::CTRL, "ctrl"),
            (Mods::ALT, "alt"),
            (Mods::SHIFT, "shift"),
            (Mods::WIN, "win"),
        ] {
            if self.mods.contains(m) {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Named(n) => f.write_str(&named_name(n)),
            Key::Char('+') => f.write_str("plus"),
            Key::Char(c) => write!(f, "{c}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChordError(pub String);

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ChordError {}

impl std::str::FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Chord, ChordError> {
        if s == "+" {
            return Ok(Chord::new(Mods::NONE, Key::Char('+')));
        }
        let parts: Vec<&str> = s.split('+').collect();
        let (key_part, mod_parts) = parts.split_last().unwrap();
        let mut mods = Mods::NONE;
        for m in mod_parts {
            mods = mods.with(match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Mods::CTRL,
                "alt" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "win" => Mods::WIN,
                "" => {
                    return Err(ChordError(format!(
                        "empty modifier in `{s}` (write `plus` for +)"
                    )));
                }
                other => return Err(ChordError(format!("unknown modifier `{other}` in `{s}`"))),
            });
        }
        if key_part.is_empty() {
            return Err(ChordError(format!("missing key in `{s}`")));
        }

        let mut chars = key_part.chars();
        let first = chars.next().unwrap();
        let key = if chars.next().is_none() {
            Key::Char(first)
        } else if key_part.eq_ignore_ascii_case("plus") {
            Key::Char('+')
        } else if let Some(n) = parse_named(key_part) {
            Key::Named(n)
        } else {
            return Err(ChordError(format!("unknown key `{key_part}` in `{s}`")));
        };

        normalize_char_chord(mods, key, s)
    }
}

/// Character chords: with ctrl/alt/win the character is a lowercase key and
/// shift stays an explicit modifier; without them shift is folded into the
/// character (`shift+j` is `J`).
fn normalize_char_chord(mods: Mods, key: Key, src: &str) -> Result<Chord, ChordError> {
    let Key::Char(c) = key else {
        return Ok(Chord::new(mods, key));
    };
    let command_mods = mods.without(Mods::SHIFT);
    if !command_mods.is_empty() {
        return Ok(Chord::new(
            mods,
            Key::Char(c.to_lowercase().next().unwrap_or(c)),
        ));
    }
    if mods.contains(Mods::SHIFT) {
        if c.is_ascii_alphabetic() {
            return Ok(Chord::new(Mods::NONE, Key::Char(c.to_ascii_uppercase())));
        }
        return Err(ChordError(format!(
            "`{src}`: shift with `{c}` depends on the keyboard layout; write the shifted character itself"
        )));
    }
    Ok(Chord::new(Mods::NONE, Key::Char(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn round_trip_canonical() {
        assert_eq!(p("Shift+Ctrl+F5").to_string(), "ctrl+shift+f5");
        assert_eq!(p("alt+LEFT").to_string(), "alt+left");
        assert_eq!(p("ctrl+q").to_string(), "ctrl+q");
        assert_eq!(p("Escape").to_string(), "esc");
        assert_eq!(
            p("win+ctrl+alt+shift+n").to_string(),
            "ctrl+alt+shift+win+n"
        );
    }

    #[test]
    fn formatting_is_stable() {
        for s in [
            "ctrl+shift+f5",
            "alt+left",
            "J",
            "j",
            "plus",
            "ctrl+plus",
            "f24",
            "[",
            "backspace",
        ] {
            assert_eq!(p(s).to_string(), s);
        }
    }

    #[test]
    fn unknown_key_is_error() {
        let err = "ctrl+foo".parse::<Chord>().unwrap_err();
        assert!(err.0.contains("`foo`"), "{}", err.0);
        assert!("f25".parse::<Chord>().is_err());
        assert!("hyper+a".parse::<Chord>().is_err());
        assert!("ctrl+".parse::<Chord>().is_err());
    }

    #[test]
    fn char_case_rules() {
        assert_eq!(p("J"), Chord::new(Mods::NONE, Key::Char('J')));
        assert_eq!(p("shift+j"), Chord::new(Mods::NONE, Key::Char('J')));
        assert_eq!(p("ctrl+N"), Chord::new(Mods::CTRL, Key::Char('n')));
        assert_eq!(
            p("ctrl+shift+n"),
            Chord::new(Mods::CTRL.with(Mods::SHIFT), Key::Char('n'))
        );
        assert!("shift+7".parse::<Chord>().is_err());
    }

    #[test]
    fn plus_key() {
        assert_eq!(p("+"), Chord::new(Mods::NONE, Key::Char('+')));
        assert_eq!(p("ctrl+plus"), Chord::new(Mods::CTRL, Key::Char('+')));
        assert!("ctrl++".parse::<Chord>().is_err());
    }
}
