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
A device keeps every epoch key it can unwrap, by its commitment, since concurrent rotations may share an epoch number, so a bundle sealed under an earlier or a losing key still opens.
A device MUST seal under the current epoch, and before sealing it MUST rotate when a member lacks a slot of the current epoch or a member was removed since the last rotation.
A removal and its rotation SHOULD travel in one bundle.

## Bundles

A bundle carries an account's membership log and one device's state, sealed under an epoch key and signed by its sender.

A device importing a bundle MUST:

1. reject a bundle of another account;
2. merge the bundle's log into its own and resolve it;
3. reject the state when the sender is not a member after resolution, or its signature fails;
4. open the state with the epoch key the bundle names, and reject it when the key commitment or the authentication fails;
5. decode the state, admit every link again, and merge it into its own.

A bundle can travel as a file through any channel: a USB drive, a messenger, or a shared folder in which each device writes only `<device>.youarel`, named by its identifier in hexadecimal.
A device syncing through a shared folder imports every other device's file there, reports and skips any it refuses, and then writes its own.
A device that syncs on its own writes its file only when the folder lacks it, or when the device holds a write of its own or an operation of the log that no file it wrote there carries.
Each seal draws a fresh nonce, so a file written with nothing new would change for every other device and carry nothing.

## The relay

The optional relay is a shared folder that checks who writes: it keeps one slot per member, holding that member's newest bundle, and hands every member's slot to any member.
It never holds an epoch key, so it reads no state.

A member's request carries the account, the device, a verb, the device's clock in milliseconds, a random nonce, and a payload, signed by the device under `youarel/v1/request`.
The relay MUST refuse a request whose time lies more than 120 seconds from its own clock, whose nonce it admitted before, whose device the account's log does not list as a member, or whose signature fails.
It keeps each admitted nonce until the request would be stale anyway.

- **Publish** carries the member's bundle, which the relay MUST verify against the account's log and the member's keys before it replaces the member's slot.
  The relay merges the bundle's operations that verify into its own copy of the log, so the first bundle of an account founds the account at the relay, and a device that returns after the relay deleted the account seeds it again.
- **Collect** hands back every member's slot, in ascending order of the devices.
- **Erase** deletes everything the relay keeps of the account.

The relay drops the slot of every device the log stops listing, keeps an account no longer than 400 days after its last admitted request, and bounds what an account holds in all.
A member hears of a device whose slot the relay has not renewed for 180 days, and ends that membership only when its person says so, since the slot's time is the relay's own claim.
A device that syncs on its own publishes its bundle again once its slot is 30 days old, even with nothing new, so no member hears of it as gone while it syncs.
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
   It then writes the `Add` operation and sends the entry: the log, which now lists the joiner, the epoch keys, and its state, sealed under the channel key.
6. The joiner opens the entry and installs what it holds, as a member.

An attacker who reads the messages cannot test codes offline, an attacker who replaces a message gets one guess per invitation, and an invitation expires when it is used or abandoned.
The inviter seals the entry only for a joiner that proved the channel key while the code was secret, so an attacker who answers the welcome itself receives nothing to open, even after it learns the code.
The secrecy of the entry rests on the key encapsulation, which resists a quantum attacker, as well as on the PAKE.

## Recovery

A recovery key brings a new device into its account after every other device is gone.
The key is a secret that a person prints and keeps, and the keys of one member grow from it, the same each time, as [CRYPTO.md](CRYPTO.md) defines.

1. A member creates the key, signs the consent as the key's member, and writes the `AddRecovery` operation.
   It then rotates, so the key's member holds a slot of the current epoch, as every member does.
2. A device that recovers finds a bundle of the account: a file, a shared folder, or the relay, whose directory names the account of the key's member.
   Through the relay, it collects the bundles as the key's member, which the relay admits as any member.
3. Before it keeps anything, the device MUST check that the bundle's log holds the key member's consented `AddRecovery` and still lists it, that a member signed the bundle, and that the key member's slots open the bundle's state.
4. The device then keeps the log and the epoch keys, writes the `Add` of its own keys under the key member's signature, and merges the state.
   Its first bundle carries that `Add`, so its next seal rotates to an epoch that reaches it.

The key's member stays a member, so the key keeps working until a member removes that member; a person who suspects that someone read the key removes its member and creates another.
A passkey's PRF output, which an authenticator returns for the same credential each time, can take the place of the recovery key.
