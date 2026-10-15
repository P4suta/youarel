---
status: proposed
date: 2026-10-15
amends: passwordless-device-identity.md
---
# Membership replayed from a signed log, joined through a code and four messages

## Context

[Passwordless identity held by devices](passwordless-device-identity.md) makes an account the set of devices that hold its keys, and leaves the rule for concurrent membership changes open.
Devices exchange bundles through files and shared folders with no server to order their writes, so two devices may add, remove, and rotate at once, and each must still reach the members every other device reaches.
A removed device keeps its old keys and may keep writing, so its writes after the removal must count for nothing.
Pairing travels through the same files, so its messages must stay safe in the hands of whoever carries them.

## Decision

The members are the replay of a log of signed operations, each naming the operations its author saw, as [the protocol](../../specs/PROTOCOL.md#resolution) specifies:

- the replay takes an operation once its parents are placed, preferring the author that joined earliest and then the smallest digest;
- an operation counts only when its author is a member at that point, and a removal never takes the last member;
- the rotation that reaches the greatest epoch first in the replay names the current key, and a device keeps every epoch key by its commitment, since two members may rotate to one epoch at once.

The account identifier is the digest of the genesis operation, which commits to the founder's keys and signature, rather than the digest of the founder's public key.

An invitation code is a short number that names the invitation and forty random bits in Crockford base32, such as `14892-VREP-P9YP`, rather than words.
Pairing takes four messages: the join, the welcome, the joiner's confirmation of the channel key, and the entry.
The inviter writes the addition and seals the log, the epoch keys, and its state into the entry only once the confirmation verifies, so nothing secret leaves before the joiner proves it held the code.

A device keeps its secret keys in a file sealed under a key that the operating system's key store holds, and `YOUAREL_KEY_STORE=file` keeps that key in a file instead, for tests and hosts without a key store.

Rejected alternatives:

- Ordering by wall time, because a device's clock is its own claim, and a removed device could date its writes before its removal.
- A continuous group key agreement such as the one in Keyhive, because an account holds a few devices, and rewrapping the epoch key for each member costs little.
- The founder's public key as the account identifier, because the same device could then found only one account, and the identifier would not commit to the genesis.
- Codes of words, because a word list must be maintained and translated, while Crockford base32 survives case and lookalike letters.
- Two pairing messages, because the inviter would then add a device it never heard confirm the channel key.
- Three messages with the admission sealed into the welcome, because an attacker who answered the invitation with its own join would keep a welcome that it opens once it learns the code and breaks the PAKE, as `specs/proverif/probes/recorded.pv` shows.

## Consequences

Every device that holds the same operations draws the same members, whatever the order they arrived in, and no server needs to see the log.
Of two devices that remove each other at once, the one that joined earlier stays, which a person can predict from the order of joining.
A device that misses a removal writes on until it hears of it, and its writes from then on fail to count wherever the log arrives.
The rule carries proofs: Kani proves each replay decision for every input, an exhaustive test and the Quint model `specs/membership.qnt` cover the replay order, and the ProVerif models under `specs/proverif` cover the pairing and the rotation after a removal.
Pairing through files takes four trips between the devices, one more than a welcome that carried the admission, and a relay carries them without the person.
