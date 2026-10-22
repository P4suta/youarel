---
status: amended
date: 2026-10-19
amends: binary-records-hold-the-state.md, icons-stay-on-the-device.md
amended-by: a-store-reads-only-the-format-it-writes.md, state-travels-as-sealed-segment-chains.md, benchmarks-measure-isolated-stores.md
---
# The state is kept in an indexed store

## Context

The owner chose ten million shortcuts as the scale the library must reach.
The store kept the whole state as one file of records, `state.v2`.
Each session read and decoded that file, then wrote the whole file again after an edit.
The window copied the state for each view and walked every shortcut for the listing, the counts, and the search.
The device kept its visits in `visits.json` and its icons in `icons.json` with their images under `icons/`, and a session read each file whole.
At ten million shortcuts the records alone take one to two gigabytes, so every session would read and write them all.

## Decision

The store keeps the state in one database, `library.redb`, of the embedded engine redb.
redb keeps to pure Rust, depends only on `libc`, and carries licenses that `cargo deny` admits.
Only `fs::open_database` opens the database.
A session takes the store's lock, opens the database, and runs one write transaction.
The transaction commits when the session changed something, and the session drops it otherwise.
Each transaction turns on redb's quick repair, so a process that dies mid-session leaves a database that opens again at once.

The database holds these tables.

- `meta` holds the store's format, 2, its device, and the greatest stamp of any write it holds.
- `devices` lists the devices that the records' stamps name, each at the place it took when the store first met it, and only grows.
- `shortcuts` and `folders` hold each item's record, removed ones included, with its fields as `record_rules` writes them.
- The index tables hold keys alone, whose byte order gives the order a reader wants.
  They take the names `placement`, `by_added`, `by_folder_added`, `by_title`, `by_folder_title`, `by_link`, `by_folder_link`, `by_link_hash`, `by_token`, `by_site`, and `orphans`.
- The count tables hold a count under each key: `home_counts` for the shortcuts directly in each folder, `site_counts`, and `added_per_day`.
- `visits` and `by_opened` hold how often and how lately this device opened each shortcut.
- `icons` holds the image that each host shows and when the device fetched it, and `images` holds the images.
  Neither table ever leaves the device.

The core decides every key.
`index_rules` names the rows that a shortcut or a folder files, and an edit writes only the rows that differ before and after it.
A key starts with the shortcut's home, its folder while that folder lives and the top otherwise, so the rows of one folder lie together.
A shortcut whose folder went away also files a row in `orphans` under that folder, so the folder's return finds it again.
A sort key keeps up to 32 bytes of a folded title or a link, cut at its first zero byte.
A zero byte and the identifier follow, so a shorter text sorts before a longer one that starts with it.
Two shortcuts whose kept starts match take their order from their whole texts, which the store reads only for them.
`by_link_hash` files each shortcut under the 64-bit Fowler–Noll–Vo hash of its link, as `hash_rules` computes it.
The shortcut that holds a link then turns up without a read of every shortcut.
`by_token` files the words of each shortcut's texts, folded and cut to 12 bytes, for the search of a later change.
`edit_rules` decides an edit from the facts that the store gathers about the entries it names, with the reference model's refusals in its order.
It also names the folders that a folder's removal takes, which the store removes in batches.
`page_rules` cuts a page from a listing and merges the listings of a folder's subtree into one.

The folders load whole into memory with their tree, and the store assumes at most `MOST_FOLDERS_IN_MEMORY` of 100,000 folders.
A session answers queries for a page of a scope in an order, an item, the entries of a folder, and the counts.
It also finds the shortcut that holds a link in a folder.
A removed item keeps its record and loses every index row.
`index_rules::purgeable` names a record that stayed removed for 180 days, which a later compaction may delete.
`State::apply` and `State::merge` stay as the reference model.
A differential test applies random edits, merges, and folder removals to the store and to the model.
Every listing, count, and greatest stamp must then agree.
Kani proves the key orders, the kept starts, the homes, the deltas, the pages, and the edits' decisions over small domains.

A store without `library.redb` builds it once from the files of an earlier build: `state.v2` or `state.json`, `device.json`, `visits.json`, and `icons.json` with `icons/`.
It writes the database under a staging name in transactions of 10,000 records and gives the database its name once it holds everything.
It then renames each old file with the ending `.migrated`.
A move cut short starts again from the old files, which stay in place until then.

A spike measured redb on an Apple M5 Pro before the decision.
It loaded a million shortcuts with three indexes in transactions of 10,000 at about 72,000 rows a second into a file of 514 MiB.
A clean open took 3 to 6 ms, and an open with the first page of 200 rows and their records took 4 to 6 ms.
A page of 200 index rows took a median of 3 to 7 µs.
After a `kill -9` in the middle of a load, the database took half a second to open without quick repair and 6 ms with it.
Every committed row survived, and redb's integrity check passed.

Rejected alternatives:

- SQLite through rusqlite with its bundled C library, which stayed the fallback in case the spike failed.
  It brings C into a tree of pure Rust and a query language that ordered keys never need.
- fjall, because its log-structured engine has less history behind it.
- sled, because nobody maintains it.
- The records in one file, read and written whole, because every session then pays for the whole library.

## Consequences

A session reads the folders, the facts about itself, and the rows its work names.
An edit writes one record and its changed rows, except a folder's removal, which writes everything inside the folder.
The window and the terminal launcher still list whole scopes through unlimited pages, so their views keep their cost until a later change pages them.

On the owner's Mac, a store of 88 shortcuts and 26 folders moved into the database during its first command, which took 85 ms in all.
The database takes 520 KB, where the records, the icon index, and 69 images took 219 KB.
The commands listed, found, and exported the same bytes before and after the move, and the window showed the same library.
A command such as `youarel ls` takes about 19 ms where it took 3 ms, and the difference goes to opening and closing the database.
A window's intent runs two or three sessions, and a later change measures whether the window should keep one handle open instead.
The spike's three indexes took 514 MiB for a million shortcuts, and the whole set takes more.
Ten million shortcuts would take about 8 to 9 GB, which shorter kept starts and fewer per-folder indexes can lower.
The database's cache keeps at most 256 MiB of pages in memory, beside the folders.
The text of an edited or removed item can remain in the database's free pages until redb reuses them.
The greatest stamp of one device's writes comes from a read of every record until delta sync keeps it.
No compaction runs yet, so removed items keep their records.
A build from before this change finds no state file and starts empty.
Renaming the `.migrated` files back restores what that build held.
