//! Command string syntax: `name arg "quoted arg" --flag --key=value`.
//!
//! Inside double quotes `\"` is a literal quote; every other backslash is
//! literal, so Windows paths need no escaping.

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Parsed {
    pub name: String,
    pub positional: Vec<String>,
    pub flags: Vec<(String, Option<String>)>,
}

struct Token {
    text: String,
    /// The first character came from inside quotes, so `"--x"` stays positional.
    quoted_start: bool,
}

fn tokenize(s: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(tokens);
        }
        let mut text = String::new();
        let mut quoted_start = false;
        let mut in_quotes = false;
        while let Some(&c) = chars.peek() {
            if !in_quotes && c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '"' => {
                    if text.is_empty() && !in_quotes {
                        quoted_start = true;
                    }
                    in_quotes = !in_quotes;
                }
                '\\' if in_quotes && chars.peek() == Some(&'"') => {
                    chars.next();
                    text.push('"');
                }
                _ => text.push(c),
            }
        }
        if in_quotes {
            return Err(ParseError(format!("unterminated quote in `{s}`")));
        }
        tokens.push(Token { text, quoted_start });
    }
}

pub fn parse(s: &str) -> Result<Parsed, ParseError> {
    let mut tokens = tokenize(s)?.into_iter();
    let Some(name) = tokens.next() else {
        return Err(ParseError("empty command".into()));
    };
    let mut parsed = Parsed {
        name: name.text,
        ..Parsed::default()
    };
    for t in tokens {
        match t.text.strip_prefix("--") {
            Some(flag) if !t.quoted_start && !flag.is_empty() => match flag.split_once('=') {
                Some((k, v)) => parsed.flags.push((k.to_string(), Some(v.to_string()))),
                None => parsed.flags.push((flag.to_string(), None)),
            },
            _ => parsed.positional.push(t.text),
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let p = parse("sort size --reverse").unwrap();
        assert_eq!(p.name, "sort");
        assert_eq!(p.positional, ["size"]);
        assert_eq!(p.flags, [("reverse".to_string(), None)]);
    }

    #[test]
    fn quoted_path_keeps_backslashes() {
        let p = parse(r#"cd "C:\Program Files""#).unwrap();
        assert_eq!(p.positional, [r"C:\Program Files"]);
    }

    #[test]
    fn escaped_quote() {
        let p = parse(r#"shell "say \"hi\"""#).unwrap();
        assert_eq!(p.positional, [r#"say "hi""#]);
    }

    #[test]
    fn unterminated_quote() {
        let err = parse(r#"cd "C:\Program Files"#).unwrap_err();
        assert!(err.0.contains("unterminated quote"), "{}", err.0);
    }

    #[test]
    fn flag_values() {
        let p = parse(r#"copy --to=other --name="a b" "--literal""#).unwrap();
        assert_eq!(
            p.flags,
            [
                ("to".to_string(), Some("other".to_string())),
                ("name".to_string(), Some("a b".to_string()))
            ]
        );
        assert_eq!(p.positional, ["--literal"]);
    }

    #[test]
    fn whitespace_and_empty() {
        assert_eq!(parse("   up   ").unwrap().name, "up");
        assert!(parse("   ").is_err());
        assert_eq!(parse(r#"cd """#).unwrap().positional, [""]);
    }
}
