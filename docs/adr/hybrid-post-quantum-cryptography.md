---
status: accepted
date: 2026-10-15
---
# Hybrid post-quantum cryptography

## Context

The links a person keeps reveal what they read and where they work.
Ciphertext recorded today may be decrypted once a quantum computer breaks elliptic-curve key exchange, so confidentiality must not rest on classical asymmetric cryptography alone.
Post-quantum schemes are young, so confidentiality must not rest on them alone either.

## Decision

Every path to a secret goes through a symmetric key or a hybrid of a classical and a post-quantum primitive:

| Purpose | Construction |
| --- | --- |
| State and icons | XChaCha20-Poly1305 under a 256-bit epoch key, with a key commitment and the account, epoch, sender, and version as associated data |
| Key delivery to a device | X-Wing, which combines ML-KEM-768 with X25519 |
| Signatures | Ed25519 and ML-DSA-65, both required |
| Key derivation | HKDF-SHA-512 with domain-separated labels |
| Pairing | A short-code PAKE combined with ML-KEM-768, where the PAKE authenticates and the KEM keeps the channel secret |
| Recovery | Symmetric key slots unwrapped by a passkey's PRF output or a printed recovery key |

libcrux, whose implementations carry machine-checked proofs, is the first candidate for the primitives, and RustCrypto fills what it lacks.
The candidate is confirmed for maintenance, license, and coverage when the cryptography lands.

Rejected alternatives:

- Classical cryptography alone, because recorded ciphertext would fall to a future quantum computer.
- Post-quantum cryptography alone, because a break of a young scheme would expose everything.
- A post-quantum PAKE, because none is standardized; the hybrid keeps authentication classical, which matters only during the pairing itself.

## Consequences

Bundles and messages grow by the post-quantum key and signature sizes, which the small state tolerates.
The protocol, wire format, cryptography, and threat model become specifications with test vectors, a symbolic model checks the secrecy of the epoch key, and an external review precedes 1.0.
