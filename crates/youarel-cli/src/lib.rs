//! The shell of `youarel`: the commands, the terminal launcher, and the effects they carry out for the core.
//!
//! The binary in `main.rs` parses its arguments and calls in here.

pub mod clock;
pub mod commands;
pub mod home;
pub mod icons;
pub mod resolve;
pub mod shell;
pub mod sync;
pub mod tui;
pub mod update;
pub mod web;
pub mod window;
