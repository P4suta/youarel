use super::{Device, NOW, pair};
use youarel_cli::sync::segments::folder::Directory;
use youarel_cli::sync::segments::{self, SegmentTarget};
use youarel_core::folder_rules::{HEAD_FILE, segment_name};
use youarel_core::segment_rules::Sequence;
use youarel_core::state::{Edit, ItemId};
use youarel_protocol::segment::{Head, Segment};
use youarel_store::fs;
use youarel_sync::hex;

const LONGEST_MESSAGE: u64 = 64 * 1024 * 1024;
const LONGEST_HEAD: u64 = 128;

fn add(device: &Device, number: u8) {
    device.edit(
        NOW + u64::from(number),
        Edit::AddShortcut {
            id: ItemId([number; 16]),
            link: format!("https://folder.example/{number}"),
            title: format!("Shortcut {number}"),
            folder: None,
        },
    );
}

#[test]
fn a_directory_exchanges_immutable_segments_without_publishing_an_echo() {
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    let mut target = Directory::new(shared.path());
    add(&first, 1);
    let head = segments::publish(&first.store, NOW, &mut target)
        .unwrap()
        .unwrap();
    let peer = shared.path().join(hex(&first.account().device()));
    let path = peer.join(segment_name(head.latest()).as_str());
    let bytes = fs::read_bounded(&path, LONGEST_MESSAGE).unwrap();
    assert_eq!(
        Segment::decode(&bytes).unwrap().header.sequence(),
        Sequence::FIRST
    );
    assert_eq!(
        Head::decode(&fs::read_bounded(&peer.join(HEAD_FILE), LONGEST_HEAD).unwrap()).unwrap(),
        head
    );
    segments::folder::merge(&second.store, NOW, shared.path()).unwrap();
    assert_eq!(first.state(), second.state());
    segments::publish(&second.store, NOW, &mut target)
        .unwrap()
        .unwrap();
    let own = target.head(&second.account().device()).unwrap();
    add(&first, 2);
    segments::publish(&first.store, NOW, &mut target)
        .unwrap()
        .unwrap();
    segments::folder::merge(&second.store, NOW, shared.path()).unwrap();
    assert_eq!(first.state(), second.state());
    assert_eq!(
        segments::publish(&second.store, NOW, &mut target).unwrap(),
        None
    );
    assert_eq!(target.head(&second.account().device()).unwrap(), own);
    target
        .write(&first.account().device(), head.latest(), b"changed")
        .unwrap_err();
    assert_eq!(fs::read_bounded(&path, LONGEST_MESSAGE).unwrap(), bytes);
}

#[test]
fn a_compacted_directory_bootstraps_a_late_reader_and_skips_a_broken_peer() {
    let shared = tempfile::tempdir().unwrap();
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    let mut target = Directory::new(shared.path());
    add(&first, 1);
    segments::publish(&first.store, NOW, &mut target).unwrap();
    add(&first, 2);
    let snapshot = segments::snapshot(&first.store, NOW, &mut target)
        .unwrap()
        .unwrap();
    let peer = shared.path().join(hex(&first.account().device()));
    assert!(snapshot.first() > Sequence::FIRST);
    assert!(!fs::exists(&peer.join(segment_name(Sequence::FIRST).as_str())).unwrap());
    let broken = shared.path().join(hex(&[99; 32]));
    fs::create_dir_all(&broken).unwrap();
    fs::replace(&broken.join(HEAD_FILE), b"incomplete").unwrap();
    fs::replace(&shared.path().join("old.youarel"), b"obsolete flat bundle").unwrap();
    let read = segments::folder::merge(&second.store, NOW, shared.path()).unwrap();
    assert!(read.iter().any(|line| line.starts_with("skipped")));
    assert!(read.iter().any(|line| line.starts_with("merged")));
    assert_eq!(first.state(), second.state());
    let merged =
        youarel_cli::sync::merge(&second.store, NOW, shared.path().to_str().unwrap()).unwrap();
    assert_eq!(merged.problems.len(), 1);
    merged.checked_pending().unwrap_err();
    youarel_cli::sync::sync_checked(&second.store, NOW, shared.path().to_str().unwrap())
        .unwrap_err();
    assert!(target.head(&second.account().device()).unwrap().is_some());
}

#[test]
#[ignore = "writes the deterministic shared-folder parser corpus"]
fn folder_seeds() {
    let peer = hex(&[2; 32]);
    let sequence = segment_name(Sequence::FIRST);
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/folder_name");
    fs::create_dir_all(&corpus).unwrap();
    for (name, text) in [
        ("first", sequence.as_str().to_owned()),
        (
            "last",
            segment_name(Sequence::new(u64::MAX).unwrap())
                .as_str()
                .to_owned(),
        ),
        ("head", format!("{peer}/{HEAD_FILE}")),
        ("segment", format!("{peer}/{sequence}")),
        ("sidecar", format!("{peer}/._{sequence}")),
        ("overflow", String::from("18446744073709551616.youarel")),
    ] {
        fs::replace(&corpus.join(name), text.as_bytes()).unwrap();
    }
}
