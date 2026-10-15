//! Three `youarel` processes, each with its own home, pair through files, sync through a shared folder, and converge, and a removed device reads nothing more.
#![cfg(test)]

use std::path::Path;
use std::process::Output;

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

    fn output(&self, arguments: &[&str]) -> Output {
        youarel()
            .args(arguments)
            .env("YOUAREL_HOME", self.home.path())
            .env("YOUAREL_KEY_STORE", "file")
            .output()
            .unwrap()
    }

    /// Runs a command that must succeed and returns what it printed.
    fn run(&self, arguments: &[&str]) -> String {
        let output = self.output(arguments);
        assert!(
            output.status.success(),
            "youarel {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// The device's identifier, as `account show` prints it.
    fn id(&self) -> String {
        let shown = self.run(&["account", "show"]);
        shown
            .lines()
            .find(|line| line.ends_with("(this device)"))
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
            .to_owned()
    }

    fn listing(&self, folder: &str) -> Vec<String> {
        let mut lines: Vec<String> = self
            .run(&["ls", folder])
            .lines()
            .map(str::to_owned)
            .collect();
        lines.sort();
        lines
    }
}

/// Pairs `joiner` into `inviter`'s account through the join, welcome, confirmation, and entry files.
fn pair(inviter: &Device, joiner: &Device, files: &Path) {
    let shown = inviter.run(&["pair", "invite"]);
    let code = shown
        .lines()
        .find_map(|line| line.strip_prefix("invitation code "))
        .unwrap()
        .to_owned();
    let join = files.join("join");
    let welcome = files.join("welcome");
    let confirmation = files.join("confirmation");
    let entry = files.join("entry");
    joiner.run(&["pair", "join", &code, "--out", path(&join)]);
    inviter.run(&["pair", "accept", path(&join), "--out", path(&welcome)]);
    joiner.run(&[
        "pair",
        "confirm",
        path(&welcome),
        "--out",
        path(&confirmation),
    ]);
    inviter.run(&["pair", "admit", path(&confirmation), "--out", path(&entry)]);
    joiner.run(&["pair", "finish", path(&entry)]);
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn sync_all(devices: &[&Device], folder: &Path) {
    for _ in 0..2 {
        for device in devices {
            device.run(&["sync", path(folder)]);
        }
    }
}

#[test]
fn devices_pair_through_files_and_converge_through_a_shared_folder() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    first.run(&[
        "add",
        "https://first.example/",
        "--no-fetch",
        "--folder",
        "Work",
    ]);
    pair(&first, &second, files.path());
    assert_eq!(second.listing("Work"), first.listing("Work"));
    second.run(&["add", "https://second.example/", "--no-fetch"]);
    first.run(&["add", "https://third.example/", "--no-fetch"]);
    sync_all(&[&first, &second], shared.path());
    assert_eq!(first.listing("/"), second.listing("/"));
    assert!(
        first
            .listing("/")
            .iter()
            .any(|line| line.contains("second.example"))
    );
    let shown = second.run(&["account", "show"]);
    assert_eq!(
        shown
            .lines()
            .filter(|line| line.starts_with("member "))
            .count(),
        2
    );
}

#[test]
fn folders_moved_into_each_other_at_once_converge_without_losing_a_shortcut() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    first.run(&["mkdir", "Alpha"]);
    first.run(&["mkdir", "Beta"]);
    first.run(&[
        "add",
        "https://alpha.example/",
        "--no-fetch",
        "--folder",
        "Alpha",
    ]);
    first.run(&[
        "add",
        "https://beta.example/",
        "--no-fetch",
        "--folder",
        "Beta",
    ]);
    pair(&first, &second, files.path());
    first.run(&["mv", "Alpha/", "Beta"]);
    second.run(&["mv", "Beta/", "Alpha"]);
    sync_all(&[&first, &second], shared.path());
    let tree = |device: &Device| {
        let mut all = device.listing("/");
        for folder in ["Alpha", "Beta", "Alpha/Beta", "Beta/Alpha"] {
            let output = device.output(&["ls", folder]);
            if output.status.success() {
                all.push(format!(
                    "{folder}: {}",
                    String::from_utf8_lossy(&output.stdout)
                ));
            }
        }
        all
    };
    assert_eq!(tree(&first), tree(&second));
    let found = first.run(&["find", "example"]);
    assert!(
        found.contains("alpha.example") && found.contains("beta.example"),
        "{found}"
    );
}

#[test]
fn a_removed_device_receives_nothing_more_and_sends_nothing_that_counts() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second, third) = (Device::new(), Device::new(), Device::new());
    first.run(&["account", "create"]);
    pair(&first, &second, files.path());
    pair(&first, &third, files.path());
    sync_all(&[&first, &second, &third], shared.path());
    let removed = third.id();
    first.run(&["remove", removed.get(..12).unwrap()]);
    third.run(&["add", "https://late.example/", "--no-fetch"]);
    first.run(&["add", "https://secret.example/", "--no-fetch"]);
    sync_all(&[&first, &second], shared.path());
    let synced = third.output(&["sync", path(shared.path())]);
    let printed = String::from_utf8_lossy(&synced.stderr);
    assert!(!synced.status.success());
    assert!(
        printed.contains("skipped") && printed.contains("wrote no bundle"),
        "{printed}"
    );
    assert!(!third.run(&["find", "secret"]).contains("secret.example"));
    sync_all(&[&first, &second], shared.path());
    assert!(!first.run(&["find", "late"]).contains("late.example"));
    assert!(second.run(&["find", "secret"]).contains("secret.example"));
    let shown = first.run(&["account", "show"]);
    assert!(!shown.contains(&removed), "{shown}");
}
