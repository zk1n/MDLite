#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod native;

fn main() {
    #[cfg(windows)]
    if let Err(error) = native::run() {
        eprintln!("MDLite: {error}");
    }
}
