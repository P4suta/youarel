---
status: accepted
date: 2026-10-21
amends: window-is-a-library-of-shortcuts.md, the-window-draws-a-page-of-the-library.md
---
# The window behaves like the Finder

## Status

Accepted.

## Context

[The library window](window-is-a-library-of-shortcuts.md) presents folders in three layouts.
[Paging](the-window-draws-a-page-of-the-library.md) lets those layouts show a large library without reading it whole.
A person needs familiar selection, navigation, and reversible edits across those layouts.
Other devices may change the same records while this window remains open.

## Decision

Back and forward walk a bounded history of places independently of edit history.
A new visit drops the places ahead.
Show in Folder clears a search, opens the item's containing folder, and selects the item.
Shift selection keeps its original anchor while the focused row moves, including across page boundaries.
An Option disclosure opens or closes every folder below its row.
The active column supports the same ranges, toggles, and select all as the cards and list, including folder entries.
Typed letters select a name through the title index without changing the search.
Letters typed within one second join the prefix, while a later key starts another prefix.
The index seek narrows candidates by prefix and the ordinary scope and order determine the first match.

Cut keeps the selected identifiers until paste or cancellation and dims those entries on every page where they appear.
Paste moves them into the current folder only after validating that destination.
Copy writes admitted shortcut links as text.
A paste without cut entries admits each whitespace-delimited HTTP(S) link into the current folder and reports duplicates and ignored words.
One text paste is one undo step.
The text input is bounded to 16 MiB.

Return edits the name of a single selection and opens the links in a multiple selection.
The Open command admits every link again before it reaches the operating system and records a visit only after the opener succeeds.
Export writes the CLI's Netscape bookmark representation to the operating system's Downloads directory with a timestamp in the name.
Creating the export refuses an existing destination instead of replacing it.
File, Edit, and View expose the same commands as the keyboard.

A drag freezes its entries and admitted links before it starts and carries a fresh local token for internal drops.
Spring loading can then change the place and selection without changing what the drag moves.
The [HTML drag data store](https://html.spec.whatwg.org/multipage/dnd.html#the-drag-data-store) accepts writes during `dragstart`, so the pointer action prepares the native payload before that synchronous event.
The payload carries all selected links as `text/uri-list` and `text/plain`, with a count badge for multiple items.
A gesture whose payload is still pending is refused until preparation completes.
Native delivery to another program remains a platform boundary.

A hover belongs to one drag token and one folder and uses elapsed monotonic time.
`library_rules::spring_due` admits it after 600 ms and refuses cancellations and stale completions.
Leaving the target or ending the drag cancels the hover.
The window validates the destination before spring loading and before every move batch, including whether a trashed ancestor hides it.

The window pins the identifiers of ranges and select all under the store generation that supplied their rows.
A new generation restores those identifiers instead of selecting new rows at their old places.
An unchanged generation keeps the compact selection of places.
The snapshot stays on the device and never travels in a view or a bundle.
Its memory and initial read grow with the selected rows.
Rows that no longer belong to the visible presence are dropped.

A refresh names the first visible entry of the listing and of each column, along with its offset in the viewport.
The store finds that identifier's new place only while the same listing is still shown.
The renderer restores the offset without revealing the focus again.
Column scroll positions, horizontal scroll, and open branches stay where they were.
An anchor whose row went away falls back to its previous place, and one from another scope is ignored.

Undo and redo keep at most 100 local steps, each containing at most 10,000 edits.
A larger operation clears the history.
Every batch of one operation belongs to one step, including completed batches before a later failure.
A new edit drops redo.
An import or permanent deletion clears edit history.
History belongs to the running window and does not sync or survive a restart.

An inverse restores only the field the edit changed.
A move restores its saved home and exact position key through the same admission checks as another move.
Undoing an addition moves its identifier to the trash; redo puts the same identifier back.
The inverse and the repeated edit receive fresh stamps and pass the ordinary store boundary.
An undo may therefore supersede a write that arrived from another device.
A missing item, a destination in the trash, or a newly cyclic destination refuses the whole step and leaves the history cursor in place.
The Edit menu names the next step and delegates undo and redo to a text field while that field has focus.

[The history decisions](../../crates/youarel-core/src/history_rules.rs), [inverse decisions](../../crates/youarel-core/src/undo_rules.rs), and [edit admission](../../crates/youarel-core/src/edit_rules.rs) hold the executable contracts and Kani harnesses.
The obligation manifest declares their bounded domains.
App tests cover batching, identity preservation, fresh stamps, refusal, and retry.

Rejected alternatives:

- Restoring an entire saved record, because that would overwrite unrelated fields another device changed.
- Replaying an addition with a new identifier, because references and later edits must keep naming the same item.
- Reusing an old stamp, because an inverse must participate in the same merge order as any new local edit.
- Restoring a move at the end, because the terminal still exposes the record's manual position.

## Consequences

Undo changes shared state through ordinary edits and can be published normally.
Trash preserves a newly created folder's descendants while the folder is hidden.
An inverse cannot revive an item purged elsewhere.
The memory bound limits retained steps and edits, while the sizes of the edited text values still contribute to memory use.
