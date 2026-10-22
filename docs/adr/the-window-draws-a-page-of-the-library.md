---
status: amended
date: 2026-10-20
amends: window-is-a-library-of-shortcuts.md
amended-by: positions-keep-short-keys.md, the-window-behaves-like-the-finder.md
---
# The window draws a page of the library

## Context

[The record of the library window](window-is-a-library-of-shortcuts.md) answered every intent with the whole view.
A view carried every shortcut of the listing, every entry of every column, and every collection for the pickers.
It also carried the selection as a list of identifiers.
[The record of the indexed store](the-state-is-kept-in-an-indexed-store.md) lets a session read a page of a listing through its indexes.
Yet the window still read whole listings through unlimited pages.
At a million shortcuts a view would carry a million items, and the page would build a million elements.
A select all would send a million identifiers back.
The owner wants the window to stay quick at ten million shortcuts.

## Decision

The library answers every intent with the page it shows.
A view carries one page of the listing that the cards or the list show: up to 200 rows around the place in sight.
It also carries the listing's total, the place of the focused row, and a row to bring into sight after a step.
`page_rules::start` decides where that page starts.
It keeps the page in sight, or starts 50 rows before a row that the view brings into sight from outside that page.
It never starts so late that a long enough listing fills less than a page.
The headings of a grouping travel as the places of their first rows with their counts.
The store reads those counts from its count tables rather than from the rows.
Each column of the walk carries its own page, the place of its selected entry, and its headings.
A key names each listing in the state of the store that the library read it in.
A count of the sessions that changed the store, the store's generation, sets those states apart.

The page asks for more rows as a person scrolls, at most 1,000 rows a call.
It calls `page` for the listing and `column_page` for a column.
A scroll never rebuilds the view, and the library remembers where the page stands, so the next view answers around that place.
The library lays a listing out once for each state of the store, in parts.
A part holds the folders kept in memory, the shortcuts that the indexes page, or the shortcuts that a search or a grouping by site gathers.
A column's part may also hold a folder's entries in their order.
The list's outline lays each open folder out as a part of its own.
A folder of a million shortcuts then opens in place without a read of them.
The library also keeps the rows it read last, so a step reads none of the rows before its place again.

A selection names rows by their places in one listing, so a select all names no row.
`selection_rules` holds it as no rows, a span of places, or every row.
It adds rows and leaves rows out by their identifiers.
The page names the place where it shows each row it selects.
The library checks that the place holds that row before it trusts it.
A selection holds to its listing through a hash of what orders the listing's rows.
Another order of the same rows keeps a select all whole, while a range gives way to the rows it added.
Kani proves that a selection never takes a place past its listing's end, and that its count counts the rows it takes.
Kani also proves that a toggle turns one row alone.

A move or a removal names its target: shortcuts and folders by their identifiers, or the selection.
The library reads the selection's entries once, in the order the window showed them.
It refuses a move of a folder into itself or into a folder that went, before any batch runs.
It then edits in batches of 10,000 entries, one session each, and passes over an entry that went since the job began.
The library's lock goes free between the batches, so the window pages and searches while a job works.
The notice shows how far the job came.

The page draws only the lines in sight, at heights that it counts rather than measures.
A row of the list, a line of cards, a heading, and an entry of a column each take a fixed height.
The script hands those heights to the style sheet.
The cards and each column set every line at its height within a frame of the listing's whole height.
The list keeps a spacer row before and after the rows in sight.
A browser lays out at most 33,554,431 pixels, so a frame takes at most 30,000,000.
A taller listing maps a scroll of its frame onto its lines in proportion, as a million cards need.
A row that the page lacks shows as a placeholder until its page arrives.
The page holds at most 5,000 rows of a listing and lets the rest go.

The collection pickers of the add and edit sheets search the collections by name, at most 200 a call.
A view no longer lists every collection for them.
The terminal launcher's screen holds the texts of one page of rows, and `youarel ls` and `youarel find` take `--limit`.
`ui_rules` and its Quint model keep their old form.

Rejected alternatives:

- A cursor of index keys for each page in place of places.
  The selection, the steps, and the headings all name places, and the store reads the rows before a place faster than a person scrolls.
- A selection by identifiers alone, because selecting a million rows would carry a million identifiers to the page and back.
- A list that measures each row as it draws, because rows of fixed heights let the page place a million rows without a measure.
- A call for rows on each scroll event, because a call per frame would queue sessions faster than the store opens them.

## Consequences

The cost of a view no longer grows with the library, and a scroll costs one call for the rows the page lacks.
A range selection refers to rows by where they stand.
A change from another device between a range selection and a move may then shift what the range takes.
A select all still takes every row.
A card takes a fixed height and its thumbnail a fixed height too, so a wide card shows a wider thumbnail rather than a taller one.
A view still reads the counts of every folder for the sidebar, which grows with the folders, at most 100,000.
The window test in `just app-linux` also scrolls a list of 600 shortcuts to its last row and removes it.
It then removes every row through select all.

The owner's Mac measured the library's calls over a synthetic store of a million recent shortcuts.
The store held a thousand shortcuts in each of a thousand folders, and each call opened its own session.

- The first view took 33 ms, and an intent took 20 to 40 ms.
- A page of 200 rows took 15 to 35 ms near the start of a listing of a million rows.
- A page near the end took 65 to 150 ms, since a page reads the index rows before its place.
- A search through the index of words took 20 to 30 ms.
- Select all took 27 ms, and a step took 27 to 31 ms.
- The first step to the end of the million rows took 168 ms.
- A batch of 10,000 removals took 1.1 to 1.5 s, and a batch of 10,000 moves into one folder took 1.0 to 2.6 s.
- A page between two batches took about 20 ms.

A page of the window waits for the batch in progress, so a scroll during a job may show placeholders for a second or two.
The moves grew slower as they went, because each move to the end of one folder lengthens the position it takes.
A position grows by one symbol for every six or so places at the end of a folder.
A move of a million rows into one folder would write positions thousands of symbols long.
A later change should place a run of entries at once.
[Positions keep short keys](positions-keep-short-keys.md) instead bounds each position by the integers that its first letter names.
