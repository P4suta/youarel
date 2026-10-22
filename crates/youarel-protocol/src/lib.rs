//! The messages of youarel's sync in the wire format of `specs/WIRE.md`, and the bytes each signature covers.
//!
//! Every message has one canonical encoding: decoding refuses trailing bytes, unknown tags, lengths beyond their limits, and lists out of order, so a digest or a signature covers exactly one meaning.

pub mod log;
pub mod message;
pub mod relay;
pub mod segment;
pub mod wire;
