---
status: accepted
date: 2026-10-22
---
# Compiler caches belong to a checkout

## Context

Sharing a Cargo target directory between checkouts reused an incompatible library artifact and hid an exported module from another checkout.
The proof, window, and Worker containers also shared compiler volumes across checkouts and image changes.
Their normalized source path inside the container does not identify the host checkout.
The source copy also interpreted Git ignore rules as rsync exclusions, which lost committed native tool dependency locks admitted by Git's negation rule.

## Decision

The compiler volume name includes the full SHA-256 digest of a versioned domain, the canonical host checkout path, a separator, and the pinned image name.
Each task retains its own volume prefix.
Downloaded registry data remains shared, while compiler outputs belong to that checkout and image.
One global compiler volume is rejected because normalized container paths hide checkout and image changes from cache ownership.
Native commands use a target directory within their own checkout.
The source copy explicitly includes `.mise/locks` before applying the repository's exclusions and leaves other ignored runtime state and Git metadata outside the container.

## Assurance

Tests inspect the actual Docker command builders for each task and require distinct compiler volumes across checkouts and images while preserving the shared registry volume.
A native rsync fixture uses the production argument builder to check committed locks and private state, including a source path with spaces and an apostrophe.
These tests cover command construction and the local copy boundary.
Container builds and verifier campaigns require their own results.
The separation assumes SHA-256 collision resistance, the filesystem's canonical path semantics, and Docker's named-volume behavior.
Cargo remains responsible for invalidating ordinary artifacts within a checkout.
Cache membership never admits a proof result or substitutes for source-bound verification.

## Consequences

The first run in another checkout or image needs a fresh compiler cache.
Existing volumes remain untouched and can be reclaimed separately.
The [container decision](proofs-run-in-memory-limited-containers.md) retains its resource limits with this narrower cache ownership.
