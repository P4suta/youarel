//! Three `youarel` processes, each with its own home, pair through files, sync through a shared folder, and converge, and a removed device reads nothing more.
#![cfg(test)]

use std::path::Path;
use std::process::Output;
use youarel_core::state::{Edit, Entry, ItemId};
use youarel_store::{Store, fs};

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
            .output()
            .unwrap()
    }

    /// The name of the device's chain directory.
    fn bundle(&self) -> String {
        self.id()
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
fn a_single_local_edit_appends_one_delta_and_an_idle_sync_writes_nothing() {
    use youarel_core::folder_rules::{HEAD_FILE, segment_name};
    use youarel_core::record_rules;
    use youarel_core::segment_rules::{Kind, Sequence};
    use youarel_protocol::segment::Head;
    use youarel_sync::account::Account;
    const LONGEST_HEAD: u64 = 128;
    const LONGEST_SEGMENT: u64 = 64 * 1024 * 1024;
    let shared = tempfile::tempdir().unwrap();
    let first = Device::new();
    first.run(&["account", "create"]);
    first.run(&["add", "https://first.example/", "--no-fetch"]);
    first.run(&["sync", path(shared.path())]);
    let peer = shared.path().join(first.id());
    let head_path = peer.join(HEAD_FILE);
    let head = Head::decode(&fs::read_bounded(&head_path, LONGEST_HEAD).unwrap()).unwrap();
    assert_eq!(head.latest(), Sequence::FIRST);
    let original = fs::read_bounded(
        &peer.join(segment_name(head.latest()).as_str()),
        LONGEST_SEGMENT,
    )
    .unwrap();
    first.run(&["add", "https://second.example/", "--no-fetch"]);
    first.run(&["sync", path(shared.path())]);
    let later = fs::read_bounded(&head_path, LONGEST_HEAD).unwrap();
    let next = Head::decode(&later).unwrap();
    assert_eq!(next.latest(), Sequence::new(2).unwrap());
    let bytes = fs::read_bounded(
        &peer.join(segment_name(next.latest()).as_str()),
        LONGEST_SEGMENT,
    )
    .unwrap();
    let opened = Account::open(&first.home.path().join("account"))
        .unwrap()
        .open_segment(&bytes)
        .unwrap();
    assert_eq!(opened.header.kind(), Kind::Delta);
    let body = record_rules::decode_body(&opened.body).unwrap();
    assert_eq!(body.shortcuts.len(), 1);
    assert_eq!(body.shortcuts[0].1.link.value, "https://second.example/");
    first.run(&["sync", path(shared.path())]);
    assert_eq!(fs::read_bounded(&head_path, LONGEST_HEAD).unwrap(), later);
    assert_eq!(
        fs::read_bounded(
            &peer.join(segment_name(head.latest()).as_str()),
            LONGEST_SEGMENT
        )
        .unwrap(),
        original
    );
}

#[test]
fn a_trash_and_put_back_propagate_through_the_shared_folder() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    first.run(&["add", "https://trash.example/", "--no-fetch"]);
    pair(&first, &second, files.path());
    let local = Store::new(first.home.path().to_path_buf());
    let remote = Store::new(second.home.path().to_path_buf());
    let now = youarel_cli::clock::now().unwrap();
    let id = local
        .session(now, |session| {
            session.holder(None, "https://trash.example/")
        })
        .unwrap()
        .unwrap();
    local
        .session(now, |session| {
            session.edit(Edit::Trash {
                entry: Entry::Shortcut(id),
            })
        })
        .unwrap();
    sync_all(&[&first, &second], shared.path());
    remote
        .session(now, |session| {
            assert_eq!(session.item(id)?, None);
            assert_eq!(
                session.trashed(id)?.unwrap().presence.value,
                youarel_core::state::Presence::Trashed
            );
            Ok(())
        })
        .unwrap();
    local
        .session(now, |session| {
            session.edit(Edit::PutBack {
                entry: Entry::Shortcut(id),
            })
        })
        .unwrap();
    sync_all(&[&first, &second], shared.path());
    remote
        .session(now, |session| {
            assert_eq!(
                session.item(id)?.unwrap().presence.value,
                youarel_core::state::Presence::Live
            );
            assert_eq!(session.trashed(id)?, None);
            Ok(())
        })
        .unwrap();
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
    sync_all(&[&first, &second], shared.path());
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
    sync_all(&[&first, &second], shared.path());
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

/// Runs one edit in a session of a device's store now.
fn edit(store: &Store, edit: Edit) {
    let now = youarel_cli::clock::now().unwrap();
    store.session(now, |session| session.edit(edit)).unwrap();
}

/// The shortcuts a device's trash lists.
fn trash(store: &Store) -> Vec<ItemId> {
    let now = youarel_cli::clock::now().unwrap();
    store
        .session(now, |session| session.trash_page(0, usize::MAX))
        .unwrap()
}

#[test]
fn a_shortcut_one_device_puts_in_the_trash_waits_in_the_other_s_and_comes_back_on_both() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    let link = "https://trashed.example/";
    first.run(&["add", link, "--no-fetch", "--folder", "Work"]);
    pair(&first, &second, files.path());
    let (mine, theirs) = (
        Store::new(first.home.path().to_path_buf()),
        Store::new(second.home.path().to_path_buf()),
    );
    let now = youarel_cli::clock::now().unwrap();
    let held: Vec<ItemId> = mine
        .session(now, |session| {
            Ok(session
                .holders(link)?
                .into_iter()
                .map(|(id, _)| id)
                .collect())
        })
        .unwrap();
    let id = *held.first().unwrap();
    let shortcut = Entry::Shortcut(id);
    edit(&mine, Edit::Trash { entry: shortcut });
    sync_all(&[&first, &second], shared.path());
    assert_eq!(trash(&theirs), [id]);
    assert!(!second.run(&["find", "trashed"]).contains(link));
    assert_eq!(second.listing("Work"), Vec::<String>::new());
    edit(&theirs, Edit::PutBack { entry: shortcut });
    sync_all(&[&first, &second], shared.path());
    assert_eq!(trash(&mine), []);
    assert!(first.run(&["find", "trashed"]).contains(link));
    assert_eq!(first.listing("Work"), second.listing("Work"));
    assert_eq!(first.listing("Work").len(), 1);
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
        printed.contains("skipped") && printed.contains("wrote no segments"),
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
    let unread_peer = youarel_sync::hex(&[99; 32]);
    fs::create_dir_all(&shared.path().join(&unread_peer).join("head")).unwrap();
    first.run(&["add", "https://beside.example/", "--no-fetch"]);
    first.run(&["sync", path(shared.path())]);
    let unread = second.run(&["sync", path(shared.path())]);
    assert!(
        unread.contains(&format!("skipped {unread_peer}")),
        "{unread}"
    );
    assert!(
        unread.contains(&format!("merged {bundle}")) && unread.contains("up to date"),
        "the sync goes on past the file it cannot read: {unread}"
    );
    assert!(second.run(&["find", "beside"]).contains("beside.example"));
}

