use super::LocalIcons;
use crate::library::Library;
use std::sync::Mutex;
use youarel_cli::browsers::cache::{CachePath, Discovery, Format};
use youarel_core::icon_rules::HOSTS_PER_TICK;
use youarel_core::state::{Edit, ItemId};
use youarel_store::{Store, fs};

const NOW: u64 = 1_700_000_000_000;
const PAGE: &str = "https://cache.example/page";
const FIXTURE: &[u8] =
    include_bytes!("../../../crates/youarel-import/tests/fixtures/chromium.sqlite");

#[test]
fn visible_work_runs_outside_the_library_lock_and_reservations_release_after_cancellation() {
    let home = tempfile::tempdir().unwrap();
    let browser = tempfile::tempdir().unwrap();
    let path = browser.path().join("Favicons");
    fs::replace(&path, FIXTURE).unwrap();
    let store = Store::new(home.path().to_owned());
    let id = ItemId([1; 16]);
    store
        .session(NOW, |session| {
            session.edit(Edit::AddShortcut {
                id,
                link: PAGE.to_owned(),
                title: "Cached".to_owned(),
                folder: None,
            })
        })
        .unwrap();
    let library = Mutex::new(Library::new(store.clone()));
    let guard = library.lock().unwrap();
    let mut local = LocalIcons::new(store.clone());
    local.discovery = Some(Discovery {
        caches: vec![CachePath {
            format: Format::Chromium,
            path,
        }],
        problems: Vec::new(),
    });
    local.queue(std::iter::once(youarel_store::codec::hex(&id.0).as_str()));
    assert!(local.reserve());
    assert!(!local.reserve());
    let report = local.tick(NOW).unwrap();
    assert_eq!(report.kept, 1);
    assert_eq!(report.problems, Vec::<String>::new());
    assert!(
        store
            .session(NOW, |session| session.icon("cache.example"))
            .unwrap()
            .is_some()
    );
    local.cancel();
    assert!(local.reserve());
    local.tick(NOW).unwrap_err();
    assert!(local.reserve());
    drop(guard);
}

#[test]
fn taking_one_visible_batch_keeps_the_rest_queued() {
    let home = tempfile::tempdir().unwrap();
    let mut local = LocalIcons::new(Store::new(home.path().to_owned()));
    local.discovery = Some(Discovery::default());
    let ids: Vec<String> = (1..=HOSTS_PER_TICK.checked_add(1).unwrap())
        .map(|value| youarel_store::codec::hex(&[u8::try_from(value).unwrap(); 16]))
        .collect();
    local.queue(ids.iter().map(String::as_str));
    assert!(local.reserve());
    local.tick(NOW).unwrap();
    assert_eq!(local.pending.lock().unwrap().len(), 1);
    assert!(local.reserve());
    local.tick(NOW).unwrap();
    assert_eq!(local.pending.lock().unwrap().len(), 0);
}
