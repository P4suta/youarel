# The cryptography of youarel

Every secret travels under a symmetric key or a hybrid of a classical and a post-quantum primitive, so a break of either one alone exposes nothing.
The primitives come from libcrux, whose implementations carry machine-checked proofs, and from RustCrypto where libcrux has none.

## Primitives

| Purpose | Primitive |
| --- | --- |
| Signatures | Ed25519 (RFC 8032) and ML-DSA-65 (FIPS 204), both over the same message; a signature verifies only when both do |
| Key encapsulation | X-Wing (draft-connolly-cfrg-xwing-kem), which combines ML-KEM-768 and X25519 |
| Authenticated encryption | XChaCha20-Poly1305, or ChaCha20-Poly1305 under a key used once, with a key commitment |
| Key derivation | HKDF with SHA-512 (RFC 5869) |
| Digests | SHA-512/256 |
| Password-authenticated key exchange | SPAKE2 over the Ed25519 group (RFC 9382), in its symmetric mode |
| Randomness | The operating system's generator |

## Domains

Every signature, derivation, and digest names its purpose with a label, so a value made for one purpose never verifies for another.
A signed message is the label, a zero byte, and the payload.

| Label | Use |
| --- | --- |
| `youarel/v1/device` | A device identifier, the digest of its public keys |
| `youarel/v1/account` | An account identifier, the digest of its genesis operation |
| `youarel/v1/operation` | The signature and the digest of a membership operation |
| `youarel/v1/segment` | A segment's signature and the digest of its plaintext record body |
| `youarel/v1/state` | The encryption of a state under an epoch key |
| `youarel/v1/commit` | The commitment to an epoch key |
| `youarel/v1/slot` | The wrapping of an epoch key for one device |
| `youarel/v1/pairing` | The channel key of a pairing |
| `youarel/v1/welcomed` | The inviter's confirmation of a pairing |
| `youarel/v1/joined` | The joiner's confirmation of a pairing |
| `youarel/v1/request` | The signature of a member's request to the relay |
| `youarel/v1/recovery` | The derivation of a recovery member's keys from its recovery key |
| `youarel/v1/recovery-check` | The checksum of a printed recovery key |
| `youarel/v1/recovery-consent` | A recovery member's consent to the account it joins |
| `youarel/v1/icon` | The identifier of an icon a device keeps, the digest of its image, which names its file and never leaves the device |

## Constructions

- **Segment encryption.** The associated data is `youarel/v1/state`, a zero byte, version `u16`, account `digest`, sender `digest`, epoch `u32`, sequence `u64`, previous plaintext `digest`, and the kind with its snapshot fields, in that order and in the wire format's fixed widths.
  The key is the epoch key, the nonce is random, and the header carries the commitment `HKDF(epoch key, info = "youarel/v1/commit")`, which a reader compares before decrypting.
  The sender signs the complete `YRS1` envelope before its signature under `youarel/v1/segment`.
  The predecessor digest is SHA-512/256 over `youarel/v1/segment`, a zero byte, and the canonical plaintext record body, so a retry preserves its chain identity independently of its nonce.
- **Key slots.** The writer encapsulates to the device's X-Wing key and derives `HKDF(shared secret, info = "youarel/v1/slot" ‖ account ‖ epoch ‖ device)`, a key it uses once to encrypt the epoch key.
- **Pairing.** Both sides run SPAKE2 with the code's forty bits as the password and the label with the invitation number as the identity.
  The inviter encapsulates to the joiner's X-Wing key.
  The channel key is `HKDF(salt = SHA-512(transcript), ikm = PAKE key ‖ shared secret, info = "youarel/v1/pairing")`, where the transcript holds both PAKE messages, the joiner's public keys, and the encapsulation.
- **Confirmations.** Each side proves the channel key with HMAC-SHA-512 over its label and the transcript, compared in constant time: the inviter under `youarel/v1/welcomed` in the welcome, and the joiner under `youarel/v1/joined` in its answer.
- **Entry.** Once the joiner's confirmation verifies, the inviter seals the admission under the channel key, with the invitation number as the associated data, so only the side that completed the PAKE and the decapsulation opens it.
- **Recovery key.** A recovery key is 32 random bytes, printed with the first two bytes of their digest under `youarel/v1/recovery-check` as 55 symbols of Crockford base32 in eleven groups of five, whose three bits of padding are zero.
  Its member's keys grow from it by `HKDF(salt = empty, ikm = recovery key, info = "youarel/v1/recovery" ‖ part)`, where the part is `ed25519` for the Ed25519 key, `ml-dsa-65` for the seed of the ML-DSA-65 key, and `x-wing` for the X-Wing seed, so the same key always grows the same member.

## Secrets at rest

A device keeps its identity, its epoch keys, and the secrets of a pairing in plain files in its account directory, beside its state.
They keep the permissions of the user's data directory on every operating system, as the state does.
The operating system's boundary between users and its disk encryption protect them as they protect the state, and no youarel program reaches the operating system's credential store.
Secrets leave memory through `zeroize` and never reach a log.
