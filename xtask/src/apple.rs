//! The Apple stages of a release: Developer ID signatures from a private temporary keychain, the installer package, and notarization.
//!
//! Passwords reach `security` through its standard input, never its arguments, and the keychain search list returns to its original order on every way out.
//! These stages run only in the release workflow's signing jobs; a workstation builds the unsigned package alone.

use crate::fs;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, bail, ensure};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The identifier of the installer package, which macOS records its receipt under.
pub const PACKAGE_IDENTIFIER: &str = "io.github.p4suta.youarel";
/// Where the package installs the program.
pub const INSTALL_LOCATION: &str = "/usr/local/bin";
/// The most arguments one `security` command line takes.
const SECURITY_ARGUMENTS: usize = 31;
/// The longest command line `security -i` reads.
const SECURITY_LINE: usize = 4094;
/// How long the temporary keychain stays unlocked, in seconds.
const UNLOCKED_SECONDS: &str = "3600";
/// How long notarization may take before the wait gives up.
const NOTARY_TIMEOUT: &str = "2h";

/// A command line for `security -i`, which holds secrets, so it is never shown.
struct Secret(Vec<u8>);

impl core::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

/// The line that `security -i` reads as `words`, each quoted.
///
/// # Errors
///
/// Returns an error when a word holds a line break or a NUL, or the line exceeds what `security` reads.
fn security_line(words: &[&str]) -> Result<Secret> {
    ensure!(
        words.len() <= SECURITY_ARGUMENTS,
        "security accepts at most {SECURITY_ARGUMENTS} arguments"
    );
    let mut line = Vec::new();
    for (index, word) in words.iter().enumerate() {
        ensure!(
            !word.bytes().any(|byte| matches!(byte, 0 | b'\n' | b'\r')),
            "a security argument holds a line break or a NUL"
        );
        if index != 0 {
            line.push(b' ');
        }
        line.push(b'"');
        for byte in word.bytes() {
            if matches!(byte, b'\\' | b'"') {
                line.push(b'\\');
            }
            line.push(byte);
        }
        line.push(b'"');
    }
    ensure!(
        line.len() <= SECURITY_LINE,
        "a security command exceeds its line limit"
    );
    line.push(b'\n');
    Ok(Secret(line))
}

/// Runs `security -i` over the lines, which never reach an argument list or a log.
fn security(root: &Path, lines: &[Secret]) -> Result<()> {
    let mut child = Tool::Security
        .command(root)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("start security")?;
    let mut input = child
        .stdin
        .take()
        .context("open the standard input of security")?;
    for line in lines {
        input.write_all(&line.0).context("write to security")?;
    }
    drop(input);
    let status = child.wait().context("wait for security")?;
    ensure!(status.success(), "security failed with {status}");
    Ok(())
}

