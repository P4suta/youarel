//! `youarel-launcher`: a window that a global shortcut or the tray brings up, which opens web shortcuts as the terminal launcher does.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    youarel_launcher::run()
}
