---
status: proposed
date: 2026-10-20
amends: the-window-draws-a-page-of-the-library.md
---
# Positions keep short keys

## Context

Each entry of a folder holds its position as a key that orders as its text, so an entry moves by one write and its siblings keep their keys.
`position_rules` chose every key by bisection, and a key after the last one was the midpoint between that key and the open end.
That midpoint lengthens the key by one symbol for every six or so entries at the end of a folder.
[The record of the paged window](the-window-draws-a-page-of-the-library.md) saw moves into one folder slow down as they went, and left this growth open.
On the owner's Mac, an import of 100,000 links into one folder took 26 seconds.
It wrote keys of up to 16,667 symbols, 833 million bytes of positions in all, and a store of 3,379 MiB.
Positions had no record of their own until this one.

## Decision

A key keeps its form: one or more of the sixty-two symbols `0` to `9`, `A` to `Z`, and `a` to `z`, whose last symbol is not `0`.
Keys order as their bytes, and every stored key keeps its place.
A key that starts with a letter reads as an integer and a fraction.
The letter names the integer's class: `a` to `z` take one to twenty-six symbols of integer after the letter, and `Z` down to `A` take one to twenty-six.
A longer integer thus sorts after a shorter one among the small letters and before it among the capitals, so integers order as their keys.

A folder's first key is `a1`.
A key after the last one is the least integer above it, and never one below `a1`.
A key before the first one is the greatest integer below it, and never one above `Zz`, the integer just before `a1`.
A key between two others stays their midpoint, symbol by symbol.
The midpoint also places a key where no integer lies past its bound: after the last integer, `z` and twenty-six symbols `z`, and before the first, `A`, twenty-five symbols `0`, and `1`, which every key that starts with a decimal digit precedes.
An integer that would end in `0` steps once more, so every key keeps the form that builds before this change read.

A stored key goes on from the integer beside it.
A key shorter than its class goes on from the least integer that starts with it, so `zV` goes on from `zV`, twenty-four zeros, and `1`.
A key below `a1` goes on from `a1`, and a key above `Zz` takes `Zz` before it.

Kani proves what `verification/obligations.toml` states over the classes of up to three digits.
The integer after an integer and the integer before it are keys on either side of it, at most one digit longer.
The least integer above a key of up to three symbols, and the greatest integer below one, lie on the right side of it and grow by one symbol at most past the longer of the key and its class.
The least integer above a key shorter than its class takes the class's length and passes over no integer above the key.
`between` hands two bounds to the midpoint, which Kani proves over keys of up to two symbols in either order, one bound to the integer beside it or to `a1` or `Zz`, and none to `a1`.
The tests sweep every pair of bounds of up to four symbols over the ends of the classes.
They also run a million appends from `a1`, a hundred thousand prepends before it, and a hundred thousand appends after each of `V`, `zV`, and `0V`, and step through every integer of the classes of one and two digits.
The fuzz target `position_key` checks on keys of any length that a key past either end grows by one symbol at most past the longer of its bound and the longest integer.

Rejected alternatives:

- Integers that end in `0`, as other schemes of fractional indexing write them.
  A key that ends in `0` leaves no key between it and the key that it extends, such as `a` and `a0`.
  A build from before this change also refuses such a key, so a bundle from an updated device would fail on a device that has not updated.
- Keys for a run of entries at once, as the record of the paged window proposed.
  They keep a run short only when the run arrives whole, while an add, an import, a move in batches, and a sync each place one entry at a time.
- Renumbering a folder once its keys grow long, because every renumbered sibling becomes a write that every device must merge.
- Integer positions without classes, because an integer of a fixed length runs out, and one of any length orders differently from its text.

## Consequences

An import of 100,000 links into one folder now takes 3.4 seconds on the owner's Mac and writes keys of at most four symbols, 396 thousand bytes of positions, and a store of 129 MiB.
A million appends from `a1` write keys of at most five symbols, and n appends write keys of about the base-62 logarithm of n symbols past their class letter.
An append after a stored key that starts with a small letter takes as many symbols as that letter's class, twenty-seven for a key that starts with `z`, and the appends after it grow only when the class runs out.
A prepend below a stored key that starts with a decimal digit still lengthens by one symbol for every six or so, and this build writes such a key only before the first integer.
A run of insertions between two fixed neighbors still lengthens by one symbol for every six or so, as any bisection does.
No stored position changes, and a build from before this change reads every key that this one writes.

The jobs of the window were measured again over a synthetic store of a million shortcuts, a thousand in each of a thousand folders.
A batch of 10,000 moves into a folder that already held 400,000 shortcuts held the store's lock for 0.54 to 0.57 seconds, and the batches no longer slowed as the folder grew.
The edits of a batch take about 30 microseconds a move and 50 a removal, and the commit of every page that the batch touched takes the rest of its hold.
Over 100,000 entries, batches of 10,000 held the lock for 0.71 seconds at the median for moves and 1 second for removals.
Batches of 5,000 held it for 0.45 and 0.53 seconds at the median and at most 0.89 and 0.96 seconds.
Batches of 2,500 held it for 0.28 and 0.41 seconds at the median and at most 0.37 and 0.64 seconds, while the moves took 30 percent longer and the removals 52 percent.
Since no smaller batch keeps every hold under about 0.3 seconds without slowing the job, `BATCH` stays 10,000.
