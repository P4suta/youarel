//! The pure decision core of youarel.
//!
//! Every fact arrives as a value and every decision leaves as a value.
//! The crate reaches no file, process, network, clock, or environment, and `cargo xtask structure` refuses a dependency, a `std` path, or a weakened deny list.
#![no_std]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::exit,
    clippy::mem_forget
)]

extern crate alloc;

pub mod browser_rules;
pub mod clock_rules;
pub mod folder_rules;
pub mod home_rules;
pub mod host_rules;
pub mod library_rules;
pub mod link_rules;
pub mod mark_rules;
pub mod match_rules;
pub mod membership_rules;
pub mod merge_rules;
pub mod position_rules;
pub mod rank_rules;
pub mod record_rules;
pub mod relay_rules;
pub mod state;
pub mod sync_rules;
pub mod theme_rules;
pub mod tree_rules;
pub mod ui_rules;
pub mod update_rules;
pub mod verification_rules;
pub mod view;
pub mod wire_rules;
