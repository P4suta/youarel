---
status: proposed
date: 2026-10-18
---
# Browser bookmarks are read from their files

## Context

The owner's design imports the bookmarks of Safari, Chrome, and Firefox with one button each, beside the import of an exported HTML file.
Each browser keeps its bookmarks in a file of its own format under a profile directory whose place depends on the host and its variables.
Chrome keeps JSON, Safari keeps a property list that macOS hides from an app without Full Disk Access, and Firefox keeps a SQLite database beside compressed JSON backups of it.
A browser's file comes from outside youarel and may be large, damaged, or written while youarel reads it.

## Decision

`browser_rules` decides where each browser keeps its bookmarks on each host, from the variables and the host it receives, with Kani harnesses that each place stands on the first usable variable.
The crate `youarel-import` reads Chrome's JSON, Firefox's newest `bookmarkbackups` file in the mozLz4 framing through `lz4_flex`, and Safari's property list through `plist`, each within a bound on its size, into the bookmark tree that an HTML import already produces.
The import then merges as an HTML import does: folders by name, and a link youarel already holds once.
youarel opens a browser's files only to read them, and never writes or locks them.

The window offers each browser for which `browser_rules` decides a place on this host, and the CLI takes `youarel import --from safari|chrome|firefox`.
An import from a browser whose file is missing says that the browser keeps no bookmarks on this device.
A refused read of Safari's file names Full Disk Access and the HTML export.

A file a person hands over may be any of these formats, such as the `Bookmarks` file of Edge or Brave copied from another device.
`browser_rules::kind` names its reader from its first 512 bytes, with a Kani harness over every file of eight bytes.
A magic number names a Firefox backup or a binary property list.
A JSON object names Chromium's file, an XML property list names Safari's, and anything else reads as HTML.
The window's file button takes such a file, and the CLI takes `youarel import FILE --folder PATH` to land it under a folder.

Rejected alternatives:

- Reading Firefox's `places.sqlite`, because a SQLite engine would join the release for one import, and the browser locks the database while it runs.
- Asking each browser through an extension, because youarel then ships and maintains an extension per browser for a one-time import.
- Only the HTML import, because the design offers each browser by name and an export takes several steps in every browser.

## Consequences

A Firefox import holds what its newest backup holds, which Firefox writes about once a day, so the newest bookmarks may wait for the next backup.
Chromium's other browsers keep the same JSON in other places, which `browser_rules` may name later.
Until then their files import as files a person hands over.
A file of another format that opens like JSON or a property list fails in that reader rather than in the HTML one.
The fuzz target `browser_bookmarks` owns the three readers with seeds of each format.
