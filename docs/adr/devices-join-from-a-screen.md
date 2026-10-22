---
status: accepted
date: 2026-10-21
amends: passwordless-device-identity.md, state-travels-as-sealed-segment-chains.md
---
# Devices join from a screen

## Status

Accepted.

## Context

Adding, naming, removing, and recovering a device must work inside the window.
The existing signed membership log and four pairing messages already provide authentication and key delivery.
A person should choose a shared folder or relay, enter an invitation code, and see progress without moving files or opening a terminal.
Waiting for another device must leave the library available for paging, searching, and editing.

## Decision

The settings include a Devices pane with the current device, active members, their names, their admission dates, and recovery identities.
The pane uses the complete membership identity digest of 32 bytes.
Clock writers keep their separate 16-byte identifiers.
A sealed name register joins by the ordinary register rule, including an empty value that clears a name.
Names contain at most 128 UTF-8 bytes without padding or control characters.
They travel as canonical device-name records through the same snapshot, dirty tracking, reservation, and received-segment transactions as folders and shortcuts.
The record body version becomes 2 and the database format becomes 4.
Older formats refuse before their records can be rewritten.

An invitation on a fresh device founds its identity in the same action.
Interactive device ports validate the local library through the store's authoritative session reader before opening an identity or touching the pairing transport.
If the database or its format cannot be opened, listing, pairing, naming, removal, recovery enrollment, and recovery refuse before those actions change identity, membership, or shared files.
Pairing checks that admission again before advancing the transport and before each local protocol mutation, outside the cancellation guard.
If the library becomes unreadable during pairing, a queued join stays available instead of being consumed before refusal.
Cleanup remains available independently of the library's readability.
The window calls [bounded CLI pairing steps](../../crates/youarel-cli/src/sync/pairing.rs) through a worker outside the library lock.
The [screen machine](../../crates/youarel-core/src/device_rules.rs) admits forward progress and makes completion, refusal, expiry, and cancellation final.
An invitation expires after five minutes, and the transport polling cadence is half a second.
A monotonic clock checks the deadline again before local authentication or admission after a transport wait.
A condition variable wakes cancellation without waiting for the next scheduled poll.
The local mutation guard serializes cancellation with admission and acquires no transport lock while holding that guard.
Once membership has been granted, the screen reports that fact and finishes syncing rather than claiming that cancellation undid the grant.

The relay's `Poll` mail tag returns immediately with the oldest message or no content.
The existing `Pick` tag keeps its waiting behavior.
Neither operation changes the four authenticated pairing messages.
The folder adapter reserves a public invitation number and a fresh public nonce, and keeps that run's messages under its nonce directory.
The joining side can wait for a cloud drive to deliver the offer before sending its join.
Cleanup removes only that run's files and preserves an admission that its peer still needs to read.
An admission remains queued for retry if delivery fails during a live pairing session.
Pairing progress emits an event only when the stage changes.
The invitation sheet displays the same code as text and as a QR matrix from the pinned MIT-licensed pure Rust [`qrcodegen` encoder](https://github.com/nayuki/QR-Code-generator/tree/master/rust).
Only nonempty payloads of at most 512 UTF-8 bytes and standard square dimensions from 21 through 177 enter this display.
The page draws validated boolean cells as black SVG paths on white with a four-module quiet zone, without fetching an image or interpreting the code as markup.
An encoder failure leaves the text code available.
Cancellation, expiry, refusal, completion, and a new run clear the previous matrix together with its text.
Failure to read the current device response also clears the last displayed invitation instead of leaving a stale code visible.
The encoder's QR semantics are an external trust assumption, checked on emitted matrices with the independent MIT-licensed [`quircs` decoder](https://github.com/dignifiedquire/quircs) in native tests.

The window keeps device work under its own controller, separate from the library.
It accepts one worker at a time, waits for cancellation cleanup before starting another, and rejects updates and results with an obsolete work identifier.
Ordinary scheduled sync waits while device work runs.
The CLI ports expose membership facts and share the naming, removal, recovery enrollment, and authenticated recovery paths with the window.
The CLI commands remain available.

Creating a recovery key also grants that recovery identity every historical epoch key the enrolling device holds.
Each historical grant uses the existing signed rotation format, with its original epoch and commitment and a slot sealed to the recovery identity.
The membership replay already retains rotations from current members even when their epoch does not advance the canonical epoch.
The canonical rotation and future-key revocation rules therefore stay intact.
The recovery reader authenticates those operations and opens the same key slots it already understands.
This lets a key created after an existing snapshot recover that snapshot and other retained histories, instead of authenticating a newer empty delta and silently recovering an empty library.
A folder recovery now checks the structured import report and refuses an incomplete or broken history.
The relay's existing retention and bootstrap limits still apply.

The recovery sheet retrieves new recovery material once, keeps it out of the general library view and progress records, and blocks another device action until the person acknowledges saving it.
Copy requires its button, and the Print button alone calls `window.print()`.
Print CSS includes only the open recovery sheet's key and explanation.
[Tauri documents `window.print()` across its desktop platforms](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html#method.print).
The window opens no credential, file-transfer, or folder-picker dialog for these actions.
All device names, codes, and error messages enter the page as text.

Rejected alternatives:

- Calling the existing waiting CLI command while holding the library lock, because that blocks normal library actions for the duration of pairing.
- Treating membership digests as item or clock identifiers, because truncation would merge unrelated device names.
- Repeating a recovery key in every view, because a later redraw or diagnostic could disclose it again.
- Forcing a fresh whole-library snapshot whenever a recovery key joins, because existing authenticated key slots can make retained history readable without changing compaction or relay quotas.

## Consequences

The window supports the same membership and recovery protocol as the CLI through structured ports.
Historical grants add signed log entries when a recovery identity joins, and those entries disclose only the same metadata as other rotations.
Admission dates reflect their authors' signed wall times rather than a trusted global clock.
A transport failure after a grant can leave a device added while its initial sync still needs attention.
Native tests cover names, canonical records, real folder pairing, cancellation, obsolete results, QR decoding and lifecycle, one-time recovery material, recovery after an older snapshot, and refusal before side effects on an unreadable library.
The bounded [screen model](../../specs/devices.qnt) exercises both roles and deadline boundary classes, and its committed traces replay through the production screen machine.
Its complete reachable-state count remains unmeasured until the full Quint gate runs.
Source-bound screen, display-bound, name-codec, membership, and record-packing harnesses accompany the change; their proof runs remain pending with the owner's verification window.
Cross-platform runtime and cross-machine pairing evidence remain pending.
