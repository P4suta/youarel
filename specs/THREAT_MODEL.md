# The threat model of youarel

## What it protects

- The links, titles, aliases, and folder names of an account.
- Which device opened which shortcut, and how often, which never leaves the device.
- The account's keys, which never leave its devices except inside a pairing or a key slot.

## Who it resists

- **Anyone who carries a bundle**, such as a messenger, a shared folder's provider, or the relay service.
  They see sizes, times, the account identifier, the device identifiers, and the device public keys, and nothing of the state.
- **Anyone who changes a bundle in transit.** A changed operation fails its signature, a changed state fails its authentication, and a bundle from a device that is not a member is refused.
- **Anyone who records ciphertext now to decrypt it once a quantum computer exists.** Every secret travels under a symmetric key or through X-Wing, whose ML-KEM half stands when X25519 falls.
- **A removed device.** The rotation that follows a removal gives the remaining members a key the removed device never sees, and its later bundles are refused because it is no longer a member.
- **Someone who intercepts a pairing.** The PAKE gives them no offline test of the code, so each forged message is one guess against forty bits, and the invitation then expires.

## What it does not resist

- **A member device under an attacker's control.** A member holds the keys, so it can read the state and write anything into it.
- **A carrier that withholds bundles or replays old ones.** The state merge keeps every write a device has, so an old bundle loses nothing, but a device can miss writes until another channel brings them.
- **Traffic analysis** of when and how much a device sends.
- **A person who loses every device and has neither a passkey nor a recovery key.** The data is then gone, and a relay deletes its copy after its retention period.
- **A quantum attacker during a pairing.** The PAKE's authentication is classical, so an attacker who can break it while the pairing runs can impersonate a side; recorded pairings stay secret through the key encapsulation.

## What the proofs cover

- **The pairing.** [`specs/proverif/pairing.pv`](proverif/pairing.pv) shows, against an attacker who controls every channel, that the code admits no offline test, the entry's keys and state stay secret, a joiner confirms only a welcome the inviter sent, the inviter adds only a joiner that confirmed the channel key, and a joiner installs only an entry the inviter sealed for it.
- **A pairing attacked now and broken later.** [`specs/proverif/recorded.pv`](proverif/recorded.pv) lets the attacker answer either side during the pairing, then gives it the code and every Diffie-Hellman exponent, and shows that the entry's keys and state stay secret.
  Its probe, the first design of the pairing, which sealed the admission into the welcome, fails: an attacker who sent its own join kept a welcome it could open later.
- **A rotation after a removal.** [`specs/proverif/rotation.pv`](proverif/rotation.pv) gives the attacker every key of the removed device, the old epoch key, and Ed25519 forgeries, and shows that the new epoch key and the state stay secret, and that a member installs only the rotation the rotating member signed.
- **The membership rule.** Kani proves every decision of the replay, and the Quint model [`specs/membership.qnt`](membership.qnt), whose traces replay through the implementation, covers three devices that write and exchange their logs in any order.

The ProVerif models are symbolic: each primitive is perfect, SPAKE2 appears as Diffie-Hellman with shares blinded by the code, and a model covers the messages of [WIRE.md](WIRE.md) as tuples, not as bytes.
They show the constructions sound under those abstractions, and the wire tests, the fuzz targets, and the libcrux proofs cover what the abstraction leaves out.

## Assumptions

- The primitives of [CRYPTO.md](CRYPTO.md) are secure, and the libcrux and RustCrypto implementations meet their published guarantees.
- The operating system's random generator and key store work as documented.
- A person compares nothing by eye; the short code carries the pairing's authentication.
