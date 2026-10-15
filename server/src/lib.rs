//! The relay as a Cloudflare Worker.
//!
//! The Worker routes a member's request to its account's Durable Object and a pairing's mail to the mailboxes' Durable Object, and each object only adapts `youarel-relay` to its SQLite database.
//! A member that listens opens a WebSocket, sends a signed collection as its first message, and then hears of every bundle another member publishes.

mod raw;
mod store;
