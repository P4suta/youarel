//! The shell of `youarel`: the commands, the terminal launcher, and the effects they carry out for the core.
//!
//! The binary in `main.rs` parses its arguments and calls in here.

pub mod clock;
pub mod commands;
pub mod home;
pub mod resolve;
pub mod tui;
pub mod web;
