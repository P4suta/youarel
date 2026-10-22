---
status: proposed
date: 2026-10-20
amends: binary-records-hold-the-state.md, icons-stay-on-the-device.md, the-state-is-kept-in-an-indexed-store.md
---
# A store reads only the format it writes

## Context

[The state is kept in an indexed store](the-state-is-kept-in-an-indexed-store.md) built the database once from the files of earlier builds.
The files of earlier builds comprised `state.v2` or `state.json`, `device.json`, `visits.json`, and `icons.json` with the images under `icons/`.
[Binary records hold the state](binary-records-hold-the-state.md) kept a reader of the state's JSON form, for a store's `state.json` and for the bundles and entries of builds from before the records.
[Icons stay on the device](icons-stay-on-the-device.md) moved the icons that such a document named into the device's own cache.
No build of youarel has shipped, so those files existed only on the owner's two devices, and both stores moved into the database.
Each reader still kept a parser alive with its fuzz target, its corpus, and its tests.
The JSON form also stayed a second encoding of the state beside the records.

## Decision

A store keeps its state, its device, its visits, and its icons only in the database it writes, `library.redb` of format 2, and reads no earlier form of them.
A store without that database makes an empty one under a staging name, with a new device and the format, and then gives it its name.
The making touches no file of the directory but the database and its staging file.
A device reads the state of a bundle or an entry only as binary records, and refuses any other bytes as a malformed state.

Rejected alternatives:

- Keeping the migration until the first release, because no store needs it, and each of its readers would keep its fuzz target, its corpus, and its mutation tests.
- Keeping the JSON reader for received states alone, because no device sends that form, and two forms of one state drift apart, as the record of the binary records says.
- Refusing to open a directory that still holds the state of an earlier build, because the store would then keep the names of files that no store holds.

## Consequences

A directory that a build from before the database left opens empty under this build, beside the files it ignores.
A build that still holds the migration has to move such a directory first.
A device on a build from before the records can no longer send its state to an updated device.
The fuzz targets `state_decode` and `icon_index_decode` left with the readers they covered, and `record_decode` reads the records alone.
The fuzz target `icon_name` covers the parser of an image's name, which the reader of the icon index covered before.
