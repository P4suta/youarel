# Cryptography

The product uses cryptography to verify program updates.
The [retired specification](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/specs/CRYPTO.md) preserves the historical synchronization cryptography contract.

Program updates require independent minisign and `ML-DSA-65` verification in [youarel-release](../crates/youarel-release/src/trust.rs).
The trusted release keys and release prohibition remain in force.
