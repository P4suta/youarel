# Threat model

Bookmarks, notes, visits, and icons remain in the local database.
The product has no account, synchronization service, shared store, or recovery-key workflow.
Protection from a person who can read or replace local files belongs to the operating system.

Stored links remain untrusted at opening.
The [link adapter](../crates/youarel-link/src/lib.rs) admits only HTTP and HTTPS URLs with a host.
The [browser adapter](../crates/youarel-cli/src/web.rs) repeats admission before opening a stored link.

The app treats page titles and icons as untrusted network input.
Fetches have time and size bounds.
The desktop renders bookmark text as text rather than markup.
The [icon adapter](../crates/youarel-icon/src/lib.rs) decodes bounded images and redraws them before the desktop displays them.

Program updates require the [release verifier](../crates/youarel-release/src/trust.rs).
Both signatures and the selected artifact's digest must verify.
The release hold remains independent of update verification.

The [retired threat model](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/specs/THREAT_MODEL.md) describes the removed synchronization system.
