---
status: accepted
date: 2026-10-21
amends: icons-stay-on-the-device.md
---
# Icons are read from the browsers' caches

## Status

Accepted.

## Context

A browser already holds images for many shortcuts imported here or received from another device.
The library should show those images without contacting their sites.
Browsers can keep their databases open and commit newer records to a write-ahead log while this library reads them.
A background reader must leave browsing, editing, and cancellation available.

## Decision

The import adapter reads Chromium-family `Favicons`, Firefox `favicons.sqlite`, and Safari `Favicon Cache/favicons.db`.
Discovery includes Chrome, Chromium, Edge, Brave, Firefox, and Safari through explicit environment facts and platform-specific profile roots.
It inspects at most 256 entries per root and 32 profiles per browser, and a lookup pass admits at most 32 databases in total.
An unavailable or unsupported cache leaves its hosts without an image and reports the failure.
Safari access uses the operating system's existing permission; the reader opens no permission or folder dialog.

The adapter pins rusqlite 0.40.2 with bundled SQLite and the backup, hooks, and limits features.
The source connection uses `SQLITE_OPEN_READ_ONLY`, no shared connection mutex, and the connection's query-only setting.
The [SQLite Backup API](https://sqlite.org/backup.html) produces the private snapshot, including committed log records, through bounded [rusqlite backup steps](https://docs.rs/rusqlite/0.40.2/rusqlite/backup/struct.Backup.html).
The reader performs no source SQL writes or checkpoint, and never copies a live database and its log as independent files.
SQLite owns its file locking and shared-memory coordination.
The temporary directory is private, and dropping the reader closes its database before deleting that directory.

The page-count and page-size admission limits a database to 256 MiB.
A backup copies at most 64 pages per call and makes at most 8,192 calls, with at most three busy or locked results.
No busy timeout or sleep guesses when a browser will release its database.
An unfinished backup returns a failure and drops its partial copy.
Retained snapshots fit 512 MiB in one pass, with at most one additional bounded candidate snapshot while its size is checked.
SQLite virtual-machine progress callbacks check cancellation and at most five million instructions per query.
The connection disables trusted schema, enables defensive mode, caps rows at 1 MiB and SQL text at 64 KiB, and requires the expected objects to be tables.
The library trusts the pinned SQLite implementation for its file format, transaction isolation, native memory safety, and instruction callbacks.
The Rust bounds do not constitute a proof of that C implementation.

Queries join each browser's page-to-icon tables rather than guessing an icon from its own URL.
The Chromium schema follows its [favicon database implementation](https://github.com/chromium/chromium/blob/main/components/favicon/core/favicon_database.cc).
Firefox uses its [Places icon tables](https://github.com/mozilla-firefox/firefox/blob/main/toolkit/components/places/nsPlacesTables.h).
Safari's table relationship and uppercase MD5 image filename follow [Firefox's Safari importer](https://github.com/mozilla-firefox/firefox/blob/main/browser/components/migration/SafariProfileMigrator.sys.mjs).
MD5 serves only that existing filename convention and provides no authentication or content identity.
The filename is generated from the database's UUID text, so that text never becomes a path.
Safari's image file is a separate best-effort read after the database snapshot and can disappear or change while Safari runs.

A query returns at most 16 candidate payloads of at most 512 KiB.
Every matching page is parsed as an admitted web link and its normalized host must equal the requested host.
Candidates prefer the largest bitmap at or below 64 pixels, then the smallest larger bitmap within the decoder's side limit.
Candidate preference applies across readable caches; an exact 64-pixel candidate needs no further search.
SVG-only Firefox records are outside the existing raster decoder and leave the host's tile in place.
Every accepted image passes through the existing normalizer into a 64-pixel PNG before the store keeps it.
An existing host icon stays unchanged, and no image enters the sealed state or a sync transport.

The command `youarel icons` fills missing live hosts through bounded store ranges and local snapshots.
The command-line import follows the same path, and a browser import selects that browser's caches.
The window uses the same readers outside the library lock at startup, after imports, and when a page brings missing icons into sight.
Its pending visible-entry queue holds at most 256 identifiers and takes at most 32 per worker call.
The background scan reads at most 256 records and returns at most 32 missing hosts per range, resuming after the last record inspected.
Only one icon worker runs at a time, cancellation is checked between bounded operations, and an import starts a new snapshot pass.
The pass remembers at most 4,096 attempted hosts and keeps at most 32 diagnostic examples while counting all reported failures.
It performs no scheduled network request.

Rejected alternatives:

- A handwritten SQLite reader, because reproducing its log, overflow, B-tree, corruption, and locking behavior would add a large independent correctness boundary.
- Copying the database and log files, because two copies do not capture one committed transaction boundary.
- Opening the browser's database for writing or checkpointing it, because this library owns neither the browser's state nor its maintenance.
- Network fallback during automatic work, because showing a local shortcut must not reveal it to its site.

## Consequences

The bundled native dependency increases build work and makes SQLite an explicit external trust assumption.
The core still has no dependency or standard library, and owns bitmap preference, snapshot-size admission, and platform-root decisions.
Their source-bound Kani harnesses and negative probe remain pending with the owner's proof window.
The database parser has synthetic committed fixtures and a bounded fuzz target; the fuzz campaign remains pending.
Native tests cover committed log records, uncommitted writes, unchanged source bytes, cancellation, snapshot cleanup, cache failures, profile discovery, store-range continuation, local import fill, and work while the library lock stays held.
Real-browser schema compatibility, supported-platform runtime checks, and window interaction remain pending.
An unsupported, large, busy, or unreadable cache can leave missing icons, and a new explicit pass can retry it.
Large libraries finish background scanning over multiple worker calls; this change makes no measured latency claim.
