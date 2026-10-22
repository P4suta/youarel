---
status: amended
date: 2026-10-19
supersedes: icons-travel-inside-the-sealed-state.md
amends: local-first-state-in-sealed-bundles.md
amended-by: the-state-is-kept-in-an-indexed-store.md, a-store-reads-only-the-format-it-writes.md, icons-are-read-from-the-browsers-caches.md
---
# Icons stay on the device

## Context

[Icons travel inside the sealed state](icons-travel-inside-the-sealed-state.md) put every image into the state that each bundle seals and each member receives.
On the owner's Mac, icons took about 60% of a library of 88 shortcuts, whose bundle held 488 KB.
Every publish carried those images again, and a relay that serves an account would keep them for as long as it keeps the account.
The window also fetched the missing icons of a few shortcuts at each tick of its schedule, so a device reached sites that nobody had just opened.
A browser reaches a site anyway when a person opens one of its links.

## Decision

The state holds no icon, and a shortcut keeps seven registers: its link, title, alias, folder, position, removal, and note.
A device keeps the icons it gathered beside its visits, under the store's lock.
Each image sits under `icons/` and takes the name of its digest, and the index `icons.json` names the image of each host and the time the device fetched it.
Every shortcut of a host shows that host's image, and a shortcut whose host has none shows its tile.

A device gathers an icon only when a person acts.
An addition fetches the page once for its title and the icons it names, then the site's favicon, and keeps the first that draws.
An opening in the window or the terminal fetches the site's favicon when the device keeps no icon for its host.
No bundle, entry, or relay carries an icon, and no device reaches a site on a schedule of its own.
`youarel icons` goes until a later change reads icons from the browsers' own caches.
`youarel-icon` still draws every image again before a device keeps it and before the window shows it.

A store from before this change moves into the cache, once, the image that each host's live shortcuts chose last, so every icon it showed still shows.
A device that reads a state from an older build leaves the icons of that state behind.

Rejected alternatives:

- Icons inside the sealed state, because they made most of every bundle, and a relay would keep every account's images of its sites.
- Separate sealed images at the relay, for the reasons the superseded record gives, and because the relay would still keep them.
- Fetching the missing icons in the background, because a device would then reach sites that nobody had just opened.
- An icon for each shortcut, because the shortcuts of one site show one favicon, and a cache by host asks each site once.

## Consequences

A bundle carries the links and their text alone, so it loses the share that the icons took.
Each device shows only the icons it gathered, so a shortcut that another device added shows its tile until a person opens it or adds a link of its host here.
A site sees a device's address when a person adds or opens one of its links, as the browser's visit shows it, and at no other time.
The relay keeps the account's log and sealed states that hold no image.
Two devices that each open a site fetch its favicon once each.
The shortcuts of a bookmark file show their tiles until a person opens them, since `youarel icons` no longer exists.
