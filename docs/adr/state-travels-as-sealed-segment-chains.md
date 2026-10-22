---
status: amended
date: 2026-10-21
amended-by: devices-join-from-a-screen.md
amends: local-first-state-in-sealed-bundles.md, the-state-is-kept-in-an-indexed-store.md
supersedes: relay-keeps-each-members-newest-bundle.md
---
# State travels as sealed segment chains

## Status

Accepted.

## Context

[Whole-state bundles](local-first-state-in-sealed-bundles.md) make the cost of publishing one edit grow with the library.
The [indexed store](the-state-is-kept-in-an-indexed-store.md) can read and merge records without holding the whole library in memory.
A publication can fail after a peer sees its bytes and before the writer records its success.
Other local edits can arrive during that delivery.

## Decision

Each device owns an increasing chain of signed, sealed segments.
A segment holds complete records for a subset of the state, with the same registers and stamps as a snapshot.
The sender, epoch, sequence, previous plaintext digest, and snapshot part information are authenticated by both the envelope signature and the seal's associated data.
The deterministic plaintext digest uses `youarel/v1/segment` and does not change when sealing chooses another nonce.

A delta includes exactly the items that still hold a new write of this device after its committed watermark.
The stamp that no device wrote never qualifies.
Each selected item carries its complete record so an out-of-order creation retains an admitted link and every field needed for the ordinary register join.
Forwarded fields keep their original stamps.
Receiving another device's writes does not make this device publish again.

The store records dirty items in the same transaction as their edits.
It reserves immutable bodies, sequences, and a watermark before delivery.
Each sealed envelope is saved before the transport can see it, so retry uses exactly the same bytes.
Any required key rotation happens before the membership count is captured, so the publication acknowledges the rotation it carries.
Only a successful delivery of every envelope and the transport head commits the watermark.
Dirty entries after that watermark remain for the next publication.

A reader authenticates each envelope and admits every link before merging its records.
Its cursor and record merges commit in one database transaction.
A repeated plaintext leaves the cursor unchanged, a conflicting plaintext at the same sequence refuses, and a missing predecessor stops the chain.
An advertised complete snapshot can restart after a gap.
A transport that no longer holds the needed prefix or a complete snapshot reports that another bootstrap source is needed.

Snapshot parts target 8 MiB and never split a record.
The wire reader admits at most 16 MiB of plaintext per segment.
Packing uses the canonical record bytes and the exact device dictionary, including changes in varint widths.
The stream keeps one part in memory and stages completed parts in the database.
An individual record that cannot fit refuses the reservation without discarding records from the store.

Compaction becomes due after 256 deltas or when their cumulative body bytes reach twice the preceding snapshot's body bytes.
Automatic compaction accompanies the next pending change, so crossing a threshold never wakes an unchanged usable chain.
These parameters can change after measurements without changing merge semantics.
A different target ordinarily starts with a snapshot.
Abandoning a reservation for another target retires every reserved sequence, so no sequence is reused for different plaintext.
A remote chain ahead of every locally owned sequence refuses publication instead of replacing its files.

A shared folder holds each writer under its lowercase device digest, with twenty-digit sequence files and a separate `head`.
The writer saves each complete file before replacing its head and removes older files only after the new head is visible.
The store's publishing lock serializes this device's local publishers.
A reader advances only after authentication and a committed record merge, so a cloud drive that delivers the head before its files leaves the cursor waiting for those files.
An unreadable peer does not stop the other peers from merging.
The recursive watch reports normalized relative paths and ignores this device's own chain and unfinished filenames.

Export writes the same layout as a snapshot directory.
Import reads every chain there, including this device's own backups, and writes nothing to that directory.
Recovery authenticates a bounded segment to learn the membership and epoch keys before reading the directory's retained history.

The pairing admission carries the checked membership, every held epoch key, and an optional snapshot hint under magic `YRH1`.
It carries no library records and refuses the earlier `YRA1` format.
The joining device checks that the log actually admits its public identity before saving its association and fetches records through ordinary segment sync afterwards.
A delayed entry cannot replace an association created after its confirmation.
Relay pairing publishes the admission's membership before handing out the final entry, so the new device can collect immediately.

The relay replaces its bundle slots with immutable per-device chains.
An append verifies both the fresh signed request and the segment against the checked membership log, admits compatible successors or a newer snapshot start, and refuses an occupied sequence with different bytes.
Exact latest-envelope replay renews an idle chain without allocating a sequence.
Collection accepts peer cursors and returns bounded pages with every chain's availability metadata, its last publication time, the checked membership, and a continuation flag.
The receiver persists only cursors it authenticated and applied; temporary skip cursors let it continue other peers after reporting one broken chain.
The app reports refused or unfinished histories instead of recording a successful sync.
Each chain's metadata also reports an unfinished latest snapshot, including when a reader already applied every currently available part.
The receiver compares that progress with a delivered latest header and waits for the remaining parts before reporting completion.

