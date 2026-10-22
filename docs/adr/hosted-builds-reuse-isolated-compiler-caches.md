---
status: accepted
date: 2026-10-22
amends: compiler-caches-belong-to-a-checkout.md
---
# Hosted builds reuse isolated compiler caches

## Context

The Linux window gate takes about 23 minutes on a fresh hosted runner, while its execution test takes seconds.
Docker volumes disappear with that runner, so the next run compiles the webview dependency tree again.
[GitHub confines pull-request caches to their merge refs](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache), while later pull requests can restore a cache from their default branch.

## Decision

Keep [checkout and image ownership](compiler-caches-belong-to-a-checkout.md) for every compiler cache.
On GitHub Actions, the window container binds a directory under the runner's temporary directory instead of a named volume.
Its name uses the same canonical checkout and pinned-image digest as the local volume.
The cache action adds the runner's operating system, architecture, and app lockfile digest.
A restore prefix keeps the same checkout and image while allowing Cargo to reuse dependencies across lockfile updates.
Native owner-side runs retain their named volumes.
After successful checks, the container returns compiler-output ownership to the checkout owner so the host can archive private incremental files without widening their permissions.

The descriptor validates absolute existing directories and UTF-8 output fields before starting Docker.
Its core byte contract rejects line delimiters and the quote that would end a Docker CSV field.
The directory's canonical form receives the same validation, and preparing a cache refuses a file without replacing it.

The ordinary gate runs all selected checks after every restore, including a cache miss.
A cache supplies no test verdict, proof receipt, or assurance credit.
The container retains its four-CPU and eight-GiB bounds.

An explicit default-branch workflow warms both the ordinary Cargo cache and the window compiler cache by running the existing Linux window check.
It adds no push or scheduled trigger and starts no proof, mutation, benchmark, signing, or publication task.
It never promotes a pull request's cache into the default branch.

## Assurance

`verification_rules::cache_field_byte` holds the supported deterministic contract, with a source-bound Kani harness and a negative probe in the obligation manifest.
Their full campaign remains unrun until the candidate gate or an explicit run.
Filesystem canonicalization, Docker CSV mounts, and GitHub cache restoration cross external semantics.
Native tests exercise the production descriptor, actual Docker argument builder, path refusals, canonical aliases, identity separation, file collisions, and preservation of existing compiler data.
The native substitute command is `cargo nextest run --locked --package xtask --lib -E 'test(app::cache::tests) | test(app::tests)'`, also covered by `just check`.
Those tests don't establish a hosted cache hit or a runtime improvement; hosted runs supply those observations.
The first hosted run passed its checks but failed to save root-owned incremental lock files.
The container now restores output ownership before a successful exit, and the hosted cache action verifies that archive boundary.
A hosted-only regression test opens every completed compiler file as the runner before the cache action can save it.
Revisit this boundary if the adapter replaces filesystem canonicalization or Docker's mount interface, or if cache contents ever influence assurance admission.

## Consequences

A fresh cache or image still needs compilation.
The first default-branch warm run pays that cost once so unrelated pull requests can reuse it.
Cargo remains responsible for invalidating artifacts within one checkout and image.
Deleting or evicting a cache changes run time without changing the required checks.
