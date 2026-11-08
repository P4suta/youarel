# Bookmark library

The desktop saves, edits, files, finds, and opens bookmarks in one local list.
Collections form a folder tree.
The command-line and terminal interfaces use the same store.

Every shortcut holds an admitted `http` or `https` URL with a host.
Opening a shortcut repeats admission before reaching the operating system's browser opener.
The [link adapter](../crates/youarel-link/src/lib.rs) owns URL parsing.

A store session holds one process lock and one database transaction.
A failed operation commits no partial edit.
The [store](../crates/youarel-store/src/lib.rs) preserves the existing database format and records.
The desktop ignores retired configuration files and requires no migration.

The desktop supports selection, search, collection navigation, editing, and undo.
Permanent deletion has no undo operation.
The product has no synchronization, device management, import, export, layout selection, or theme configuration.

The [local bookmark library decision](../docs/adr/the-product-is-a-local-bookmark-library.md) defines the product boundary.
The [retired synchronization protocol](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/specs/PROTOCOL.md) preserves the historical synchronization contract.