/// The keychains of the user's search list, in order, as `security list-keychains` prints them.
fn search_list(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(|line| line.trim().trim_matches('"').to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// A certificate with its private key, as the release environment holds it: a base64 PKCS #12 export and its password.
#[derive(Clone, Copy, Debug)]
pub struct Import<'a> {
    pub base64: &'a str,
    pub password: &'a str,
}

/// A private keychain that holds the release certificates while it lives, and leaves the search list as it found it.
#[derive(Debug)]
pub struct Keychain {
    root: PathBuf,
    path: PathBuf,
    original: Vec<String>,
    _directory: tempfile::TempDir,
}

impl Keychain {
    /// A new keychain that holds the certificates and comes first in the search list.
    ///
    /// # Errors
    ///
    /// Returns an error when a certificate does not decode or `security` refuses a step.
    pub fn create(root: &Path, imports: &[Import<'_>]) -> Result<Self> {
        let directory = tempfile::tempdir().context("create the keychain directory")?;
        let location = directory.path().join("release.keychain-db");
        let password = random_password()?;
        let listing = tool::capture(
            Tool::Security
                .command(root)
                .args(["list-keychains", "-d", "user"]),
            "security list-keychains",
        )?;
        let keychain = Self {
            root: root.to_path_buf(),
            path: location,
            original: search_list(&listing),
            _directory: directory,
        };
        let path = keychain.text()?;
        let mut lines = vec![
            security_line(&["create-keychain", "-p", &password, path])?,
            security_line(&["set-keychain-settings", "-lut", UNLOCKED_SECONDS, path])?,
            security_line(&["unlock-keychain", "-p", &password, path])?,
        ];
        for (index, import) in imports.iter().enumerate() {
            let certificate = keychain
                .path
                .with_file_name(format!("certificate-{index}.p12"));
            fs::write_private(&certificate, &decode_base64(import.base64)?)?;
            let certificate = certificate
                .to_str()
                .context("the certificate path is not UTF-8")?
                .to_owned();
            lines.push(security_line(&[
                "import",
                &certificate,
                "-k",
                path,
                "-f",
                "pkcs12",
                "-P",
                import.password,
                "-T",
                tool::CODESIGN,
                "-T",
                tool::PRODUCTSIGN,
            ])?);
        }
        lines.push(security_line(&[
            "set-key-partition-list",
            "-S",
            "apple-tool:,apple:,codesign:",
            "-s",
            "-k",
            &password,
            path,
        ])?);
        let mut search: Vec<&str> = vec!["list-keychains", "-d", "user", "-s", path];
        search.extend(keychain.original.iter().map(String::as_str));
        lines.push(security_line(&search)?);
        security(root, &lines)?;
        Ok(keychain)
    }

    fn text(&self) -> Result<&str> {
        self.path.to_str().context("the keychain path is not UTF-8")
    }

    /// Signs a program with the Developer ID Application identity, with the hardened runtime and a trusted timestamp, and checks the signature.
    ///
    /// # Errors
    ///
    /// Returns an error when `codesign` refuses to sign or the signature does not verify.
    pub fn sign_program(&self, identity: &str, program: &Path) -> Result<()> {
        tool::succeed(
            Tool::Codesign
                .command(&self.root)
                .args([
                    "--force",
                    "--options",
                    "runtime",
                    "--timestamp",
                    "--keychain",
                ])
                .arg(&self.path)
                .args(["--sign", identity])
                .arg(program),
            "codesign",
        )?;
        tool::succeed(
            Tool::Codesign
                .command(&self.root)
                .args(["--verify", "--strict", "--verbose=2"])
                .arg(program),
            "codesign --verify",
        )?;
        let details = tool::output(
            Tool::Codesign
                .command(&self.root)
                .args(["--display", "--verbose=4"])
                .arg(program),
            "codesign --display",
        )?;
        let text = String::from_utf8_lossy(&details.stderr);
        ensure!(
            signed_for_distribution(&text),
            "{} lacks the hardened runtime or a timestamp:\n{text}",
            program.display()
        );
        Ok(())
    }

    /// Signs an installer package with the Developer ID Installer identity and a timestamp, and checks the signature.
    ///
    /// # Errors
    ///
    /// Returns an error when `productsign` refuses or `pkgutil` finds no trusted signature.
    pub fn sign_package(&self, identity: &str, unsigned: &Path, signed: &Path) -> Result<()> {
        tool::succeed(
            Tool::Productsign
                .command(&self.root)
                .args(["--sign", identity, "--timestamp", "--keychain"])
                .arg(&self.path)
                .arg(unsigned)
                .arg(signed),
            "productsign",
        )?;
        tool::succeed(
            Tool::Pkgutil
                .command(&self.root)
                .arg("--check-signature")
                .arg(signed),
            "pkgutil --check-signature",
        )
    }
}

impl Drop for Keychain {
    fn drop(&mut self) {
        // A failure here leaves the keychain in the search list, so it is reported, though the primary error stays the caller's.
        let mut restore: Vec<&str> = vec!["list-keychains", "-d", "user", "-s"];
        restore.extend(self.original.iter().map(String::as_str));
        let restored = security_line(&restore).and_then(|line| security(&self.root, &[line]));
        let deleted = self
            .text()
            .and_then(|path| security_line(&["delete-keychain", path]))
            .and_then(|line| security(&self.root, &[line]));
        for (what, result) in [
            ("restore the keychain search list", restored),
            ("delete the release keychain", deleted),
        ] {
            if let Err(error) = result {
                eprintln!("release: cannot {what}: {error:#}");
            }
        }
    }
}

/// Whether `codesign --display` reports the hardened runtime and a trusted timestamp.
fn signed_for_distribution(details: &str) -> bool {
    let runtime = details
        .lines()
        .any(|line| line.starts_with("CodeDirectory") && line.contains("runtime"));
    let timestamp = details.lines().any(|line| line.starts_with("Timestamp="));
    runtime && timestamp
}

/// Thirty-two random bytes in hexadecimal, a password nobody types.
fn random_password() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("draw a keychain password: {error}"))?;
    Ok(crate::hex(&bytes))
}

