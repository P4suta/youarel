---
status: accepted
date: 2026-10-15
---
# Updates verify two signatures and never go back

## Context

`youarel self-update` replaces the running program with one it downloads, so whoever can make it accept a file can run code as the person.
The download host, the network, and a stolen signing key are each a way in.
An attacker who holds an old signed release can also offer it again to bring back a fixed vulnerability.

## Decision

A release publishes a manifest that names its version and, for each target, the asset, its size, and its SHA-256 digest.
The manifest carries a minisign signature, which is Ed25519, and an ML-DSA-65 signature under the context `youarel/v1/release`.
The program compiles in the public keys it trusts and accepts a manifest only when both signatures verify under one of them; a build without keys accepts none.
It downloads the asset no larger than the manifest says, compares the digest, and replaces itself by renaming a complete file over the old one.

The highest version a device has installed or run is recorded beside its state and locked from the check through the replacement.
An update installs only a version above both the running one and that mark; going back needs `--allow-downgrade`, and the mark never falls.
The decision is the pure `update_rules` of `youarel-core`, which Kani proves.

Release workflows build, sign, notarize, and publish only when the repository variable `RELEASES_ENABLED` is `true`, which stays unset while releases are prohibited.
`cargo xtask release candidate` builds and checks the same artifacts on a workstation, signed with keys made for that run alone.

Rejected alternatives:

- Trusting the TLS connection to the download host, because it authenticates the host, not the release.
- One signature scheme, because either alone falls to one break: Ed25519 to a quantum computer, ML-DSA-65 to a flaw in a young scheme.
- An update framework such as TUF, because one product with one signing role needs only its root of trust and its rollback rule, which this keeps small enough to prove.

## Consequences

Rotating a release key needs a release signed by an old key that carries the new one.
A person who never updates keeps the old keys and the old program, and the mark protects only devices that ran a newer version.
The signing keys live in the release environment's secrets, so a release run needs the owner's approval.
