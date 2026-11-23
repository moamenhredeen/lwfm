#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cmd;
mod fs;
mod keys;
mod nav;
mod trace;
mod ui;

fn main() {
    trace::init();
    let start = std::env::args_os().nth(1);
    let code = match ui::window::run(start) {
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