/// The bytes a standard base64 text spells, line breaks ignored.
fn decode_base64(text: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let compact: String = text.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .context("a certificate export is not base64")
}

/// The unsigned installer package of a program: a component that installs it in [`INSTALL_LOCATION`], wrapped as a product archive.
///
/// # Errors
///
/// Returns an error when `pkgbuild` or `productbuild` fails.
pub fn build_package(root: &Path, program: &Path, version: &str, output: &Path) -> Result<()> {
    let stage = tempfile::tempdir().context("create the package stage")?;
    let payload = stage.path().join("payload");
    fs::create_dir_all(&payload)?;
    fs::write_executable(&payload.join("youarel"), &fs::read(program)?)?;
    let component = stage.path().join("youarel-component.pkg");
    tool::succeed(
        Tool::Pkgbuild
            .command(root)
            .env("COPYFILE_DISABLE", "1")
            .arg("--root")
            .arg(&payload)
            .args(["--identifier", PACKAGE_IDENTIFIER, "--version", version])
            .args([
                "--install-location",
                INSTALL_LOCATION,
                "--ownership",
                "recommended",
            ])
            .arg(&component),
        "pkgbuild",
    )?;
    tool::succeed(
        Tool::Productbuild
            .command(root)
            .arg("--package")
            .arg(&component)
            .args(["--identifier", PACKAGE_IDENTIFIER, "--version", version])
            .arg(output),
        "productbuild",
    )
}

/// The files a package installs, as `pkgutil --payload-files` lists them.
///
/// # Errors
///
/// Returns an error when `pkgutil` cannot read the package.
pub fn payload_files(root: &Path, package: &Path) -> Result<Vec<String>> {
    let listing = tool::capture(
        Tool::Pkgutil
            .command(root)
            .arg("--payload-files")
            .arg(package),
        "pkgutil --payload-files",
    )?;
    Ok(listing.lines().map(str::to_owned).collect())
}

/// What the notary service said about a submission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Notarized {
    Accepted,
    /// Still in progress when the wait ended.
    Pending,
    Rejected(String),
}

/// The App Store Connect API key that authenticates notarization: the private key file, its identifier, and its issuer.
#[derive(Clone, Copy, Debug)]
pub struct NotaryKey<'a> {
    pub file: &'a Path,
    pub id: &'a str,
    pub issuer: &'a str,
}

fn notary(root: &Path, key: &NotaryKey<'_>) -> std::process::Command {
    let mut command = Tool::Xcrun.command(root);
    command.arg("notarytool").arg("--key").arg(key.file).args([
        "--key-id",
        key.id,
        "--issuer",
        key.issuer,
        "--output-format",
        "json",
    ]);
    command
}

/// The submission identifier of a `notarytool submit` reply.
fn submission(reply: &str) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(reply).context("notarytool printed no JSON")?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .context("notarytool named no submission")?;
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-'),
        "notarytool named a malformed submission `{id}`"
    );
    Ok(id.to_owned())
}

/// The state a `notarytool wait` or `info` reply reports.
fn state(reply: &str) -> Result<Notarized> {
    let value: serde_json::Value =
        serde_json::from_str(reply).context("notarytool printed no JSON")?;
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .context("notarytool named no status")?;
    Ok(match status {
        "Accepted" => Notarized::Accepted,
        "In Progress" => Notarized::Pending,
        other => Notarized::Rejected(other.to_owned()),
    })
}

