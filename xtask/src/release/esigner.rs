use super::{desktop, ensure_enabled, required};
use crate::fs;
use crate::package;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::process::{Command, Output};
use youarel_release::manifest::Digest;

const DIRECTORY: &str = "release-tools/codesign";
const ARCHIVE: &str = "codesign.zip";
const URL: &str =
    "https://github.com/SSLcom/CodeSignTool/releases/download/v1.3.2/CodeSignTool-v1.3.2.zip";
const ARCHIVE_SHA256: &str = "f14b1e1ef14bfa1fd00279c363aab0debbf5dcfba0e4bcdce5d22bb771de0e3a";
const JAR: &str = "jar/code_sign_tool-1.3.2.jar";
const CONFIG: &str = "conf/code_sign_tool.properties";
const VERSION: &str = "1.3.2";
const REPORTED_VERSION: &str = "v1.3.0";
const PRODUCTION_AUTH: &str = "OAUTH2_ENDPOINT=https://login.ssl.com/oauth2/token";
const PRODUCTION_SIGNING: &str = "CSC_API_ENDPOINT=https://cs.ssl.com";
const DOWNLOAD_SECONDS: &str = "90";
const MAX_ARCHIVE_BYTES: &str = "40000000";
const SIGNING_INPUT: &str = "signing-input";
const HEAP: &str = "-Xmx1024m";
const MIN_JAVA_MAJOR: u32 = 17;
const USERNAME: &str = "SSLDOTCOM_USERNAME";
const PASSWORD: &str = "SSLDOTCOM_PASSWORD";
const CREDENTIAL: &str = "SSLDOTCOM_CREDENTIAL_ID";
const TOTP: &str = "SSLDOTCOM_TOTP_SECRET";
const INPUTS: [&str; 4] = [USERNAME, PASSWORD, CREDENTIAL, TOTP];
const JAVA_OPTIONS: [&str; 3] = ["JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS"];

fn arguments(words: &[String]) -> Result<Vec<u8>> {
    let mut output = String::new();
    for word in words {
        ensure!(!word.contains('\0'), "a Java argument contains a NUL");
        output.push('"');
        for character in word.chars() {
            match character {
                '\\' => output.push_str("\\\\"),
                '"' => output.push_str("\\\""),
                '\n' => output.push_str("\\n"),
                '\r' => output.push_str("\\r"),
                '\t' => output.push_str("\\t"),
                '\u{c}' => output.push_str("\\f"),
                other => output.push(other),
            }
        }
        output.push_str("\"\n");
    }
    Ok(output.into_bytes())
}

fn command(root: &Path, input: &Path) -> Command {
    let mut command = Tool::Java.command(root);
    command.arg(format!("@{}", input.display()));
    for name in JAVA_OPTIONS.into_iter().chain(INPUTS) {
        command.env_remove(name);
    }
    command
}

fn bundle(root: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let bytes = fs::read(&root.join(DIRECTORY).join(ARCHIVE))?;
    ensure!(
        Digest::of(&bytes).hex() == ARCHIVE_SHA256,
        "the CodeSignTool archive changed"
    );
    let entries = package::read_zip(&bytes)?;
    let jar = entries
        .iter()
        .find(|entry| entry.path == JAR)
        .context("the SDK lacks its pinned JAR")?;
    let config = entries
        .iter()
        .find(|entry| entry.path == CONFIG)
        .context("the SDK lacks its production configuration")?;
    let text = std::str::from_utf8(&config.bytes)?;
    ensure!(
        text.contains(PRODUCTION_AUTH) && text.contains(PRODUCTION_SIGNING),
        "the SDK does not select the production provider"
    );
    Ok((jar.bytes.clone(), config.bytes.clone()))
}

fn execute(root: &Path, words: &[String]) -> Result<Output> {
    let (expected, config) = bundle(root)?;
    ensure!(
        Digest::of(&fs::read(&root.join(DIRECTORY).join(JAR))?) == Digest::of(&expected),
        "the prepared signing JAR changed"
    );
    let directory = tempfile::Builder::new()
        .prefix("youarel signing ")
        .tempdir()?;
    let input = directory.path().join("arguments");
    fs::write_private(&input, &arguments(words)?)?;
    fs::create_dir_all(&directory.path().join("conf"))?;
    fs::write(&directory.path().join(CONFIG), &config)?;
    let output = tool::output(
        command(directory.path(), &input).env("CODE_SIGN_TOOL_PATH", directory.path()),
        "run the verified signing tool through a private argument file",
    );
    let cleanup = directory.close();
    let output = output?;
    cleanup.context("remove private signing arguments and provider logs")?;
    ensure!(
        output.status.success(),
        "the signing tool exited with {}; provider output remains private",
        output.status
    );
    Ok(output)
}

fn jar_arguments(root: &Path, operation: &str) -> Vec<String> {
    vec![
        HEAP.to_owned(),
        "-jar".to_owned(),
        root.join(DIRECTORY)
            .join(JAR)
            .to_string_lossy()
            .into_owned(),
        operation.to_owned(),
    ]
}