#[test]
fn a_merge_writes_nothing_into_the_folder_while_the_store_gains_the_other_device_s_shortcut() {
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
    assert_eq!(merged.lines, [format!("merged {}", first.bundle())]);
    assert!(
        merged.pending,
        "the folder lacks a bundle of the merging device"
    );
    assert_eq!(
        fs::list(shared.path()).unwrap(),
        [first.bundle()],
        "the merge wrote nothing into the folder"
    );
    assert!(second.run(&["find", "merged"]).contains("merged.example"));
    assert_eq!(
        youarel_cli::sync::own_chain(&store).unwrap(),
        second.bundle()
    );
}

#[test]
fn a_device_with_nothing_new_writes_nothing_while_another_device_s_bundle_arrives() {
    let files = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.run(&["account", "create"]);
    pair(&first, &second, files.path());
    let store = Store::new(second.home.path().to_path_buf());
    let target = path(shared.path());
    let now = youarel_cli::clock::now().unwrap();
    assert!(
        youarel_cli::sync::unpublished(&store, now, target).unwrap(),
        "a device that never published there"
    );
    sync_all(&[&first, &second], shared.path());
    assert!(!youarel_cli::sync::unpublished(&store, now, target).unwrap());
    let own = shared.path().join(second.bundle()).join("head");
    let written = fs::read_bytes_if_present(&own).unwrap().unwrap();
    for _ in 0..2 {
        let merged = youarel_cli::sync::merge(&store, now, target).unwrap();
        assert!(!merged.pending, "{merged:?}");
        assert_eq!(fs::read_bytes_if_present(&own).unwrap().unwrap(), written);
    }
    first.run(&["add", "https://arrived.example/", "--no-fetch"]);
    first.run(&["sync", target]);
    let merged = youarel_cli::sync::merge(&store, now, target).unwrap();
    assert!(
        merged.lines.contains(&format!("merged {}", first.bundle())),
        "{merged:?}"
    );
    assert!(
        !merged.pending,
        "another device's write is that device's to publish"
    );
    assert_eq!(fs::read_bytes_if_present(&own).unwrap().unwrap(), written);
    assert!(second.run(&["find", "arrived"]).contains("arrived.example"));
    second.run(&["add", "https://own.example/", "--no-fetch"]);
    assert!(
        youarel_cli::sync::merge(&store, now, target)
            .unwrap()
            .pending,
        "a write of its own waits for a publish"
    );
    assert!(
        youarel_cli::sync::unpublished(&store, now, target).unwrap(),
        "and a restart finds it unpublished"
    );
    second.run(&["sync", target]);
    assert!(!youarel_cli::sync::unpublished(&store, now, target).unwrap());
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(
        youarel_cli::sync::unpublished(&store, now, path(elsewhere.path())).unwrap(),
        "a publish carries nothing to another folder"
    );
}
