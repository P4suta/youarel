---
status: amended
date: 2026-10-16
amended-by: the-device-keys-live-beside-the-state.md
---
# A recovery key grows into a member that consents to its account

## Context

[Passwordless identity held by devices](passwordless-device-identity.md) promises recovery after losing every device, through a passkey's PRF output or a printed recovery key.
[Membership replayed from a signed log](membership-replayed-from-a-signed-log.md) gives the epoch keys only to members, through the slots of each rotation, and the relay admits only members' requests.
A device that recovers holds nothing but the key, so the key alone must open the newest state, prove its holder to the relay, and let the device add itself.
The relay routes every request by the account, which the recovering device does not know.

## Decision

The recovery key is 32 random bytes from which the keys of one member grow, the same each time, as [CRYPTO.md](../../specs/CRYPTO.md) defines.
A member creating the key writes an `AddRecovery` operation for that member and rotates, so the key's member holds a slot of every later rotation like any device.
A recovering device acts as that member: it opens the bundle's state with the member's slots, collects from the relay under the member's signature, and writes the `Add` of its own keys under the member's signature.

The `AddRecovery` carries the member's consent, its signature of the account's identifier, and every reader discards an `AddRecovery` without it.
The relay keeps a directory from each consented recovery member to its account, noted when a member publishes, which a recovering device asks without a signature.
A recovering device keeps nothing until the bundle's log lists the key's consented member, a member signed the bundle, and the state opens.

Rejected alternatives:

- Key slots for the recovery key outside the log, as a separate kind of slot, because every rotation would need a second path, and the relay could not admit the key's requests without listing it anyway.
- Asking only that the log list the key's member, the first design, because a member that once read the log can list the same public keys in an account it founds, and the directory would steer the recovering device there; the probe of [`specs/proverif/recovery.pv`](../../specs/proverif/recovery.pv) shows the device then seals its later state for that member.
- Printing the account's identifier with the key, because it lengthens the key by 52 symbols and still needs the relay to name the account's current copy.
- Deriving the member from the key and the account's identifier, because the recovering device would need the identifier before it finds the account.

## Consequences

The key works until a member removes its member, so a person who suspects that someone read the key removes that member and creates another.
The relay learns which recovery member belongs to which account, which it could already read from the log, and answers that to anyone who knows the member's identifier.
The directory lives in one object at the relay, like the mailboxes, and holds a row per recovery member.
A passkey's PRF output can seed a member the same way, so the adapters for libfido2's `hmac-secret`, the Windows WebAuthn API, and the macOS platform API reuse this path; they wait for hardware to test them on, and the recovery key covers recovery until then.
