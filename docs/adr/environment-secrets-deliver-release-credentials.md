---
status: accepted
date: 2026-11-08
---
# Environment secrets deliver release credentials

## Context

Each release operation uses its own credentials.
The workplace uses 49 of its 50 permitted OIDC identities.
Adding 5 identities exceeds the [Doppler platform limit](https://docs.doppler.com/docs/platform-limits).

## Decision

Use [Doppler's native GitHub integration](https://docs.doppler.com/docs/github-actions) to deliver each purpose-specific config to its matching GitHub environment.
Reference the existing shared App and platform credentials without copying their canonical values.
Keep product manifest keys in the product config.
Restrict environments to main and turn off administrator bypass.
Require the owner's approval for signing, notarization, and publication, while automatic proposals have no reviewer gate.
Verify credential delivery through a read-only workflow before enabling consumers.
Keep build jobs free of environment secrets.

## Alternatives

OIDC avoids stored delivery credentials but needs unavailable identity capacity.
Static service tokens add rotation work without improving the existing native integration.

## Consequences

Secret changes remain managed in Doppler and propagate to scoped GitHub environments.
GitHub retains encrypted environment copies.
Reconsider OIDC when purpose-specific identity capacity becomes available.

## Confirmation

Check each destination, source config, initial sync, and read-only authentication before activation.
