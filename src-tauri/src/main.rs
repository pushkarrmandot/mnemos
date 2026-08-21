// Keeps a console window from flashing on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mnemos_tauri_lib::run();
}
