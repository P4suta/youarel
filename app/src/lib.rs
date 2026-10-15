//! The window launcher: a window that a global shortcut or the tray brings up, which opens web shortcuts as the terminal launcher does.
//!
//! [`launcher`] holds the window's side of the shell, which the tests drive without a webview, and a private module binds it to Tauri.

pub mod launcher;
mod raw;

pub use raw::run;
