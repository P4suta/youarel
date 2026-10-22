//! A `youarel` process on a new device recovers the account with a recovery key alone, from a bundle file, a shared folder, or a relay, after every other device is gone.
#![cfg(test)]

use std::io::Write as _;
use std::path::Path;
use std::process::{Output, Stdio};

/// A command for the built program.
#[expect(
    clippy::disallowed_methods,
    reason = "the end-to-end test starts the built program itself, which it names in one place"
)]
fn youarel() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_youarel"))
}

struct Device {
    home: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// Runs a command with `input` on its standard input.
    fn output(&self, arguments: &[&str], input: &str) -> Output {
        let mut child = youarel()
            .args(arguments)
            .env("YOUAREL_HOME", self.home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn run_with(&self, arguments: &[&str], input: &str) -> String {
        let output = self.output(arguments, input);
        assert!(
            output.status.success(),
            "youarel {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn run(&self, arguments: &[&str]) -> String {
        self.run_with(arguments, "")
    }

    /// Founds an account with a shortcut and a recovery key, and returns the key.
    fn found_with_recovery(&self) -> String {
        self.run(&["account", "create"]);
        self.run(&["add", "https://kept.example/", "--no-fetch"]);
        let shown = self.run(&["recovery", "create"]);
        shown
            .lines()
            .find_map(|line| line.strip_prefix("recovery key "))
            .unwrap()
            .to_owned()
    }

    fn listing(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.run(&["ls", "/"]).lines().map(str::to_owned).collect();
        lines.sort();
        lines
    }

    fn id(&self) -> String {
        self.run(&["account", "show"])
            .lines()
            .find(|line| line.ends_with("(this device)"))
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
            .to_owned()
    }
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// The variable that names an external relay for the test.
const RELAY_URL: &str = "YOUAREL_RELAY_URL";

/// The relay the test talks to: the one `YOUAREL_RELAY_URL` names, such as the Worker under `wrangler dev`, or else one served in memory here.
fn relay() -> (Option<youarel_relay::local::Local>, String) {
    if let Ok(url) = std::env::var(RELAY_URL) {
        return (None, url);
    }
    let local =
        youarel_relay::local::serve("127.0.0.1:0", || youarel_cli::clock::now().unwrap()).unwrap();
    let url = format!("http://127.0.0.1:{}", local.port());
    (Some(local), url)
}

/// What a refused command printed on its standard error.
fn refusal(output: &Output) -> String {
    assert!(!output.status.success());
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_recovery_key_brings_a_new_device_in_from_a_bundle_and_from_a_shared_folder() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let lost = Device::new();
    let key = lost.found_with_recovery();
    assert!(lost.run(&["account", "show"]).contains("(recovery key)"));
    let bundle = files.path().join("bundle");
    lost.run(&["bundle", "export", path(&bundle)]);
    lost.run(&["sync", path(shared.path())]);
    let wrong = Device::new();
    // The last symbol carries padding that a key never sets, so a 1 there differs from every key.
    let mistyped = format!("{}1", key.get(..key.len() - 1).unwrap());
    assert!(
        refusal(&wrong.output(&["recover", "--bundle", path(&bundle)], &mistyped))
            .contains("no recovery key")
    );
    let from_bundle = Device::new();
    from_bundle.run_with(&["recover", "--bundle", path(&bundle)], &key);
    assert_eq!(from_bundle.listing(), lost.listing());
    let from_folder = Device::new();
    let recovered = from_folder.run_with(&["recover", "--folder", path(shared.path())], &key);
    assert!(recovered.contains("recovered the account"), "{recovered}");
    assert_eq!(from_folder.listing(), lost.listing());
    from_folder.run(&["add", "https://after.example/", "--no-fetch"]);
    from_folder.run(&["remove", lost.id().get(..12).unwrap()]);
    from_folder.run(&["sync", path(shared.path())]);
    assert!(refusal(&lost.output(&["sync", path(shared.path())], "")).contains("no key"));
    let again = refusal(&from_bundle.output(&["recover", "--bundle", path(&bundle)], &key));
    assert!(again.contains("already"), "{again}");
}

#[test]
fn a_recovery_key_brings_a_new_device_in_through_a_relay() {
    let (_local, relay) = relay();
    let lost = Device::new();
    let key = lost.found_with_recovery();
    let early = Device::new();
    assert!(
        refusal(&early.output(&["recover", "--relay", &relay], &key)).contains("found no account")
    );
    lost.run(&["sync", &relay]);
    let fresh = Device::new();
    fresh.run_with(&["recover", "--relay", &relay], &key);
    assert_eq!(fresh.listing(), lost.listing());
    fresh.run(&["remove", lost.id().get(..12).unwrap()]);
    fresh.run(&["sync", &relay]);
    assert!(refusal(&lost.output(&["sync", &relay], "")).contains("no member"));
    let other = Device::new().found_with_recovery();
    assert!(
        refusal(&Device::new().output(&["recover", "--relay", &relay], &other))
            .contains("found no account")
    );
}
