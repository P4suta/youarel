---
status: amended
date: 2026-10-19
amends: window-syncs-on-its-own-schedule.md
amended-by: the-window-publishes-only-its-own-changes.md
---
# The window watches the sync folder

## Context

[The window syncs on its own schedule](window-syncs-on-its-own-schedule.md): every five minutes, ten seconds after an edit, and after failures once a backoff has passed.
A bundle that another device writes into the shared folder waits up to five minutes before the window merges it.
macOS, Windows, and Linux tell a watching process of the changes in a directory, through FSEvents, `ReadDirectoryChangesW`, and inotify.
Each publish seals the state under a fresh nonce, so no bundle repeats the bytes of the one before it.
A cloud drive carries every changed bundle to the other devices, whose folders then change too.

## Decision

While the window syncs through a shared folder, it watches that folder, without its subfolders, through the `notify` crate.
A notice that may bring another device's bundle brings a merge forward, which reads the other devices' bundles and writes nothing.
`folder_rules` decides which notices count: a rescan always, a read never, and some changed files.
A changed file counts when its name ends with `.youarel` after a stem and starts with neither `.` nor `~$`, unless it names this device's own bundle.
`sync_rules` waits until the folder has stayed quiet for two seconds after the last notice, so a bundle that another device or its cloud drive still writes settles first.
A notice never brings a publish forward, and no merge runs during a backoff.
The interval and the settling count from the last publish, and a backoff from the last attempt of either sync, so a merge never puts a publish off.
Kani proves these promises over every schedule and clock reading, and that no sync stays due once it begins.

A notice that counts wakes the window, which lets the folder rest for the quiet span and asks the schedule.
On every tick the window starts the watch the settings ask for and stops one they no longer ask for.
It also starts a watch again once the watch loses its folder or a sync fails, since Windows drops the watch of a deleted folder without a notice.
A watch that fails to start leaves the window on its schedule, and the log names the reason once.
A sync reports and skips a bundle it can't read, since a cloud drive may still write it or hold it locked.
The settings say whether the window watches the folder.

Rejected alternatives:

- A debouncing crate such as `notify-debouncer-full`, because the core's schedule already waits for the folder to rest under a Kani proof.
  A second timer outside the core would only decide the same again.
- A faster poll, because each poll reads every bundle in the folder, while a watch costs nothing until something changes.
- A publish in answer to a notice, because each publish writes fresh bytes that the other device notices in turn, so two devices would answer each other without end.
- A notice that cuts a backoff short, because a failing folder would then see a sync at every change.
- A watch of a relay, which holds no folder, so a relay keeps the schedule alone.

## Consequences

A change from another device reaches a running window within seconds, in both directions, and the bundle files stop changing after one round.
A device learns the name of its own bundle with its first sync, so until then one merge follows its own publish and changes nothing.
A cloud drive that downloads a bundle only when a program reads it raises one more notice, which costs one more merge.
A watch that the operating system drops without a word and without a failed sync leaves the window on its schedule until the settings change or the window starts again.
The terminal's `youarel sync` still publishes every time, since a person runs it by hand.
