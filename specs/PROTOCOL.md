# The youarel protocol

This document specifies how devices of one account agree on their members, share keys, and exchange their state.
The wire format is [WIRE.md](WIRE.md), the cryptography is [CRYPTO.md](CRYPTO.md), and the attacker the design resists is [THREAT_MODEL.md](THREAT_MODEL.md).
The key words MUST, MUST NOT, SHOULD, and MAY are to be read as in RFC 2119.

## Accounts and devices

A device holds a signing key pair and a key-encapsulation key pair, which [CRYPTO.md](CRYPTO.md) defines.
A device's identifier is the SHA-512/256 digest of its public keys in their wire form.
An account's identifier is the digest of the genesis operation, which its first device signs.
Nothing else names an account: no email, no password, and no server record.

## The membership log

The members of an account are the result of a log of signed operations, each naming the operations it saw.

An operation holds the account, the digests of its parents, its author, and one action:

- `Genesis` names the first device and its public keys; it has no parents and its author is that device.
- `Add` names a device and its public keys.
- `Remove` names a member by its identifier.
- `Rotate` names a new epoch number and holds one key slot for every member it means to reach.
- `AddRecovery` names the member that a recovery key grows into, its public keys, and its consent: its signature of the account's identifier.
  The resolution treats it as an `Add`.

An operation MUST carry the author's signature over its wire form, and a device MUST discard an operation whose signature fails, whose parents it lacks, or whose account differs.
A device MUST also discard an `AddRecovery` whose consent fails under the keys it names, for the account the operation names.
The log is the set of operations a device accepted; logs merge by union.

### Resolution

A device resolves the log into the members, the current epoch, and the epoch keys it can read, by replaying the operations in one order:

1. The order is a linear extension of the parent relation.
2. Among operations whose parents are all placed, the next is the one whose author joined earliest in the order placed so far, and then the one with the smallest digest.
3. An operation applies only if its author is a member at that point of the replay; otherwise it is skipped.
4. `Add` makes its device a member, `Remove` ends a membership unless it would leave no member, and `Rotate` sets the epoch when its number is greater than the current one.

An author that is not yet a member ranks after every member in rule 2.

Every device that accepted the same operations computes the same order and the same result, whatever the order it received them in.
A removal and a concurrent action of the removed device resolve to the removal when the remover joined earlier, and two devices that remove each other at once leave the earlier one a member.
The account never loses its last member to a concurrent removal, because the replay applies only one of two mutual removals.

### Epochs

The epoch key encrypts the state.
A `Rotate` operation carries the new key wrapped for each member it reaches, so a member reads the key with its own key-encapsulation key.
The rotation that sets the current epoch in the replay names the current key; when two members rotate to one epoch at once, the first of them in the replay wins.
A device keeps every epoch key it can unwrap, by its commitment, since concurrent rotations may share an epoch number, so a segment sealed under an earlier or a losing key still opens.
A device MUST seal under the current epoch, and before sealing it MUST rotate when a member lacks a slot of the current epoch or a member was removed since the last rotation.
A removal and its rotation SHOULD travel in one publication.

## Segment chains

A device owns an increasing chain of immutable sealed, signed segments.
Each segment carries complete records of a partial state, with the ordinary register stamps and join, and authenticates the sender, epoch, sequence, preceding plaintext digest, and kind.
The first sequence is one, with a zero predecessor.
The first segment and the first part of a snapshot carry the membership log; other segments MAY carry it too.
The current writer carries the full checked log in every envelope.

A device receiving a segment MUST:

1. reject a segment of another account;
2. merge the operations that verify into its log and resolve it;
3. reject the state when the sender is not a member after resolution, or its signature fails;
4. open the records with the named epoch key and reject a failed commitment or authentication;
5. require the next sequence and matching predecessor, or the first part of an advertised complete retained snapshot after a gap;
6. decode the complete record body, admit every link again, and commit the record merges with the peer cursor in one transaction.

A duplicate plaintext at the cursor's sequence is harmless; a conflicting digest there refuses.
An older sequence does not change the cursor.
A missing prefix with no complete retained snapshot reports `Lost` and requires a one-time bootstrap through another source.
An adapter MUST report incomplete or refused peers instead of claiming a complete sync, while continuing other peers and its own publication.

A delta includes only records with this device's writes after its committed watermark.
Received writes do not trigger an echo.
The writer reserves immutable bodies and saves their exact envelopes before exposing them, then commits the watermark only after delivery of every part.
A retry reuses those envelopes, and edits after reservation remain dirty.
Automatic compaction accompanies a pending change after 256 deltas or twice the preceding snapshot's body bytes.
Snapshots stream complete records in parts targeting 8 MiB, with an admitted plaintext limit of 16 MiB per segment.

A shared folder holds `<device hex>/<20-digit sequence>.youarel` and `<device hex>/head`.
The writer saves segment files before replacing its head and prunes older files afterwards.
Readers report files that have not arrived yet without advancing their cursors.
Export and import use a snapshot directory of that same layout; import writes nothing there.
Changing targets ordinarily writes a full snapshot after every previously reserved sequence, without reusing a sequence.

## The relay

The optional relay checks who writes and keeps a bounded segment chain per member.
It never holds an epoch key, so it reads no state.

A member's request carries the account, the device, a verb, the device's clock in milliseconds, a random nonce, and a payload, signed by the device under `youarel/v1/request`.
The relay MUST refuse a request whose time lies more than 120 seconds from its own clock, whose nonce it admitted before, whose device the account's log does not list as a member, or whose signature fails.
It keeps each admitted nonce until the request would be stale anyway.

