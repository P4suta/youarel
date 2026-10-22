//! The window: a library of shortcuts that a global shortcut, the tray, or the Dock brings forward.
//!
//! [`library`] holds the window's side, which the tests drive without a webview, [`intent`] and [`view`] hold what crosses to the page and back, and a private module binds them to Tauri.

pub mod emoji;
pub mod intent;
pub mod library;
mod raw;
pub mod syncing;
pub mod themes;
pub mod view;
pub mod watch;

pub use raw::run;
