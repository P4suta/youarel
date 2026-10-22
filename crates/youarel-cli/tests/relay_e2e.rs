//! `youarel` processes pair and sync through a relay served over HTTP on this host, and a removed device reaches nothing there.
#![cfg(test)]

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Output, Stdio};

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

    fn command(&self, arguments: &[&str]) -> std::process::Command {
        let mut command = youarel();
        command
            .args(arguments)
            .env("YOUAREL_HOME", self.home.path());
        command
    }

    fn output(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().unwrap()
    }

    fn run(&self, arguments: &[&str]) -> String {
        let output = self.output(arguments);
        assert!(
            output.status.success(),
            "youarel {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn listing(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.run(&["ls", "/"]).lines().map(str::to_owned).collect();
        lines.sort();
        lines
    }
}

/// Pairs `joiner` into `inviter`'s account through the relay: the inviter waits there while the joiner answers its code.
fn pair(inviter: &Device, joiner: &Device, relay: &str) {
    let mut waiting: Child = inviter
        .command(&["pair", "invite", "--relay", relay])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(waiting.stdout.take().unwrap()).lines();
    let code = lines
        .find_map(|line| {
            line.unwrap()
                .strip_prefix("invitation code ")
                .map(str::to_owned)
        })
        .unwrap();
    joiner.run(&["pair", "join", &code, "--relay", relay]);
    let done = waiting.wait_with_output().unwrap();
    assert!(
        done.status.success(),
        "the inviter failed: {}",
        String::from_utf8_lossy(&done.stderr)
    );
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

#[test]
fn devices_pair_and_converge_through_a_relay_and_a_removed_device_reaches_nothing() {
    let (_local, relay) = relay();
    let (first, second, third) = (Device::new(), Device::new(), Device::new());
    first.run(&["account", "create"]);
    first.run(&["add", "https://first.example/", "--no-fetch"]);
    pair(&first, &second, &relay);
    assert_eq!(second.listing(), first.listing());
    pair(&first, &third, &relay);
    second.run(&["add", "https://second.example/", "--no-fetch"]);
    for device in [&first, &second, &third, &first, &second, &third] {
        device.run(&["sync", &relay]);
    }
    assert_eq!(first.listing(), second.listing());
    assert_eq!(third.listing(), first.listing());
    assert!(
        third
            .listing()
            .iter()
            .any(|line| line.contains("second.example"))
    );
    let shown = third.run(&["account", "show"]);
    let removed = shown
        .lines()
        .find(|line| line.ends_with("(this device)"))
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .to_owned();
    first.run(&["remove", removed.get(..12).unwrap()]);
    first.run(&["add", "https://secret.example/", "--no-fetch"]);
    first.run(&["sync", &relay]);
    let refused = third.output(&["sync", &relay]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("no member"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    second.run(&["sync", &relay]);
    assert!(
        second
            .listing()
            .iter()
            .any(|line| line.contains("secret.example"))
    );
    assert!(
        !third
            .listing()
            .iter()
            .any(|line| line.contains("secret.example"))
    );
}

#[test]
fn a_relay_over_plain_http_elsewhere_is_refused() {
    let device = Device::new();
    device.run(&["account", "create"]);
    let refused = device.output(&["sync", "http://relay.example/"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("https"));
}