- **Publish** carries one segment whose account, sender, membership, and signature MUST verify.
  It requires the next sequence with compatible snapshot parts, or a newer snapshot part zero as a restart.
  An exact retained replay is accepted; different bytes at an occupied sequence refuse, and a pruned older publication does not change the head.
  Checked membership operations establish or update the account at the relay.
- **Collect** carries cursors and returns a bounded `Delivered` page in device order, with checked membership operations, chain availability, publication times, and a `more` flag.
- **Bridge** explicitly starts a newer delta suffix after a device uses another transport for a large-library bootstrap.
  It cannot claim to be a snapshot and advertises no snapshot hint.
  A reader still requires the matching preceding plaintext digest, so only a peer already caught up through that other transport can continue.
- **Erase** deletes everything the relay keeps of the account.

The relay retains at most 64 chains and 64 MiB of segment envelopes in an account.
An append may prune only its own older prefix, keeping the newest complete snapshot and its suffix when they fit.
Otherwise it keeps the largest fitting suffix and MUST omit the incomplete snapshot hint.
An incoming segment that cannot fit beside the other chains refuses with `TooLarge`.
A library whose snapshot exceeds that quota uses deltas at the relay, and a lagging peer receives `Lost` until it bootstraps through a shared folder or snapshot directory.
A known oversized snapshot or an existing relay suffix without a complete snapshot suppresses automatic full snapshots at that target.

The relay drops the chain of every device the log stops listing and keeps an account no longer than 400 days after its last admitted request.
A member hears of a device whose chain the relay has not renewed for 180 days and ends that membership only when its person says so, since the time is the relay's own claim.
A device renews an unchanged chain after 30 days by resending its exact latest envelope in a fresh signed request, without allocating a sequence or emitting a new edit.
A device that returns after a long absence revives nothing, because the state keeps every tombstone.

The relay keeps a directory that names the account of every recovery member: whenever a member publishes, it notes the members whose `AddRecovery` the account's log holds and still lists.
The directory answers a question of location without a signature, since only the recovery key or the account's log reveals a recovery member's identifier, and the consent keeps a member that learned that identifier from listing it in an account of its own.

A pairing through the relay uses a mailbox instead of files.
The inviting device asks the relay for a mailbox, whose small nameplate becomes the code's number, and the four pairing messages travel through the mailbox, each to one side.
The mailbox needs no signature, since the pairing messages authenticate themselves through the code, and it closes five minutes after it opens.
A device that asks for its side's next message while none waits hears nothing for up to 25 seconds, unless a message arrives first, and asks again.

## Pairing

A new device joins through a short code and four messages, which a channel of the person's choice carries.

1. The inviting member shows a code: a short number that names the invitation and forty random bits in Crockford base32.
2. The joining device sends the join: its public keys and its PAKE message, bound to the code.
3. The inviter completes the PAKE, encapsulates to the joiner's key, and derives the channel key from both secrets and the transcript.
   It sends the welcome: its own PAKE message, the encapsulation, and its confirmation of the channel key, and nothing secret.
4. The joiner completes the PAKE, decapsulates, derives the same key, and checks the inviter's confirmation.
   It sends its own confirmation of the channel key.
5. The inviter MUST check that confirmation before it sends anything secret.
   It then writes the `Add` operation and sends the entry: the log, which now lists the joiner, every held epoch key, and an optional snapshot hint, sealed under the channel key.
   It sends no library records in the entry.
6. The joiner checks that the log actually admits its public identity, installs the metadata, and fetches records through ordinary segment sync.
   An entry MUST NOT replace an existing association.

An attacker who reads the messages cannot test codes offline, an attacker who replaces a message gets one guess per invitation, and an invitation expires when it is used or abandoned.
The inviter seals the entry only for a joiner that proved the channel key while the code was secret, so an attacker who answers the welcome itself receives nothing to open, even after it learns the code.
The secrecy of the entry rests on the key encapsulation, which resists a quantum attacker, as well as on the PAKE.

## Recovery

A recovery key brings a new device into its account after every other device is gone.
The key is a secret that a person prints and keeps, and the keys of one member grow from it, the same each time, as [CRYPTO.md](CRYPTO.md) defines.

1. A member creates the key, signs the consent as the key's member, and writes the `AddRecovery` operation.
   It then rotates, so the key's member holds a slot of the current epoch, as every member does.
   The enrolling device also signs a rotation grant for each other epoch key it holds, with its original epoch and commitment and a slot sealed to the recovery member.
   These historical grants remain in the checked rotation list without replacing the canonical current epoch, so the new key can read retained snapshots and segments from before its enrollment.
2. A recovering device finds a retained chain in a shared folder, snapshot directory, or the relay, whose directory names the account of the key's member.
   Through the relay it collects as the key's member, which the relay admits as any member.
3. Before saving its association, the device MUST check the key member's consented `AddRecovery`, current membership, the segment sender's signature, and the key member's ability to open the segment under its named epoch.
4. The device keeps the log and epoch keys, writes the `Add` of its own keys under the key member's signature, and receives the retained records through ordinary sync.
   Its first publication carries that `Add` and rotates to an epoch that reaches it.
   A relay without a usable bootstrap requires recovery through a shared folder or snapshot directory.
   An incomplete or broken import is an unfinished recovery, never a successful empty library.

The key's member stays a member, so the key keeps working until a member removes that member; a person who suspects that someone read the key removes its member and creates another.
