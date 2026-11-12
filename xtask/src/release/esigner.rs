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
const RUNTIME_JAR: &str = "codesign.jar";
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

fn run(root: &Path, words: &[String]) -> Result<Output> {
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
    fs::write(&directory.path().join(RUNTIME_JAR), &expected)?;
    fs::create_dir_all(&directory.path().join("conf"))?;
    fs::write(&directory.path().join(CONFIG), &config)?;
    let output = tool::output(
        command(directory.path(), &input).env("CODE_SIGN_TOOL_PATH", directory.path()),
        "run the verified signing tool through a private argument file",
    );
    let cleanup = directory.close();
    let output = output?;
    cleanup.context("remove private signing arguments and provider logs")?;
    Ok(output)
}

fn private_result(output: Output) -> Result<Output> {
    ensure!(
        output.status.success(),
        "the signing tool exited with {}; provider output remains private",
        output.status
    );
    ensure!(
        !String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line.starts_with("Unknown option:") || line.starts_with("Missing required")),
        "the signing tool rejected its arguments; provider output remains private"
    );
    Ok(output)
}

fn execute(root: &Path, words: &[String]) -> Result<Output> {
    private_result(run(root, words)?)
}

fn public_probe(root: &Path, words: &[String], name: &str) -> Result<Output> {
    let output = run(root, words)?;
    ensure!(
        output.status.success(),
        "the public {name} probe exited with {}:\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn jar_arguments(operation: &str) -> Vec<String> {
    vec![
        HEAP.to_owned(),
        "-jar".to_owned(),
        RUNTIME_JAR.to_owned(),
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
    let transport_probe = public_probe(
        root,
        &[
            format!("-Dyouarel.argumentProbe={probe}"),
            "-XshowSettings:properties".to_owned(),
            "-version".to_owned(),
        ],
        "Java argument transport",
    )?;
    ensure!(
        String::from_utf8_lossy(&transport_probe.stderr)
            .contains(&format!("youarel.argumentProbe = {probe}")),
        "Java did not preserve private argument-file values"
    );
    let sdk_version = public_probe(root, &jar_arguments("--version"), "SDK startup")?;
    ensure!(
        String::from_utf8_lossy(&sdk_version.stdout).trim() == REPORTED_VERSION,
        "the signing SDK reports another version"
    );
    probe_batch(root)?;
    println!(
        "Verified CodeSignTool {VERSION}, its archive digest, Java, and private argument-file transport."
    );
    Ok(())
}

fn probe_batch(root: &Path) -> Result<()> {
    let fixture = tempfile::Builder::new()
        .prefix("youarel batch probe ")
        .tempdir_in(root.join(DIRECTORY))?;
    fs::write(
        &fixture.path().join("probe.fixture"),
        b"public parser fixture",
    )?;
    let mut words = jar_arguments("batch_sign");
    words.extend(batch_paths(fixture.path(), &fixture.path().join("output")));
    words.extend([
        "-username=public-probe".to_owned(),
        "-password=unused".to_owned(),
        "-credential_id=unused".to_owned(),
        "-totp_secret=unused".to_owned(),
    ]);
    let parser = public_probe(root, &words, "batch signing argument parser")?;
    ensure!(
        String::from_utf8_lossy(&parser.stdout)
            .starts_with("Error: File extension not allowed: probe.fixture"),
        "the signing SDK did not read the public batch fixture before authentication"
    );
    Ok(())
}

fn authenticated(operation: &str) -> Result<Vec<String>> {
    super::credentials::inputs("windows")?;
    let mut words = jar_arguments(operation);
    for (option, input) in [
        ("username", USERNAME),
        ("password", PASSWORD),
        ("credential_id", CREDENTIAL),
    ] {
        words.push(format!("-{option}={}", required(input)?));
    }
    Ok(words)
}

fn batch_paths(input: &Path, output: &Path) -> Vec<String> {
    vec![
        format!("-input_dir_path={}", input.display()),
        format!("-output_dir_path={}", output.display()),
    ]
}

pub(super) fn verify_credentials(root: &Path) -> Result<()> {
    let result = execute(root, &authenticated("credential_info")?)?;
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
    let mut words = authenticated("batch_sign")?;
    words.push(format!("-totp_secret={}", required(TOTP)?));
    let source = tempfile::Builder::new()
        .prefix("youarel unsigned ")
        .tempdir()?;
    let output = root.join(SIGNING_INPUT);
    for path in [
        super::built_program(root, target),
        desktop::launcher(root, target),
    ] {
        let name = path.file_name().context("program file has no name")?;
        fs::write(&source.path().join(name), &fs::read(&output.join(name))?)?;
    }
    words.extend(batch_paths(source.path(), &output));
    let result = execute(root, &words);
    let cleanup = source.close();
    result?;
    cleanup.context("remove temporary unsigned signing inputs")?;
    println!("The provider operation finished; native publisher and timestamp checks follow.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_sign_names_distinct_input_and_output_without_single_file_options() {
        assert_eq!(
            batch_paths(Path::new("unsigned files"), Path::new("signed files")),
            [
                "-input_dir_path=unsigned files",
                "-output_dir_path=signed files",
            ]
        );
    }

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

    #[test]
    fn the_sdk_jar_uses_its_private_working_directory() {
        let words = jar_arguments("--version");
        let [_, _, jar, _] = words.as_slice() else {
            panic!("the SDK invocation must name its JAR");
        };
        assert!(
            Path::new(jar).is_relative(),
            "Java must not receive a host's canonical JAR path"
        );
    }

    #[test]
    fn credential_failures_keep_captured_provider_output_private() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt as _;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt as _;
        #[cfg(unix)]
        const FAILURE_STATUS: i32 = 256;
        #[cfg(windows)]
        const FAILURE_STATUS: u32 = 1;
        for (status, stdout, stderr) in [
            (
                FAILURE_STATUS,
                "PRIVATE_PROVIDER_STDOUT",
                "PRIVATE_PROVIDER_STDERR",
            ),
            (0, "", "Unknown option: PRIVATE_PROVIDER_ARGUMENT"),
        ] {
            let output = Output {
                status: std::process::ExitStatus::from_raw(status),
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            };
            let error = private_result(output).unwrap_err().to_string();
            assert!(error.contains("provider output remains private"));
            assert!(!error.contains("PRIVATE_PROVIDER"));
        }
    }
}
