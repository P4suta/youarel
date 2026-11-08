---
status: accepted
date: 2026-11-08
---
# Release proposals reserve product versions

## Context

Release-please prepares one desktop product version with a companion command-line program.
Implementation crates are private and share a product version.
The release hold remains active.

## Decision

Use the [simple strategy](https://github.com/googleapis/release-please/blob/v17.6.0/docs/customizing.md) with a manifest, Conventional Commits, and typed extra-file updates for owned versions and lockfiles.
Bootstrap from the last main revision before the bookmark simplification.
Use the existing release-please App through a repository-scoped token.
Set [skip-github-release](https://github.com/googleapis/release-please-action/tree/v5.0.0) so proposals create neither tags nor releases.
A merged proposal reserves its version until the separate release controller publishes that version.
Reconcile release labels after confirmed publication.

## Alternatives

The Rust strategy follows crate releases but doesn't match the private implementation packages.
Keeping release-plz contradicts the selected proposal tool.

## Consequences

App metadata and owned lockfile entries need explicit synchronization checks.
An unpublished merged proposal blocks another version reservation.
Reconsider independent versions if the desktop and command-line programs need independent release contracts.

## Confirmation

Exercise the pinned upstream updaters and reject version drift in the normal project check.
