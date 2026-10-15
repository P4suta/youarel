---
status: accepted
date: 2026-10-15
---
# Passwordless identity held by devices

## Context

Accounts with email and passwords add sign-up friction, personal data, and a recovery path that the operator controls.
An identity provider adds a dependency on another company for every device.
End-to-end encryption needs every device to hold the same key, which a password would weaken.

## Decision

An identity is the set of devices that hold the account's keys, and nothing else.
Each device generates its own signing and key-encapsulation keys and keeps them behind the operating system's key store.
The account identifier is the hash of the first device's public key, so creating an account needs no registration.
A new device joins through a short code, such as `7-crossword-banana`, that an existing device shows; two messages carry the join request and the welcome bundle, through the relay or as files.
Another device revokes a lost one, and the remaining devices rotate the epoch key.
Recovery after losing every device uses a passkey's PRF output, from any provider, or a printed recovery key; neither is a password.

Rejected alternatives:

- Email and a password, because they add personal data and a weak secret.
- Sign-in through GitHub or Google, because it ties every device to a third party.
- Magic links, because they require mail delivery and still hold an email address.

## Consequences

Losing every device without a passkey or a recovery key loses the data, which the interface and the README state plainly.
Platform passkeys on macOS need an app bundle with an associated domain, so the terminal interface offers hardware security keys and the recovery key until the graphical launcher lands.
Concurrent membership changes need a resolution rule, which a Quint model checks before it ships.
