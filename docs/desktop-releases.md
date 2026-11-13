# Desktop releases

Release-please reserves the product version and changelog through a pull request.
Merging a proposal creates no tag or GitHub Release.
The release hold remains active until you explicitly lift it and set `RELEASES_ENABLED=true`.
`RELEASE_PROPOSALS_ENABLED` controls automatic proposals independently.

The proposal runtime and updater tests use the same locked release-please package.
A scoped minimatch override uses the patched release because the upstream range includes [affected versions](https://github.com/advisories/GHSA-g3hv-75wg-m249).
The normal check verifies token-file input, repository glob matching, version updates, and the dependency audit.

## Distribution

The native build matrix covers Apple silicon and Intel macOS, x64 and ARM64 Windows, and x64 and ARM64 Linux.
Every target builds the desktop launcher and companion CLI from the same locked source.
macOS ships a Developer ID signed, notarized, stapled installer containing `youarel.app` and command links under `/usr/local/bin`.
Windows ships a portable ZIP containing both timestamped Authenticode executables.
The desktop program requires the Microsoft WebView2 Runtime.
Linux ships gzip-compressed tar archives and Debian packages containing both executables, a desktop entry, and an icon.
Debian packages require the Ubuntu 24.04 generation of glibc and the declared WebKitGTK and AppIndicator runtimes.
The standalone CLI assets remain available for the CLI updater.
The desktop installer and archive remain the upgrade path for the desktop program.

## Credentials

Doppler's `youarel` project holds purpose-specific `automation`, `macos`, `windows`, `notarize`, and `manifest` configs.
Native GitHub syncs deliver each config to its matching release environment.
The automation config targets `release-please`.
Other configs target `release-macos`, `release-windows`, `release-notarize`, and `release-manifest`.
Existing canonical App, Apple, and SSL.com credentials remain references to `shared-automation.release_please`, `shared-signing.macos`, and `shared-signing.windows`.
The read-only `cargo xtask release access inventory ABSOLUTE_AUTH_SCOPE` and `sync-inventory` commands inspect metadata without fetching secret values.
`sync-prepare` creates native writers and refuses existing writers.
Inspect an existing writer's destination before retrying setup.
Preserve unrelated workplace identities.

Match the product's minisign and `ML-DSA-65` keys to `crates/youarel-release/src/trust.rs` before enabling signing.
On a Unix host, prepare an owner-only parent outside the checkout with `install -d -m 700 PRIVATE_PARENT`.
Generate both manifest key pairs with `cargo xtask release keygen PRIVATE_PARENT/manifest`.
The command requires an existing private parent, refuses an existing destination, and atomically creates an owner-only directory.
Earlier versions accepted existing destinations.
Use a fresh destination with the current command.
If generation fails after creating output, preserve the private files and retry with a fresh destination.
Key generation refuses Windows hosts before creating output because inherited permissions can expose private inputs.
On macOS, the command also refuses granting access control entries on the parent and its ancestors.
The output contains encrypted minisign inputs, an `ML-DSA-65` signing key, and matching public keys.
Import the owner-readable `manifest-secrets.json` with `doppler secrets upload PRIVATE_PARENT/manifest/manifest-secrets.json --silent --project youarel --config manifest`.
Keep the private directory as an offline recovery copy.
Use `minisign.public` and the raw `ml-dsa-public-key.bin` as the compiled public trust anchors.
Commit the public trust anchors through a reviewed pull request before building a distributable candidate.
Set `WINDOWS_SIGNING_IDENTITY_SHA256` to the intended certificate's public SHA-256 fingerprint.
Set `MACOS_APPLICATION_IDENTITY_SHA1` and `MACOS_INSTALLER_IDENTITY_SHA1` to the intended Apple certificates' public SHA-1 fingerprints.
The read-only Apple credential check prints valid native selectors after importing the existing exports.
Signing requires each fingerprint to match its expected Developer ID publisher.

Run `Verify release credentials without publication` for each purpose on main before enabling its consumer.
Run `cargo xtask release prepare-windows` on a native Windows host before connecting signing credentials.
The workflow validates nonempty resolved inputs, checks App repository access, imports and validates Apple identities, queries notarization history, and checks SSL.com credential metadata and TOTP authorization.
Windows authorization uses two fixed public probe hashes and discards the activation data without requesting signatures.
The manifest check validates the minisign trust anchor and `ML-DSA-65` key encoding and size.
A signed rehearsal verifies that the `ML-DSA-65` secret matches its public key.
The verification workflow never signs, submits, tags, or publishes.

## Command migration

Use `just check` before ordinary pushes.
Replace `just push-check` and `cargo xtask push-check` with `just release-check` for the full gate on a clean committed checkout.
Use `just proofs` in place of `just proofs-native`, `just proofs-ci`, and `cargo xtask proofs --ci`.
The replacement runs the full campaign in bounded containers, including mutation testing that `--ci` omitted.
Update local scripts to use the replacement commands.

`cargo xtask ci plan BASE` now emits schema 3 instead of schema 2.
Read `full` instead of `campaign`, and stop expecting the `proofs` inventory in JSON.
The plan selects from `check`, `app`, `macos`, and `windows`.
Use `cargo xtask release-check --plan` for the complete local verifier inventory.
These repository development interfaces change before the initial 1.0.0 release.

Candidate dispatch now requires `checked_tree` from successful local release checks.
Signing dispatch now requires `candidate_run` from a successful unsigned candidate.
Prepare another unsigned candidate when the source tree changes.

## Candidate verification

Run `just release-check` locally on a clean committed checkout before preparing a release candidate.
Preview the inventory and container limits with `just release-check --plan`.
The command checks the host, the Linux window, and the full Kani, Quint, Miri, corpus replay, and mutation inventory in bounded containers.
A failed campaign stops later campaigns.
The command reports a Git tree identifier only after all checks pass and the source remains unchanged.
The identifier binds committed files and permits a squash merge with identical files.
It confirms the operator's local verification and supplies no independent proof of execution.

Dispatch `Desktop release candidate` on main with `operation=candidate`, the exact main commit SHA, the product version, and `checked_tree` from the successful local command.
Leave `candidate_run` empty.
The controller rejects a missing confirmation or a different source tree, source commit, or version.
Hosted CI checks portable shipping behavior once and runs native checks and builds on six targets in parallel.
Linux native checks include Rust app tests and a real window under a virtual display.
Compiler caches reduce repeated compilation without supplying a check verdict.
Hosted CI runs no proof, Miri, fuzz, or mutation campaign.
Build jobs receive no environment secrets.
Both programs report their exact version before packaging, and version reporting creates no library files.
CycloneDX inventories describe each executable's resolved dependencies for its target.
Candidate payloads bind source, version, target, workflow, run, attempt, and SHA-256.
Original build provenance accompanies the candidate and survives later signing.
After all native builds succeed, set `RELEASE_SOURCE_SHA`, `RELEASE_VERSION`, and `RELEASE_CANDIDATE_RUN` for the selected checkout and run `cargo xtask release inspect-candidate`.
The read-only command checks every target's downloaded payload, metadata, hashes, and original provenance without executing candidate programs.

`cargo xtask release candidate` performs an unsigned host rehearsal with temporary manifest keys.
The rehearsal validates its packages and rejects its temporary keys through the compiled CLI trust boundary.
The rehearsal grants no release authority.

## Signing

After you lift the release hold and credential checks pass, dispatch the same workflow with `operation=sign`, the same source and version, and `candidate_run` naming the successful unsigned run.
Signing verifies the original workflow and every successful native build before consuming the preserved candidates.
Signing neither repeats the campaigns nor rebuilds the candidates.
Protected jobs compile the reviewed controller before restoring verified candidate bytes and obtaining credentials.
Candidate executables and build hooks never run in credential-bearing jobs.
Apple signing seals the nested CLI and launcher before the app and installer.
Windows batch signing covers both executables, and native verification requires the expected publisher and a timestamp.
The controller requires a clean `scan_code` result for each exact input before `batch_sign`, following the [provider's batch signing procedure](https://www.ssl.com/guide/esigner-codesigntool-command-guide/).
Each provider operation must confirm success even when the SDK exits successfully.
The controller verifies the CodeSignTool archive hash and Java 17 or later before loading credentials.
The pinned 1.3.2 archive reports `v1.3.0` from its version command, so the archive hash identifies the SDK release.
The [Java argument file](https://docs.oracle.com/en/java/javase/25/docs/specs/man/java.html#java-command-line-argument-files) carries private options without exposing them in process arguments.
The controller removes temporary arguments and provider logs after each operation.
The controller resolves a trusted Microsoft SDK SignTool before requesting signatures, then requires `verify /all /pa /tw /v` to exit successfully for each executable.
Notarization persists package hashes and submission identifiers before waiting.
Unknown submission outcomes stop before another submission.
Stapling and native assessment precede final checksums and the signed asset inventory.
Final provenance covers the transformed distribution and retains original build bundles.
The signed inventory includes `distribution.json`, which binds the original unsigned run to the signed distribution run.
The successful run produces `release-RUN_ID` for later publication without rebuilding or signing again.

## Publication

Dispatch `Publish a verified desktop release` with `operation=draft`, the original source, version, and successful signed build run ID.
The controller verifies the original workflow identity, signatures, every target package, checksums, provenance, and version tag binding.
It creates a draft and uploads missing verified assets.
Existing assets must have exactly matching hashes.
The controller rejects unknown assets, changed bytes, conflicting tags, and published releases.

Inspect the complete draft and dispatch the publication workflow with `operation=publish` and the same identity.
The `release-publish` environment approval is the final publication gate.
Enable Immutable Releases and protect version-tag updates and deletions before the first publication.
Publishing changes the verified draft to public once.
Never replace published assets, move a version tag, or delete and recreate a release to repair a failure.
After confirmed publication, reconcile the associated release-please proposal from `autorelease: pending` to `autorelease: tagged`.
An unpublished merged proposal intentionally blocks another reservation.

## Recovery

Keep the original run, source, version, candidate payload, final distributions, and provenance bundles.
Artifact retention is 30 days for build transfers and 90 days for final distributions and notarization submissions.
Published provenance bundles outlive workflow artifact retention.
Use the original notarization receipt and signed package with `cargo xtask release notarize finish` to resume waiting.
Check Apple history and supply the confirmed receipt for an unknown submission.
Reruns reuse verified native candidates, completed Windows signatures, preserved notarization receipts, and complete final distributions.
The controller checks prior provider steps before allowing a first signature or submission, and unresolved previous outcomes stop without another request.
Reinspect provider state after any ambiguous write before retrying.
The controller resumes an interrupted draft upload by reusing matching assets and refusing changed bytes.
An already published version requires a new authorized version for a correction.

## Governing references

[Release-please manifest configuration](https://github.com/googleapis/release-please/blob/v17.11.2/docs/manifest-releaser.md) defines proposal and extra-file behavior.
[Tauri distribution](https://v2.tauri.app/distribute/) defines the production desktop build and bundle operations.
[Apple's code-signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/_index.html) explains nested code and distribution signatures.
[GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations) describe provenance verification.
[Doppler GitHub Actions integration](https://docs.doppler.com/docs/github-actions) defines environment-specific native secret delivery.
