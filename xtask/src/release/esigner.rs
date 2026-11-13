use super::{desktop, ensure_enabled, required};
use crate::fs;
use crate::package;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
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
const CERTIFICATE_LABELS: [&str; 3] = ["- Subject DN:", "- Certificate Expiry:", "- Issuer DN:"];
const CLEAN_SCAN: &str = "code object is not a malware. You can proceed signing this code object";
const BATCH_SUCCESS: &str =
    "Batch sign command executed successfully. Output directory for signed files: ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Credentials,
    Scan,
    Batch,
}

impl Operation {
    const fn command(self) -> &'static str {
        match self {
            Self::Credentials => "credential_info",
            Self::Scan => "scan_code",
            Self::Batch => "batch_sign",
        }
    }

    fn confirmed(self, text: &str) -> bool {
        if text.lines().any(|line| line.trim().starts_with("Error:")) {
            return false;
        }
        match self {
            Self::Credentials => CERTIFICATE_LABELS
                .iter()
                .all(|label| text.lines().any(|line| line.trim().starts_with(label))),
            Self::Scan => text.trim() == CLEAN_SCAN,
            Self::Batch => text.trim().strip_prefix(BATCH_SUCCESS).is_some_and(|path| {
                !path.is_empty() && !path.contains('\n') && !path.contains('\r')
            }),
        }
    }
}

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

fn private_result(output: Output, operation: Operation) -> Result<Output> {
    let problem = [
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    ]
    .iter()
    .flat_map(|text| text.lines())
    .map(super::esigner_authorization::Problem::classify)
    .find(|problem| *problem != super::esigner_authorization::Problem::Unknown)
    .unwrap_or(super::esigner_authorization::Problem::Unknown);
    ensure!(
        output.status.success(),
        "CodeSignTool {} exited with {}: {}; provider output remains private",
        operation.command(),
        output.status,
        problem.message()
    );
    ensure!(
        std::str::from_utf8(&output.stdout).is_ok_and(|text| operation.confirmed(text))
            && !String::from_utf8_lossy(&output.stderr).lines().any(|line| {
                let line = line.trim();
                line.starts_with("Error:")
                    || line.starts_with("Unknown option:")
                    || line.starts_with("Missing required")
            }),
        "CodeSignTool {} did not confirm success: {}; provider output remains private",
        operation.command(),
        problem.message()
    );
    Ok(output)
}

fn execute(root: &Path, operation: Operation, words: &[String]) -> Result<Output> {
    private_result(run(root, words)?, operation)
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
    probe_signing_parsers(root)?;
    println!(
        "Verified CodeSignTool {VERSION}, its archive digest, Java, and private argument-file transport."
    );
    Ok(())
}

