#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod cmd;
mod config;
mod fs;
mod keys;
mod lua;
mod nav;
mod search;
mod shell;
mod trace;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

use config::discover::{self, Env, Found};

struct Args {
    dir: Option<OsString>,
    config: Option<PathBuf>,
    check_config: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        dir: None,
        config: None,
        check_config: false,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--config") => args.config = it.next().map(PathBuf::from),
            Some("--check-config") => args.check_config = true,
            _ => args.dir = Some(a),
        }
    }
    args
}

fn load_file(path: &Path) -> Result<config::Loaded, config::ConfigError> {
    config::read_file(path).and_then(|src| config::load(Some(path), Some(&src)))
}

/// Loads the config before the window exists. Problems never stop startup:
/// they fall back to the defaults and come back as a status message.
fn load_config(env: &Env) -> (config::Loaded, Option<PathBuf>, Option<String>) {
    let started = Instant::now();
    let result = match discover::discover(env, &|p| p.is_file()) {
        Found::File(p) => match load_file(&p) {
            Ok(l) => {
                let msg = (!l.warnings.is_empty())
                    .then(|| format!("Config warnings: {}", l.warnings.join("; ")));
                (l, Some(p), msg)
            }
            Err(e) => (config::defaults(), Some(p), Some(e.to_string())),
        },
        Found::Missing(p) => (
            config::defaults(),
            None,
            Some(format!("Config not found: {}", p.display())),
        ),
        Found::None => (config::defaults(), None, None),
    };
    trace::log(|| {
        let at = result
            .1
            .as_ref()
            .map_or("defaults".into(), |p| p.display().to_string());
        format!("config loaded in {} ({at})", trace::ms(started.elapsed()))
    });
    result
}

/// `lwfm --check-config`: validate and report, no window.
fn check_config(env: &Env) -> i32 {
    lwfm_win::sys::attach_parent_console();
    match discover::discover(env, &|p| p.is_file()) {
        Found::File(p) => match load_file(&p) {
            Ok(l) => {
                println!("config OK: {}", p.display());
                for w in &l.warnings {
                    println!("warning: {w}");
                }
                0
            }
            Err(e) => {
                println!("config error: {e}");
                1
            }
        },
        Found::Missing(p) => {
            println!("config not found: {}", p.display());
            1
        }
        Found::None => {
            println!("no config file, using defaults");
            0
        }
    }
}

fn main() {
    trace::init();
    let args = parse_args();
    let env = Env::from_process(args.config);
    if args.check_config {
        std::process::exit(check_config(&env));
    }
    let (config, config_file, message) = load_config(&env);
    let startup = app::Startup {
        dir: args.dir,
        config,
        config_file,
        env,
        message,
    };
    let code = match app::run(startup) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("lwfm: {e}");
            1
        }
    };
    // Exit right away: enumeration threads blocked on slow storage must not
    // keep the process alive.
    std::process::exit(code);
}
