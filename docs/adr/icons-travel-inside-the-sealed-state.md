---
status: superseded
date: 2026-10-16
superseded-by: icons-stay-on-the-device.md
---
# Icons travel inside the sealed state

## Context

[The window launcher renders the terminal launcher's machine](window-launcher-renders-the-terminal-launchers-machine.md) can show a picture beside each shortcut, and the terminal cannot.
The first plan kept icons as separate encrypted blobs at the relay, named by an HMAC under the account's key.
That plan needs a second channel beside the bundle, storage and its collection at the relay, and a separate way through a file or a shared folder.
An icon comes from a site, so its image is untrusted input, and a member's device could send any bytes as an icon to the others.

## Decision

An icon is a square PNG of 64 pixels that `youarel-icon` draws from the site's image, under limits on the bytes it reads, the sides it accepts, and the memory it allocates.
The state holds each image once, under its digest, and a shortcut holds the digest of its icon in a register like its other fields.
Images merge by the union of their digests, and two images under one digest settle on the greater, so the merge stays a join.
A device keeps only the images its live shortcuts show, pruning when it saves, never inside a merge, and another replica's state brings back any image it still shows.

The device that adds a shortcut fetches the page once for its title and the icons it names, best first, then the site's favicon, and keeps the first that draws.
`youarel icons` fetches the missing icons of shortcuts that came without one, such as a bookmark file's.
The window does the same on its own, a few shortcuts at each tick of its schedule, and asks each site once while it runs.
Its icons then travel to every member, so an import's hosts reach the network from the window that first runs after it, as an addition's reach it from the device that adds.
The window launcher draws every image again with `youarel-icon` before it shows it, so the webview's decoders read only images that youarel encoded, whichever member sent them.

Rejected alternatives:

- Separate blobs at the relay, because they need a second sync channel, collection at the relay, and a different path for files and shared folders, while the sealed state already reaches every member by every path.
- Each device fetching its own icons, because every device would then reveal every shortcut's host to the network, not just the one that added it.
- Keeping the site's own image, because its size and format vary without bound, and every device would decode it with whatever decoder it has.

## Consequences

The state grows by a few kilobytes for each distinct icon, which the longest state of 16 MiB holds thousands of times.
A site that offers only a vector icon shows none, since `youarel-icon` draws from pixels.
The device that adds a shortcut reaches the icon's host as it reaches the page's, and no other device does.
Two windows that run before either's icons arrive may both ask a site, which reveals no host beyond the members.
