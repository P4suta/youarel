---
status: amended
date: 2026-10-18
amended-by: window-watches-the-sync-folder.md
---
# The window syncs on its own schedule

## Context

The owner's design shows a switch that syncs with the other devices and a line that says when the last sync happened.
`youarel sync` already syncs through a shared folder or a relay, but only when a person runs it, and nothing remembers where it synced or when.
A sync through a relay may wait up to a minute on the network, and the window must answer every intent meanwhile.
Pairing a device and creating an account take codes and choices that the terminal already guides.

## Decision

The settings keep whether the window syncs on its own, the shared folder or the relay's address it syncs through, and the time of the last sync that succeeded.
`sync_rules` decides whether a sync is due: at once for the first, ten seconds after an edit, every five minutes otherwise, and after failures only once a backoff of thirty seconds, doubled for each further failure up to thirty minutes, has passed.
Kani proves that the backoff stays between its bounds and never shrinks with another failure, that a sync is due only once its wait has passed, and that a due sync stays due.

The window asks every five seconds whether a sync is due, and runs one with the same code as `youarel sync`, outside the library's lock, so intents keep their answers while it runs.
Only one sync runs at a time, and a person's click on the sidebar's line syncs at once.
The window reads whether the device belongs to an account from the account's genesis file alone, so it never creates an identity or reaches the credential store until a sync runs.
Without an account the settings say how to create or join one in the terminal, and the window does not pair devices.

Rejected alternatives:

- Syncing after every intent, because a burst of edits would then sync many times, and a sync through a relay may take a minute.
- Pairing and creating an account in the window, because the terminal already guides both, and the design shows only the switch.
- Keeping the sync's target in the sealed state, because the target is where this device reaches the others, which each device chooses.

## Consequences

An edit reaches the other devices within about ten seconds while the window runs, and a closed window still syncs, since closing hides it.
A failing relay is asked again at most every thirty minutes until it answers, and a person can ask sooner.
The sidebar's line and the settings show the last sync and the reason of a failure, which a person may need to read in the terminal for more.
