---
status: proposed
date: 2026-10-19
---
# Binary records hold the state

## Context

The store kept the state as one JSON document, and every bundle sealed the same document.
Each register wrote its stamp in full, with a wall time, a counter, and a device identifier of thirty-two hexadecimal digits.
A shortcut cost about a kilobyte that way.
The owner chose ten million shortcuts as the scale the library must reach, which later changes approach with an indexed store, paged views, and delta sync.
A delta sync chains its segments by the digests of their plaintexts, which needs one encoding for each state.

## Decision

Binary records hold the state.
They open with the magic `YRR1` and a version, then a table of the devices whose stamps they write.
Every folder and every shortcut follow, each list in ascending order of the items' identifiers, with no identifier twice.
A stamp names its device by its place in the table.
An item's first register writes its stamp in full as the item's base.
Each other register spends two bits on its mode: the base's stamp, a stamp of its own, or the stamp that no device wrote.
A stamp of its own writes its wall time as the zigzag difference from the base's, so a write moments later costs a byte or two.
Counts, lengths, and the parts of stamps take the shortest form of a LEB128 varint.
A flag byte says which optional fields a record holds, so an empty alias, note, or mark costs nothing.

The reader refuses every form but the writer's own.
It refuses a longer varint than the shortest, a reserved flag or mode, and an empty text that a flag announces.
It also refuses a stamp of its own that equals the base, a device that no stamp names, and items out of order.
One state then has one encoding, which a digest of the records stands for.
The wire format in [`specs/WIRE.md`](../../specs/WIRE.md) gives the layout.

The store keeps the records in `state.v2`, and the plaintext of a bundle and the state of an entry hold the same records.
A store that holds only `state.json` reads it once, writes the records, and keeps the document as `state.json.migrated`.
A device still reads the JSON form that a build from before the records sends, so two devices that update one after the other converge.

Kani proves the varints over every 64-bit value, every stamp beside every base, the one form of a byte of modes, and the order of a body's lists.
Tests sweep whole records over every flag and every mode.
The fuzz target `record_decode` requires every body it reads to write the same bytes again.

Rejected alternatives:

- A general binary format such as CBOR, MessagePack, or Protocol Buffers, because each lets one value take more than one encoding, and a stamp for each register would still cost its full width.
- A compressed JSON document, because a device would still decode the whole larger form, and compression leaves no single form to digest.
- The JSON document beside the records, because two forms of one state would drift apart.

## Consequences

On the owner's Mac, the records of 90 shortcuts and 26 folders take 10,470 bytes, against 96,757 bytes of compact JSON without the icons.
Its bundle went from 491,174 bytes to 43,945 bytes once its window synced, and the account's signed log makes most of what remains.
A shortcut costs about 100 to 200 bytes, by the lengths of its link and title, so ten million shortcuts still take one to two gigabytes.
That passes the 16 MiB of a bundle's state and the relay's quota of 64 MiB, which delta sync has to address.
A person reads the state through `youarel export`, since its file no longer holds text.
A build from before the records refuses a bundle from an updated device until it updates too.
Kani proves the parts of a record rather than a whole record.
A whole record with a choice of flags or of modes, even with texts of one byte, took the solver past the proof gate's 300 seconds for a harness, and wider ones past its memory.
