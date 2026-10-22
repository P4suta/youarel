---
status: proposed
date: 2026-10-19
amends: window-syncs-on-its-own-schedule.md, window-watches-the-sync-folder.md
---
# The window publishes only its own changes

## Context

[The window syncs on its own schedule](window-syncs-on-its-own-schedule.md), and [it watches the sync folder](window-watches-the-sync-folder.md) to merge another device's bundle within seconds.
It still published this device's bundle every five minutes, even when the device held nothing new.
Each publish seals the state under a fresh nonce, so no bundle repeats the bytes of the one before it.
A cloud drive carries every changed file to every other device, and Synology Drive reported a sync every few minutes while nothing changed.
Every device reads every other device's bundle, and each bundle carries the whole state of its device.
A relay stamps a member's slot only when the member publishes.
The other members hear of a device whose slot stayed unrenewed for 180 days.

## Decision

The window writes this device's bundle only when the device has something new to say at its sync target.
A write of the device's own counts as new until a publish to the target carries it.
An operation of the account's log counts as new until a publish carries it too.
A target that lacks the device's bundle, or that no publish reached, also counts as new.
`sync_rules::pending` decides from values: the newest stamp of the device's own writes, what the last publish carried, and whether the target holds the bundle.
Every new write of a device orders after every stamp its state holds, and the log only grows.
Something new thus shows as a later stamp or a longer log.
A write of the device that another device's later write replaced leaves no stamp behind, and that device's bundle carries what replaced it.
The store keeps what the last publish carried and the target it reached in `published.json` under its lock.
A lock of its own lets one process at a time publish and record, so a record never names a bundle that another process overwrote.
`youarel sync` still publishes every time a person runs it, and every publish records what it carried.

The interval now paces the merges, which read the other devices' bundles every five minutes and after each notice, and write nothing.
Each merge then asks the store whether the device has something new to say there.
That question catches a write of the terminal, and an operation that another device brought.
The window also marks something new on its own edits, additions, and imports, and asks the store at its start.
A publish waits ten seconds after the last edit of a burst, and goes at once for anything else.
A person's request to sync reads at once and publishes at once what waits, as a change of the sync's settings does.
A failed publish keeps its writes pending and tries again after the backoff, and a failed merge tries again as a merge.
Kani proves that no publish falls due while nothing waits for one, and that a pending device publishes once the settling or the backoff has passed.
It also proves that a merge falls due within the interval, that a failed publish keeps its writes, and that only something new makes a device pending.

Through a relay, a merge counts this device's bundle as missing once the relay stamped it thirty days ago.
The device then publishes again, long before the other members would hear of it as gone.
A shared folder keeps no such clock, so a bundle there stays unchanged while nothing changes.

Rejected alternatives:

- Comparing a new bundle's bytes with the old one's, because every seal draws a fresh nonce, so no two bundles share their bytes.
- A longer interval, which only spreads the same writes further apart.
- Republishing the writes that a merge brought from another device, since every device reads every bundle and the bundle of the write's origin carries it.
- Counting only the window's own edits, because the terminal writes the same store, and a restart would forget what waited.
- A record of each target the device ever published to.
  A record of the last target already keeps every target's bundle current, at the cost of one more publish when a person switches between two targets.

## Consequences

An idle device writes nothing into the shared folder, so a cloud drive reports no sync while nothing changes.
An edit changes its own device's bundle once, and the other devices only read it.
The first start after this change publishes once on every device, since no record of a publish exists yet.
A change of the account's members makes each device publish once more.
That publish also lets a device rotate to a key that every member holds.
A write that a terminal command makes while the window runs reaches the other devices within the interval.
A carrier of the bundles learns when a device's person edits, since a bundle now changes only after an edit.
A bundle at a relay renews itself every thirty days while nothing changes.
