# Desktop releases

Release-please reserves the product version and changelog through a pull request.
Merging a proposal creates no tag or GitHub Release.
The release hold remains active until the owner explicitly lifts it and sets `RELEASES_ENABLED=true`.
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
Open the prepared manifest input fields with `doppler open --project youarel --config manifest`.
Generate the `ML-DSA-65` key with `cargo xtask release keygen PRIVATE_DIRECTORY`.
Create an encrypted minisign key pair with `minisign -G` and a unique password.
Store private keys in Doppler and keep an offline recovery copy.
Commit the public trust anchors through a reviewed pull request before building a distributable candidate.
Set `WINDOWS_SIGNING_IDENTITY_SHA256` to the intended certificate's public SHA-256 fingerprint.
Set `MACOS_APPLICATION_IDENTITY_SHA1` and `MACOS_INSTALLER_IDENTITY_SHA1` to the intended Apple certificates' public SHA-1 fingerprints.
The read-only Apple credential check prints valid native selectors after importing the existing exports.
Signing requires each fingerprint to match its expected Developer ID publisher.

Run `Verify release credentials without publication` for each purpose on main before enabling its consumer.
The workflow validates nonempty resolved inputs, checks App repository access, imports and validates Apple identities, queries notarization history, and requests SSL.com credential metadata.
The manifest check validates the minisign trust anchor and `ML-DSA-65` key encoding and size.
A signed rehearsal verifies that the `ML-DSA-65` secret matches its public key.
The verification workflow never signs, submits, tags, or publishes.

## Candidate verification

Dispatch `Desktop release candidate` on main with `operation=candidate`, the exact main commit SHA, and the product version.
The controller rejects source or version drift and checks the complete release inventory before native builds.
The normal PR gate stays fast.
Proofs and the full native release matrix run at the release gate.
Build jobs receive no environment secrets.
Both programs report their exact version before packaging, and version reporting creates no library files.
CycloneDX inventories describe each executable's resolved dependencies for its target.
Candidate payloads bind source, version, target, workflow, run, attempt, and SHA-256.
Original build provenance accompanies the candidate and survives later signing.

`cargo xtask release candidate` performs an unsigned host rehearsal with temporary manifest keys.
The rehearsal validates its packages and rejects its temporary keys through the compiled CLI trust boundary.
The rehearsal grants no release authority.

## Signing

After the owner lifts the release hold and credential checks pass, dispatch the same workflow with `operation=sign`.
Protected jobs compile the reviewed controller before restoring verified candidate bytes and obtaining credentials.
Candidate executables and build hooks never run in credential-bearing jobs.
Apple signing seals the nested CLI and launcher before the app and installer.
Windows batch signing covers both executables, and native verification requires the expected publisher and a timestamp.
The controller resolves a trusted Microsoft SDK SignTool before requesting signatures, then requires `verify /all /pa /tw /v` to exit successfully for each executable.
Notarization persists package hashes and submission identifiers before waiting.
Unknown submission outcomes stop before another submission.
Stapling and native assessment precede final checksums and the signed asset inventory.
Final provenance covers the transformed distribution and retains original build bundles.
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
The owner checks Apple history and supplies the confirmed receipt for an unknown submission.
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
