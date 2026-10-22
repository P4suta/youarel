---
status: proposed
date: 2026-10-18
---
# Notes travel inside the sealed state

## Context

The library window keeps a person's note on a link, such as why it was saved or what to read first, and the note must reach every device of an account.
[Local-first state exchanged as sealed bundles](local-first-state-in-sealed-bundles.md) makes every field of a shortcut a last-writer-wins register.
The owner's design also showed tags, and a first build carried them, but [the window is a library of shortcuts](window-is-a-library-of-shortcuts.md) keeps folders as the one way to group shortcuts.

## Decision

A shortcut holds its note in one register, which the edit sheet and `youarel note` write and an empty note clears.
The search reads each word of a note, so a note also finds its shortcut.
A shortcut's date added is the time its version 7 identifier starts with, so the state gains no field for it.
The time a device last opened a shortcut stays in that device's visits and never leaves it.

The codec keeps format 1 and reads a document without notes as one where no device wrote them.

Rejected alternatives:

- Tags on shortcuts, because a shortcut already lives in one folder, and a second way to group shortcuts asks a person to keep a vocabulary that drifts while the folders and the search already find each link.
- A register over a list of notes, because a note is one text a person edits as a whole.
- A new codec format, because nothing has been released, so every device of an account updates together.

## Consequences

A build from before notes refuses a state that carries a note, since its codec denies unknown fields.
A state written by the first build with tags no longer reads, which touches only stores made before any release.
Two devices that edit one note concurrently keep the later note whole, as every register does.