/// Submits a signed package for notarization and returns its submission identifier, without waiting.
///
/// # Errors
///
/// Returns an error when `notarytool` refuses the submission or names none.
pub fn submit(root: &Path, key: &NotaryKey<'_>, package: &Path) -> Result<String> {
    let mut command = notary(root, key);
    command.args(["submit", "--no-wait"]);
    command.arg(package);
    submission(&tool::capture(&mut command, "notarytool submit")?)
}

/// Waits for a submission, and staples and checks the package once Apple accepts it.
///
/// # Errors
///
/// Returns an error when the submission is rejected, still pending after the wait, or the staple or the Gatekeeper assessment fails.
pub fn finish(root: &Path, key: &NotaryKey<'_>, id: &str, package: &Path) -> Result<()> {
    let mut command = notary(root, key);
    command.args(["wait", id, "--timeout", NOTARY_TIMEOUT]);
    match state(&tool::capture(&mut command, "notarytool wait")?)? {
        Notarized::Accepted => {}
        Notarized::Pending => {
            bail!("submission {id} is still in progress; run the finish step again to resume it")
        }
        Notarized::Rejected(status) => bail!("Apple did not accept submission {id}: {status}"),
    }
    tool::succeed(
        Tool::Xcrun
            .command(root)
            .args(["stapler", "staple"])
            .arg(package),
        "stapler staple",
    )?;
    tool::succeed(
        Tool::Xcrun
            .command(root)
            .args(["stapler", "validate"])
            .arg(package),
        "stapler validate",
    )?;
    tool::succeed(
        Tool::Spctl
            .command(root)
            .args(["--assess", "--type", "install", "--verbose=2"])
            .arg(package),
        "spctl --assess",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Notarized, search_list, security_line, signed_for_distribution, state, submission,
    };

    #[test]
    fn security_lines_quote_every_word_and_refuse_line_breaks() {
        let line = security_line(&["import", "a \"b\" \\c", "-P", "pw"]).unwrap();
        assert_eq!(line.0, b"\"import\" \"a \\\"b\\\" \\\\c\" \"-P\" \"pw\"\n");
        security_line(&["unlock-keychain", "-p", "a\nb"]).unwrap_err();
        security_line(&["x"; 32]).unwrap_err();
        security_line(&[&"x".repeat(5000)]).unwrap_err();
    }

    #[test]
    fn the_search_list_reads_back_in_order() {
        let listing = "    \"/Users/a/Library/Keychains/login.keychain-db\"\n    \"/Library/Keychains/System.keychain\"\n";
        assert_eq!(
            search_list(listing),
            [
                "/Users/a/Library/Keychains/login.keychain-db",
                "/Library/Keychains/System.keychain"
            ]
        );
    }

    #[test]
    fn a_distribution_signature_needs_the_runtime_and_a_timestamp() {
        let signed = "Identifier=youarel\nCodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1\nTimestamp=Oct 15, 2026\n";
        assert!(signed_for_distribution(signed));
        assert!(!signed_for_distribution(
            "CodeDirectory v=20500 flags=0x0(none)\nTimestamp=Oct 15, 2026\n"
        ));
        assert!(!signed_for_distribution(
            "CodeDirectory v=20500 flags=0x10000(runtime)\nSigned Time=Oct 15, 2026\n"
        ));
    }

    #[test]
    fn notary_replies_name_a_submission_and_a_state() {
        let id = "2efe2717-52ef-43a5-96dc-0797e4ca1041";
        assert_eq!(
            submission(&format!("{{\"id\":\"{id}\",\"message\":\"ok\"}}")).unwrap(),
            id
        );
        submission("{\"id\":\"../x\"}").unwrap_err();
        submission("not json").unwrap_err();
        assert_eq!(
            state("{\"status\":\"Accepted\"}").unwrap(),
            Notarized::Accepted
        );
        assert_eq!(
            state("{\"status\":\"In Progress\"}").unwrap(),
            Notarized::Pending
        );
        assert_eq!(
            state("{\"status\":\"Invalid\"}").unwrap(),
            Notarized::Rejected(String::from("Invalid"))
        );
    }
}