fn probe_signing_parsers(root: &Path) -> Result<()> {
    let fixture = tempfile::Builder::new()
        .prefix("youarel batch probe ")
        .tempdir_in(root.join(DIRECTORY))?;
    fs::write(
        &fixture.path().join("probe.fixture"),
        b"public parser fixture",
    )?;
    let mut batch_words = jar_arguments(Operation::Batch.command());
    batch_words.extend(batch_paths(fixture.path(), &fixture.path().join("output")));
    batch_words.extend([
        "-username=public-probe".to_owned(),
        "-password=unused".to_owned(),
        "-credential_id=unused".to_owned(),
        "-totp_secret=unused".to_owned(),
    ]);
    let batch_parser = public_probe(root, &batch_words, "batch signing argument parser")?;
    ensure!(
        String::from_utf8_lossy(&batch_parser.stdout)
            .starts_with("Error: File extension not allowed: probe.fixture"),
        "the signing SDK did not read the public batch fixture before authentication"
    );
    let mut scan_words = jar_arguments(Operation::Scan.command());
    scan_words.push(format!(
        "-input_file_path={}",
        fixture.path().join("probe.fixture").display()
    ));
    scan_words.extend([
        "-username=public-probe".to_owned(),
        "-password=unused".to_owned(),
        "-credential_id=unused".to_owned(),
    ]);
    let scan_parser = public_probe(root, &scan_words, "malware scan argument parser")?;
    ensure!(
        String::from_utf8_lossy(&scan_parser.stdout)
            .starts_with("Error: Unsupported file format for signing - fixture"),
        "the signing SDK did not read the public scan fixture before authentication"
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
    let operation = Operation::Credentials;
    let result = execute(root, operation, &authenticated(operation.command())?)?;
    let text = String::from_utf8(result.stdout)?;
    for line in text.lines().filter(|line| {
        CERTIFICATE_LABELS
            .iter()
            .any(|label| line.trim().starts_with(label))
    }) {
        println!("{}", line.trim());
    }
    super::esigner_authorization::verify(
        &required(USERNAME)?,
        &required(PASSWORD)?,
        &required(CREDENTIAL)?,
        &required(TOTP)?,
    )?;
    Ok(())
}

pub(super) fn sign(root: &Path, target: &str) -> Result<()> {
    ensure_enabled()?;
    desktop::supported(target)?;
    ensure!(
        target.contains("windows"),
        "Windows signing requires a Windows target"
    );
    let source = tempfile::Builder::new()
        .prefix("youarel unsigned ")
        .tempdir()?;
    let output = root.join(SIGNING_INPUT);
    let mut programs = Vec::new();
    for path in [
        super::built_program(root, target),
        desktop::launcher(root, target),
    ] {
        let name = path.file_name().context("program file has no name")?;
        let input = source.path().join(name);
        fs::write(&input, &fs::read(&output.join(name))?)?;
        programs.push(input);
    }
    let result = scan_and_sign(&programs, source.path(), &output, |operation, paths| {
        let mut words = authenticated(operation.command())?;
        if operation == Operation::Batch {
            words.push(format!("-totp_secret={}", required(TOTP)?));
        }
        words.extend(paths);
        run(root, &words)
    });
    let cleanup = source.close();
    result?;
    cleanup.context("remove temporary unsigned signing inputs")?;
    println!("The provider operation finished; native publisher and timestamp checks follow.");
    Ok(())
}

fn scan_and_sign(
    programs: &[PathBuf],
    input: &Path,
    output: &Path,
    mut invoke: impl FnMut(Operation, Vec<String>) -> Result<Output>,
) -> Result<()> {
    ensure!(
        programs.len() == super::WINDOWS_PROGRAM_COUNT,
        "Windows signing requires both candidate programs"
    );
    for program in programs {
        private_result(
            invoke(
                Operation::Scan,
                vec![format!("-input_file_path={}", program.display())],
            )?,
            Operation::Scan,
        )?;
    }
    private_result(
        invoke(Operation::Batch, batch_paths(input, output))?,
        Operation::Batch,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn successful_output(stdout: &str) -> Output {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt as _;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt as _;
        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn both_clean_scans_precede_batch_signing() {
        let input = Path::new("unsigned files");
        let output = Path::new("signed files");
        let programs = [
            input.join("youarel.exe"),
            input.join("youarel-launcher.exe"),
        ];
        let mut calls = Vec::new();
        scan_and_sign(&programs, input, output, |operation, paths| {
            calls.push((operation, paths));
            Ok(successful_output(match operation {
                Operation::Scan => CLEAN_SCAN,
                Operation::Batch => "Batch sign command executed successfully. Output directory for signed files: signed files",
                Operation::Credentials => panic!("credential metadata is not a signing operation"),
            }))
        })
        .unwrap();
        assert_eq!(
            calls,
            [
                (
                    Operation::Scan,
                    vec![format!("-input_file_path={}", programs[0].display())],
                ),
                (
                    Operation::Scan,
                    vec![format!("-input_file_path={}", programs[1].display())],
                ),
                (Operation::Batch, batch_paths(input, output)),
            ]
        );
    }

    #[test]
    fn a_rejected_scan_prevents_any_signature_request() {
        let input = Path::new("unsigned files");
        let output = Path::new("signed files");
        let programs = [
            input.join("youarel.exe"),
            input.join("youarel-launcher.exe"),
        ];
        for rejected in 0..programs.len() {
            for response in [
                "code object is a malware. This code object cannot be signed using eSigner",
                "Error: PRIVATE_PROVIDER_REJECTION",
                "",
                "unrecognized provider response",
            ] {
                let mut calls = Vec::new();
                let result = scan_and_sign(&programs, input, output, |operation, _| {
                    calls.push(operation);
                    Ok(successful_output(if operation == Operation::Batch {
                        "Batch sign command executed successfully. Output directory for signed files: signed files"
                    } else if calls.len() - 1 == rejected {
                        response
                    } else {
                        CLEAN_SCAN
                    }))
                });
                let error = result.unwrap_err().to_string();
                assert!(error.contains("provider output remains private"));
                assert!(!error.contains("PRIVATE_PROVIDER"));
                assert_eq!(calls, vec![Operation::Scan; rejected + 1]);
            }
        }
    }

    #[test]
    fn provider_success_requires_the_requested_operations_confirmation() {
        let certificate = "- Subject DN: public subject\n- Certificate Expiry: public date\n- Issuer DN: public issuer";
        let batch = "Batch sign command executed successfully. Output directory for signed files: signed files";
        for (operation, confirmation) in [
            (Operation::Credentials, certificate),
            (Operation::Scan, CLEAN_SCAN),
            (Operation::Batch, batch),
        ] {
            private_result(successful_output(confirmation), operation).unwrap();
            for response in ["", "unrecognized response", "Error: PRIVATE_PROVIDER_ERROR"] {
                private_result(successful_output(response), operation).unwrap_err();
            }
            let conflicting = format!("{confirmation}\nError: PRIVATE_PROVIDER_ERROR");
            let error = private_result(successful_output(&conflicting), operation)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("PRIVATE_PROVIDER"));
            for other in [Operation::Credentials, Operation::Scan, Operation::Batch] {
                if other != operation {
                    private_result(successful_output(confirmation), other).unwrap_err();
                }
            }
        }
        let mut malformed = successful_output(batch);
        malformed.stdout.push(u8::MAX);
        private_result(malformed, Operation::Batch).unwrap_err();
    }

    #[test]
    fn incomplete_or_extra_programs_prevent_provider_requests() {
        let input = Path::new("unsigned files");
        let output = Path::new("signed files");
        let programs = [
            input.join("youarel.exe"),
            input.join("youarel-launcher.exe"),
            input.join("unexpected.exe"),
        ];
        for count in [0, 1, programs.len()] {
            scan_and_sign(&programs[..count], input, output, |_, _| {
                panic!("an incomplete or extra candidate must not reach the provider")
            })
            .unwrap_err();
        }
    }

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
            let error = private_result(output, Operation::Credentials)
                .unwrap_err()
                .to_string();
            assert!(error.contains("provider output remains private"));
            assert!(!error.contains("PRIVATE_PROVIDER"));
        }
    }

    #[test]
    fn zero_exit_provider_errors_remain_private() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt as _;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt as _;
        for (stdout, stderr) in [
            ("Error: PRIVATE_PROVIDER_REJECTION", ""),
            (
                "",
                "java.lang.IllegalStateException: PRIVATE_PROVIDER_EXCEPTION",
            ),
        ] {
            let output = Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            };
            let error = private_result(output, Operation::Batch)
                .unwrap_err()
                .to_string();
            assert!(error.contains("provider output remains private"));
            assert!(!error.contains("PRIVATE_PROVIDER"));
        }
    }

    #[test]
    fn clean_scan_errors_on_stderr_remain_private() {
        for stderr in [
            " \tError: PRIVATE_PROVIDER_REJECTION\n",
            " \tUnknown option: PRIVATE_PROVIDER_ARGUMENT\n",
            " \tMissing required PRIVATE_PROVIDER_ARGUMENT\n",
        ] {
            let mut output = successful_output(CLEAN_SCAN);
            output.stderr = stderr.as_bytes().to_vec();
            let error = private_result(output, Operation::Scan)
                .unwrap_err()
                .to_string();
            assert!(error.contains("provider output remains private"));
            assert!(!error.contains("PRIVATE_PROVIDER"));
        }
    }

    #[test]
    fn signing_failures_name_the_safe_category_without_private_values() {
        let output = successful_output("Error: invalid otp PRIVATE_TOTP_SECRET");
        let error = private_result(output, Operation::Batch)
            .unwrap_err()
            .to_string();
        assert!(error.contains("one-time password"));
        assert!(!error.contains("PRIVATE_TOTP_SECRET"));
    }
}
