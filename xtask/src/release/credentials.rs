use super::{required, unhex, with_keychain, with_notary_key};
use crate::tool::{self, Tool};
use anyhow::{Context, Result, bail, ensure};
use std::path::Path;
use youarel_release::trust;

const AUTOMATION: [&str; 2] = [
    "RELEASE_PLEASE_APP_CLIENT_ID",
    "RELEASE_PLEASE_APP_PRIVATE_KEY",
];
const MACOS: [&str; 6] = [
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_INSTALLER_CERTIFICATE",
    "APPLE_INSTALLER_CERTIFICATE_PASSWORD",
    "APPLE_INSTALLER_IDENTITY",
];
const WINDOWS: [&str; 4] = [
    "SSLDOTCOM_USERNAME",
    "SSLDOTCOM_PASSWORD",
    "SSLDOTCOM_CREDENTIAL_ID",
    "SSLDOTCOM_TOTP_SECRET",
];
const NOTARIZE: [&str; 3] = [
    "APPLE_NOTARY_KEY",
    "APPLE_NOTARY_KEY_ID",
    "APPLE_NOTARY_ISSUER_ID",
];
const MANIFEST: [&str; 3] = [
    "RELEASE_MINISIGN_KEY",
    "RELEASE_MINISIGN_PASSWORD",
    "RELEASE_ML_DSA_KEY",
];

fn resolved(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains("${")
}

/// Required secret names for one release operation.
///
/// # Errors
///
/// Rejects an unknown credential purpose.
pub fn names(purpose: &str) -> Result<&'static [&'static str]> {
    Ok(match purpose {
        "automation" => &AUTOMATION,
        "macos" => &MACOS,
        "windows" => &WINDOWS,
        "notarize" => &NOTARIZE,
        "manifest" => &MANIFEST,
        _ => bail!("unknown credential purpose"),
    })
}

pub(super) fn inputs(purpose: &str) -> Result<()> {
    for name in names(purpose)? {
        ensure!(
            resolved(&required(name)?),
            "{name} is empty or an unresolved secret reference"
        );
    }
    Ok(())
}

pub(super) fn verify(root: &Path, purpose: &str) -> Result<()> {
    inputs(purpose)?;
    match purpose {
        "macos" => with_keychain(root, |keychain| {
            keychain.verify_identities(
                &required("APPLE_SIGNING_IDENTITY")?,
                &required("APPLE_INSTALLER_IDENTITY")?,
            )
        })?,
        "notarize" => with_notary_key(|key| crate::apple::verify_notary(root, key))?,
        "manifest" => {
            let encoded = minisign::SecretKeyBox::from_string(&required("RELEASE_MINISIGN_KEY")?)?;
            let secret = encoded.into_secret_key(Some(required("RELEASE_MINISIGN_PASSWORD")?))?;
            let public = minisign::PublicKey::from_secret_key(&secret)?.to_base64();
            ensure!(
                trust::RELEASE_KEYS.iter().any(|key| key.minisign == public),
                "the minisign key does not match the compiled trust anchors"
            );
            let ml_dsa = unhex(&required("RELEASE_ML_DSA_KEY")?)
                .context("the ML-DSA signing key is not hexadecimal")?;
            ensure!(
                ml_dsa.len() == trust::ML_DSA_SIGNING_KEY_BYTES,
                "the ML-DSA signing key has the wrong size"
            );
        }
        "automation" => {
            let reply = tool::capture(
                Tool::Gh.command(root).args([
                    "repo",
                    "view",
                    "P4suta/youarel",
                    "--json",
                    "nameWithOwner",
                ]),
                "verify the proposal App repository access",
            )?;
            let value: serde_json::Value = serde_json::from_str(&reply)?;
            ensure!(
                value
                    .get("nameWithOwner")
                    .and_then(serde_json::Value::as_str)
                    == Some("P4suta/youarel"),
                "the proposal App reached another repository"
            );
        }
        "windows" => super::esigner::verify_credentials(root)?,
        _ => bail!("unknown credential purpose"),
    }
    println!(
        "Verified {purpose} credential inputs without signing, submitting, tagging, or publishing."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolved;

    #[test]
    fn missing_or_unresolved_credential_references_are_rejected() {
        for value in [
            "",
            " \n",
            "${shared-signing.macos.APPLE_CERTIFICATE}",
            "prefix ${missing}",
        ] {
            assert!(!resolved(value));
        }
        assert!(resolved("provided value"));
    }
}
