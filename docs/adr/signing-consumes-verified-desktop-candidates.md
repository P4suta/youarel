---
status: accepted
date: 2026-11-08
---
# Signing consumes verified desktop candidates

## Context

The product requires signed distributions for macOS, Windows, and Linux.
Existing builds omit the desktop program and expose signing credentials while executing source.

## Decision

Build both programs from an immutable main revision without release credentials.
Bind candidate files to their source, version, target, and hashes.
Use separate protected jobs for signing, notarization, and publication under [GitHub's privilege guidance](https://docs.github.com/en/actions/reference/security/secure-use).
Signing runs trusted controller tools and consumes candidate bytes without executing candidate programs or hooks.
Follow [Apple's nested-code rules](https://developer.apple.com/library/archive/technotes/tn2206/_index.html) and verify Windows Authenticode publishers and timestamps.
Keep macOS installers, Windows archives, and Linux archives and Debian packages, including desktop entry points.
Assemble verified assets in a draft before immutable publication.
The release hold gates every credential-bearing distribution operation.

## Alternatives

Signing within build jobs needs fewer transfers but exposes credentials to candidate execution.
Publishing first simplifies uploads but prevents complete immutable releases.

## Consequences

Artifact identity checks and isolated jobs add release steps.
Live credential and installation checks remain required before distribution.
Reconsider packaging when installer requirements or supported architectures change.

## Confirmation

Reject mismatched candidates, test both programs in packages, and exercise native signing and installation before publication.