pub(super) fn prepare(root: &Path) -> Result<()> {
    let directory = root.join(DIRECTORY);
    fs::create_dir_all(&directory)?;
    let archive = directory.join(ARCHIVE);
    if !fs::is_file(&archive)? {
        let download = tempfile::tempdir()?;
        let path = download.path().join(ARCHIVE);
        tool::succeed(
            Tool::Curl
                .command(root)
                .args([
                    "--disable",
                    "--fail",
                    "--location",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--max-time",
                    DOWNLOAD_SECONDS,
                    "--max-filesize",
                    MAX_ARCHIVE_BYTES,
                    "--output",
                ])
                .arg(&path)
                .arg(URL),
            "download the pinned public CodeSignTool archive",
        )?;
        let bytes = fs::read(&path)?;
        ensure!(
            Digest::of(&bytes).hex() == ARCHIVE_SHA256,
            "the downloaded CodeSignTool archive does not match its pin"
        );
        fs::write(&archive, &bytes)?;
    }
    let (jar, _) = bundle(root)?;
    fs::create_dir_all(&directory.join("jar"))?;
    fs::write(&directory.join(JAR), &jar)?;
    let java = tool::capture(
        Tool::Java.command(root).arg("--version"),
        "inspect the maintained Java runtime",
    )?;
    let major = java
        .split_whitespace()
        .nth(1)
        .and_then(|word| word.trim_matches('"').split('.').next())
        .context("Java did not report a release number")?
        .parse::<u32>()
        .context("Java reported an invalid release number")?;
    ensure!(
        major >= MIN_JAVA_MAJOR,
        "CodeSignTool requires the runner's maintained Java 17 or later"
    );
    let probe = "space \"quotes\" \\slash #";
    let transport_probe = execute(
        root,
        &[
            format!("-Dyouarel.argumentProbe={probe}"),
            "-XshowSettings:properties".to_owned(),
            "-version".to_owned(),
        ],
    )?;
    ensure!(
        String::from_utf8_lossy(&transport_probe.stderr)
            .contains(&format!("youarel.argumentProbe = {probe}")),
        "Java did not preserve private argument-file values"
    );
    let sdk_version = execute(root, &jar_arguments(root, "--version"))?;
    ensure!(
        String::from_utf8_lossy(&sdk_version.stdout).trim() == REPORTED_VERSION,
        "the signing SDK reports another version"
    );
    println!(
        "Verified CodeSignTool {VERSION}, its archive digest, Java, and private argument-file transport."
    );
    Ok(())
}

fn authenticated(root: &Path, operation: &str) -> Result<Vec<String>> {
    super::credentials::inputs("windows")?;
    let mut words = jar_arguments(root, operation);
    for (option, input) in [
        ("username", USERNAME),
        ("password", PASSWORD),
        ("credential_id", CREDENTIAL),
    ] {
        words.push(format!("-{option}={}", required(input)?));
    }
    Ok(words)
}

pub(super) fn verify_credentials(root: &Path) -> Result<()> {
    let result = execute(root, &authenticated(root, "credential_info")?)?;
    let text = String::from_utf8(result.stdout)?;
    let labels = ["- Subject DN:", "- Certificate Expiry:", "- Issuer DN:"];
    ensure!(
        labels
            .iter()
            .all(|label| text.lines().any(|line| line.trim().starts_with(label))),
        "the provider did not confirm certificate metadata; inspect authentication without another signature request"
    );
    for line in text
        .lines()
        .filter(|line| labels.iter().any(|label| line.trim().starts_with(label)))
    {
        println!("{}", line.trim());
    }
    Ok(())
}

pub(super) fn sign(root: &Path, target: &str) -> Result<()> {
    ensure_enabled()?;
    desktop::supported(target)?;
    ensure!(
        target.contains("windows"),
        "Windows signing requires a Windows target"
    );
    let mut words = authenticated(root, "batch_sign")?;
    words.push(format!("-totp_secret={}", required(TOTP)?));
    words.push(format!(
        "-input_dir_path={}",
        root.join(SIGNING_INPUT).display()
    ));
    words.push("-override=true".to_owned());
    execute(root, &words)?;
    println!("The provider operation finished; native publisher and timestamp checks follow.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_receives_only_a_private_file_path_and_no_inherited_signing_options() {
        let command = command(Path::new("."), Path::new("private input/arguments"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["@private input/arguments"]);
        for name in JAVA_OPTIONS.into_iter().chain(INPUTS) {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
    }

    #[test]
    fn java_argument_files_quote_spaces_metacharacters_and_line_breaks() {
        let encoded = arguments(&["space \"quote\" \\ #\n\r\t\u{c}".to_owned()]).unwrap();
        assert_eq!(encoded, b"\"space \\\"quote\\\" \\\\ #\\n\\r\\t\\f\"\n");
        arguments(&["bad\0argument".to_owned()]).unwrap_err();
    }
}
