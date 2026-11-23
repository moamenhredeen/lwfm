//! Finding the config file: `--config`, `LWFM_CONFIG`, then the dotfile and
//! AppData locations.

use std::path::{Path, PathBuf};

/// The inputs of the lookup, captured once so the logic stays testable.
#[derive(Default, Clone, Debug)]
pub struct Env {
    pub explicit: Option<PathBuf>,
    pub env_var: Option<PathBuf>,
    pub userprofile: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
}

impl Env {
    pub fn from_process(explicit: Option<PathBuf>) -> Env {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Env {
            explicit,
            env_var: var("LWFM_CONFIG"),
            userprofile: var("USERPROFILE"),
            appdata: var("APPDATA"),
        }
    }

    /// Where `edit-config` creates a starter config.
    pub fn default_location(&self) -> Option<PathBuf> {
        self.appdata
            .as_ref()
            .map(|a| a.join("lwfm").join("config.lua"))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Found {
    File(PathBuf),
    /// `--config` or `LWFM_CONFIG` named a file that does not exist.
    Missing(PathBuf),
    None,
}

pub fn discover(env: &Env, exists: &dyn Fn(&Path) -> bool) -> Found {
    if let Some(named) = env.explicit.as_ref().or(env.env_var.as_ref()) {
        return if exists(named) {
            Found::File(named.clone())
        } else {
            Found::Missing(named.clone())
        };
    }
    let candidates = [
        env.userprofile
            .as_ref()
            .map(|u| u.join(".config").join("lwfm").join("config.lua")),
        env.default_location(),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| exists(p))
        .map_or(Found::None, Found::File)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            explicit: None,
            env_var: None,
            userprofile: Some(r"C:\Users\me".into()),
            appdata: Some(r"C:\Users\me\AppData\Roaming".into()),
        }
    }

    const DOT: &str = r"C:\Users\me\.config\lwfm\config.lua";
    const APP: &str = r"C:\Users\me\AppData\Roaming\lwfm\config.lua";

    fn with(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p| files.iter().any(|f| Path::new(f) == p)
    }

    #[test]
    fn explicit_path_wins() {
        let e = Env {
            explicit: Some(r"D:\my.lua".into()),
            ..env()
        };
        assert_eq!(
            discover(&e, &with(&[APP, r"D:\my.lua"])),
            Found::File(r"D:\my.lua".into())
        );
    }

    #[test]
    fn env_var_before_locations() {
        let e = Env {
            env_var: Some(r"D:\env.lua".into()),
            ..env()
        };
        assert_eq!(
            discover(&e, &with(&[DOT, r"D:\env.lua"])),
            Found::File(r"D:\env.lua".into())
        );
    }

    #[test]
    fn dotfile_before_appdata() {
        assert_eq!(
            discover(&env(), &with(&[DOT, APP])),
            Found::File(DOT.into())
        );
        assert_eq!(discover(&env(), &with(&[APP])), Found::File(APP.into()));
    }

    #[test]
    fn nothing_found() {
        assert_eq!(discover(&env(), &with(&[])), Found::None);
    }

    #[test]
    fn missing_explicit_file() {
        let e = Env {
            explicit: Some(r"D:\missing.lua".into()),
            ..env()
        };
        assert_eq!(
            discover(&e, &with(&[APP])),
            Found::Missing(r"D:\missing.lua".into())
        );
    }

    #[test]
    fn default_location_is_appdata() {
        assert_eq!(env().default_location().unwrap(), Path::new(APP));
    }
}