The relay retains at most 64 chains and 64 MiB of segment envelopes per account.
Retention touches only the publishing device's prefix and keeps the newest complete snapshot and its suffix when they fit beside the other chains.
Otherwise it keeps a bounded suffix without a snapshot hint, leaving a fresh or lagging reader with `Lost`.
A segment that cannot fit beside the other chains refuses with `TooLarge`.
This makes the quota explicit for a snapshot larger than 64 MiB: the relay carries deltas, and a lagging device bootstraps once through a shared folder or snapshot directory.
A known oversized snapshot or an existing relay suffix without a complete snapshot suppresses automatic snapshots at that target.
Returning from another transport uses a distinct signed `Bridge` request carrying a newer delta and its ordinary plaintext predecessor, with no snapshot claim.
Only a peer whose cursor already matches that predecessor can continue, while others receive `Lost`.
An ordinary publisher cannot silently bridge a missing predecessor.

The Worker stores envelope and pairing-message chunks of at most 1 MiB with separate metadata and lengths, because [Durable Objects limit each SQL row to 2 MiB](https://developers.cloudflare.com/durable-objects/platform/limits/).
Sequences use canonical twenty-digit text so SQLite and JavaScript cannot round or sign-truncate a `u64` sequence.
Quota and collection planning read metadata before loading selected ciphertext.
Account, mailbox, and directory requests run inside the platform's SQLite storage transactions, including membership, nonce, segment chunks, pruning, head changes, and consuming a message.
The Rust adapter finishes those SQL operations without intervening I/O and returns its result only after the transaction commits.
An object with the earlier bundle or unchunked-mail tables refuses before changing them, preserving its data for an explicit cutover to fresh object storage.
Native transport tests cover paging, multipart completion, exact replay, conflicting envelopes, oversized snapshots, removal, pairing, and recovery.
The Worker compiles for its pinned Wasm target; its local runtime campaign and Windows and Linux checks remain deferred with the owner's long verification window.
Full release-candidate assurance runs the local Worker end-to-end check before the proof campaign.

The earlier `YRB1` and `YRL1` formats and their bundle APIs are removed.
The wire version and membership operation bytes stay unchanged.

The database format becomes 3.
Earlier executables must refuse it because they cannot maintain the dirty table or publication reservations.
The current executable reads only its own format, following [the store's format boundary](a-store-reads-only-the-format-it-writes.md).
An older store stays untouched and requires an explicit cutover with a preserved backup.

[Cursor and publication decisions](../../crates/youarel-core/src/segment_rules.rs), [canonical packing](../../crates/youarel-core/src/segment_rules/records.rs), and [store reservations](../../crates/youarel-store/src/session/publication.rs) hold the contracts.
[Transport tests](../../crates/youarel-cli/tests/segment_replay.rs) exercise lost acknowledgments and delayed exchanges among three stores against the full-state join.
[The Quint model](../../specs/segments.qnt) bounds the exchange to three writers, four reserved sequences per writer, two writes per writer, and single-part snapshots.
Its digests encode the bounded plaintext injectively, leaving cryptographic authentication to the segment adapter and its tests.
Every committed delivery trace replays through the production cursor step, including the missing-predecessor probe.
Multi-part bounds and canonical record packing have separate source-bound Kani harnesses.
Native generation checks the pinned CLI version and writes the same normalized traces without starting complete model checking.
The obligation manifest leaves an unmeasured complete state count absent, and the proof gate refuses that absence even if a later complete exploration succeeds.
The owner has deferred the full proof campaign, so those obligations remain pending until their exact-source results and complete state count are recorded.

Rejected alternatives:

- Sending only operations, because state joins tolerate duplicate and reordered delivery without coordinating a global operation log.
- Omitting unrelated fields inside a record, because a partial creation would need a second representation for an unknown link or missing required fields.
- Clearing dirty items before delivery, because a crash could then lose an unpublished edit.
- Resealing a visible reservation on every retry, because immutable sequence files and relay entries must keep one envelope.
- Reusing a reservation's sequence at another target, because the two targets could later expose conflicting plaintext under one device and sequence.

## Consequences

Ordinary publication grows with touched records rather than the whole library.
Snapshots and target changes still read the library, but do so in bounded parts.
The relay quota can require another bootstrap source and does not promise that every library fits a complete snapshot there.
The database temporarily holds reserved plaintext and sealed envelopes in addition to the state.
Local disk encryption and the operating system's user boundary protect those files as they protect the library.
Older pre-release stores and executables cannot participate in this format.
