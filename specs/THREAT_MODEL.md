# The threat model of youarel

## What it protects

- The links, titles, aliases, notes, and folder names and marks of an account.
- Which device opened which shortcut, and how often, which never leaves the device.
- The icons a device gathered for the hosts of its shortcuts, which never leave the device.
- The account's keys, which never leave its devices except inside a pairing or a key slot.

## Who it resists

- **Anyone who carries a segment chain**, such as a shared folder's provider or the relay service.
  They see sizes, times, chain lengths, sequence numbers, snapshot boundaries, the account identifier, device identifiers, and public keys, and nothing of the records.
- **Anyone who changes a segment in transit.** A changed operation fails its signature, a changed header or ciphertext fails authentication, and a segment from a device that is not a member is refused.
- **Anyone who records ciphertext now to decrypt it once a quantum computer exists.** Every secret travels under a symmetric key or through X-Wing, whose ML-KEM half stands when X25519 falls.
- **A removed device.** The rotation that follows a removal gives the remaining members a key the removed device never sees, and its later segments are refused because it is no longer a member.
- **Someone who intercepts a pairing.** The PAKE gives them no offline test of the code, so each forged message is one guess against forty bits, and the invitation then expires.
- **Anyone who replays a request to the relay, or sends one as a member it is not.** The relay admits a request once, within two minutes of its time, and only under the signature of a device the account's log lists.
- **A site that sends a crafted image as an icon.** A device decodes a site's image with a memory-safe decoder under limits before it keeps the image, and the window draws every icon again the same way before its webview shows it.
  No member sends an icon, since no segment carries one.
- **A site or a member that writes markup into a title, a note, or a folder's name.** The window's page sets every such text as text, never as markup, and its content security policy admits only the window's own script and styles.
- **A crafted bookmark file or color theme.** youarel reads a browser's bookmarks and a theme's file within bounds on their size, with memory-safe parsers that fuzz targets exercise, keeps only the links that a shortcut admits and only colors as palettes, and never writes a browser's files.
- **A former member that steers a recovery to its own account.** It knows the recovery member's public keys from the log, so it can list them in an account it founds, and the relay's directory answers anyone; a recovering device joins only an account whose log holds the recovery member's own consent to it.

## What it does not resist

- **A member device under an attacker's control.** A member holds the keys, so it can read the state and write anything into it.
- **A carrier that withholds segments or replays old ones.** The record join keeps every write a device has and the cursor refuses a conflicting or missing predecessor, but withholding can delay progress.
  A lagging reader gets `Lost` when the retained suffix has no complete bootstrap, and uses another source without weakening admission.
- **Traffic analysis** of when and how much a device sends.
  A device that syncs on its own writes segments only for pending changes or a required snapshot or bridge, so a carrier learns roughly when its person edits or changes transports.
- **What a site learns from its icons.** A device asks a site for its page and its icons when a person adds one of its links, and for its favicon when a person opens one of its links and the device keeps no icon for its host.
  The site then sees the device's address, as it sees the browser's visit, and no device asks a site at any other time.
- **What the relay learns by serving.** It sees each request's address, time, size, peer cursors, account and device identifiers, public keys, chain lengths, sequence numbers, snapshot part counts, device count, and publication times; it holds no key to any records.
- **A flood of pairings at the relay.** Its mailboxes need no signature, so someone can hold them open until they expire; the relay bounds how many it keeps and what each holds.
- **A flood of accounts at the relay.** Anyone can found an account, so the relay bounds what each holds and deletes an idle one after its retention; a deployment adds the rate limits of its platform.
- **A person who loses every device and has no recovery key.** The data is then gone, and a relay deletes its copy after its retention period.
- **Someone who reads the device's files.** The state, the settings, and the device's keys sit in the user's data directory with the directory's own permissions on every operating system, so their reader holds the device, as the first item says.
  The settings hold the window's sync target, the time of its last sync, and its themes in `settings.json` beside the store.
  The store's database can also hold the earlier text of an edited or removed item until it reuses those pages.
- **Someone who reads a recovery key.** The key grows into a member, so its reader joins the account as that member until a member removes it.
- **What the relay's directory reveals.** It names the account of any recovery member whose identifier someone asks about, and that identifier appears only in the account's log and to the key's holder.
- **A quantum attacker during a pairing.** The PAKE's authentication is classical, so an attacker who can break it while the pairing runs can impersonate a side; recorded pairings stay secret through the key encapsulation.

## What the proofs cover

- **The pairing.** [`specs/proverif/pairing.pv`](proverif/pairing.pv) shows, against an attacker who controls every channel, that the code admits no offline test, the entry's keys and state stay secret, a joiner confirms only a welcome the inviter sent, the inviter adds only a joiner that confirmed the channel key, and a joiner installs only an entry the inviter sealed for it.
- **A pairing attacked now and broken later.** [`specs/proverif/recorded.pv`](proverif/recorded.pv) lets the attacker answer either side during the pairing, then gives it the code and every Diffie-Hellman exponent, and shows that the entry's keys and state stay secret.
  Its probe, the first design of the pairing, which sealed the admission into the welcome, fails: an attacker who sent its own join kept a welcome it could open later.
- **A rotation after a removal.** [`specs/proverif/rotation.pv`](proverif/rotation.pv) gives the attacker every key of the removed device, the old epoch key, and Ed25519 forgeries, and shows that the new epoch key and the state stay secret, and that a member installs only the rotation the rotating member signed.
- **A recovery.** [`specs/proverif/recovery.pv`](proverif/recovery.pv) gives the attacker the recovery member's public keys, the directory, accounts of its own, and Ed25519 forgeries, and shows that a recovering device joins only an account that the key's member consented to, and that the epoch key and the state the device seals afterwards stay secret.
  Its probe, the first design of recovery, which asked only that the account's log list the key's member, fails: the attacker listed that member in its own account and read what the device sealed there.
- **The membership rule.** Kani proves every decision of the replay, and the Quint model [`specs/membership.qnt`](membership.qnt), whose traces replay through the implementation, covers three devices that write and exchange their logs in any order.
- **Segment cursors, publication, and relay retention.** Source-bound Kani harnesses and the bounded [`specs/segments.qnt`](segments.qnt) model describe the new contracts, and committed traces replay through the production cursor step.
  Their full proof campaign and complete model state count remain pending under the owner's verification hold.

The ProVerif models are symbolic: each primitive is perfect, SPAKE2 appears as Diffie-Hellman with shares blinded by the code, and a model covers the messages of [WIRE.md](WIRE.md) as tuples, not as bytes.
They show the constructions sound under those abstractions, and the wire tests, the fuzz targets, and the libcrux proofs cover what the abstraction leaves out.

## Assumptions

- The primitives of [CRYPTO.md](CRYPTO.md) are secure, and the libcrux and RustCrypto implementations meet their published guarantees.
- The operating system's random generator works as documented.
- A person compares nothing by eye; the short code carries the pairing's authentication.
