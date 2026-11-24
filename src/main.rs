#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(windows)]
mod locale;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod scroll;
#[cfg(windows)]
mod view;
#[cfg(windows)]
mod window;

mod markdown;
mod syntax;

#[cfg(windows)]
fn main() {
    window::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("mdrilo runs on Windows.");
}
