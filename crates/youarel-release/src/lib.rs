//! The signed release manifests that youarel's updater trusts and its release task writes.
//!
//! A manifest names a release's version and, for each target, the asset that holds its program, with its size and SHA-256 digest.
//! It counts only when a minisign signature and an ML-DSA-65 signature both verify under one key the build trusts.

pub mod manifest;
pub mod trust;
