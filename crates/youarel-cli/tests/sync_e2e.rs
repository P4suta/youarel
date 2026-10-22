//! Three `youarel` processes, each with its own home, pair through files, sync through a shared folder, and converge, and a removed device reads nothing more.
#![cfg(test)]

use std::path::Path;
use std::process::Output;
use youarel_store::{Store, fs};

/// The variable that names where a device keeps the key that seals its secrets.
const KEY_STORE: &str = "YOUAREL_KEY_STORE";
/// The key store of every device in these tests: a file beside the account.
const FILE_KEY_STORE: &str = "file";

/// A command for a program the test starts: the built program, or this test binary again.
#[expect(
    clippy::disallowed_methods,
    reason = "the end-to-end test starts the built program and itself, which it names in one place"
)]
fn command(program: &Path) -> std::process::Command {
    std::process::Command::new(program)
}

/// A command for the built program.
fn youarel() -> std::process::Command {
    command(Path::new(env!("CARGO_BIN_EXE_youarel")))
}

/// Whether the test named `test` may sync inside this process, which it may once the environment names the file key store.
///
/// A test cannot set its own environment while others run, so otherwise this runs the test again in a child of this binary whose environment names the store, and asserts that the child ran it and passed.
fn syncs_here(test: &str) -> bool {
    if std::env::var(KEY_STORE).as_deref() == Ok(FILE_KEY_STORE) {
        return true;
    }
    let output = command(&std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture"])
        .env(KEY_STORE, FILE_KEY_STORE)
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && printed.contains("1 passed"),
        "{printed}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
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
            .env(KEY_STORE, FILE_KEY_STORE)
            .output()
            .unwrap()
    }

    /// The name of the device's bundle in a shared folder.
    fn bundle(&self) -> String {
        format!("{}{}", self.id(), youarel_cli::sync::BUNDLE_SUFFIX)
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

#[test]
fn files_beside_the_bundles_count_for_nothing_and_an_unreadable_bundle_is_skipped() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    pair(&first, &second, files.path());
    first.run(&["sync", path(shared.path())]);
    let bundle = first.bundle();
    for beside in [
        format!("._{bundle}"),
        format!("{bundle}.sdtmp"),
        format!("~${bundle}"),
        String::from(".DS_Store"),
    ] {
        fs::replace(&shared.path().join(beside), b"no bundle").unwrap();
    }
    let synced = second.run(&["sync", path(shared.path())]);
    assert!(!synced.contains("skipped"), "{synced}");
    assert!(synced.contains(&format!("merged {bundle}")), "{synced}");
    fs::create_dir_all(&shared.path().join("unread.youarel")).unwrap();
    let unread = second.run(&["sync", path(shared.path())]);
    assert!(unread.contains("skipped unread.youarel"), "{unread}");
    assert!(
        unread.contains(&format!("merged {bundle}")) && unread.contains(&second.bundle()),
        "the sync goes on past the file it cannot read: {unread}"
    );
}

#[test]
fn a_merge_writes_nothing_into_the_folder_while_the_store_gains_the_other_device_s_shortcut() {
    if !syncs_here(
        "a_merge_writes_nothing_into_the_folder_while_the_store_gains_the_other_device_s_shortcut",
    ) {
        return;
    }
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    pair(&first, &second, files.path());
    first.run(&["add", "https://merged.example/", "--no-fetch"]);
    first.run(&["sync", path(shared.path())]);
    let store = Store::new(second.home.path().to_path_buf());
    let now = youarel_cli::clock::now().unwrap();
    let merged = youarel_cli::sync::merge(&store, now, path(shared.path())).unwrap();
    assert_eq!(merged, [format!("merged {}", first.bundle())]);
    assert_eq!(
        fs::list(shared.path()).unwrap(),
        [first.bundle()],
        "the merge wrote nothing into the folder"
    );
    assert!(second.run(&["find", "merged"]).contains("merged.example"));
    assert_eq!(
        youarel_cli::sync::own_bundle(&store).unwrap(),
        second.bundle()
    );
}
