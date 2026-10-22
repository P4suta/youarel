---
status: proposed
date: 2026-10-19
amends: passwordless-device-identity.md, membership-replayed-from-a-signed-log.md, recovery-key-grows-into-a-consenting-member.md
---
# The device's keys live beside the state

## Context

[Passwordless identity held by devices](passwordless-device-identity.md) keeps each device's keys behind the operating system's key store.
[Membership replayed from a signed log](membership-replayed-from-a-signed-log.md) seals the identity and the epoch keys in files under a key that the Keychain, the Credential Manager, or the Secret Service holds.
The threat model ends at the device, since a member device under someone else's control holds the keys and the state alike.
The state already sits in plain files in the user's data directory, beside the sealed keys.
The key store kept the keys from a reader of those files, who reads the state beside them anyway.
macOS asked the person to allow each rebuilt program that read the key.
Linux needed a running Secret Service, whose client pulled in zbus 5, and the split of zbus left unmaintained crates that `cargo audit` had to accept.
A remote shell on Windows couldn't write the Credential Manager, and a copy of the data directory left the key behind.
[A recovery key grows into a member](recovery-key-grows-into-a-consenting-member.md) also planned a passkey's PRF output as a second way to recover, which only the operating system's authenticator gives.

## Decision

A device keeps its identity, its epoch keys, its pending invitations, and a joining device's state as plain files in its account directory, beside its state.
Each file holds the encoding that the sealed file held inside it.
`youarel_store::fs::replace` writes each whole, as it writes every other file of the store.
Each file keeps the permissions of the user's data directory on every operating system, as the state does.
No youarel program reads or writes the Keychain, the Credential Manager, or the Secret Service.
Recovery rests on the recovery key alone.

A one-time migration opened the sealed files under the old key and rewrote them plain, the identity last.
It left the code together with the key store and `YOUAREL_KEY_STORE` once both devices of the account had run it.

Rejected alternatives:

- A passphrase that seals the keys, because the identity stays passwordless, and a person would type it at every start.
- Keys in hardware, such as the Secure Enclave or a TPM, because each is a feature of the operating system, with the dialogs and the platform code of the key store.
- A passkey's PRF output as a way to recover, for the same reason.
- A key derived from facts of the machine, such as its serial number, because whoever reads the files can read those facts as well.
- The key store on macOS and Windows alone, because its dialogs and platform code would stay, while the threat model gains nothing from them.
- A mode that lets only the file's owner read the keys, because the data directory already bounds who reads the state beside them, and Windows has no such mode.

## Consequences

Whoever reads the user's data directory holds the device, since the state, the settings, and the keys sit there together, as the threat model states.
Disk encryption and the operating system's boundary between users protect the keys as they protect the state.
Every account that the user's data directory admits reads the keys, on every operating system.
A key file that an earlier build wrote with the mode 0600 keeps that mode until the account writes it again.
A copy of the data directory carries the device, keys and all, so a backup needs the care that a key needs.
No dialog of the operating system appears while a device syncs, pairs, or recovers.
A store that an earlier build sealed no longer opens, since the migration left with the key store; nothing was released, and both devices of the account migrated first.
The item that an earlier build left in the credential store stays until its person removes it.
