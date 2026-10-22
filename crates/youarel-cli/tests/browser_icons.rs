//! Synthetic browser caches fill imported shortcuts without a site or browser process.
#![cfg(test)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use youarel_cli::browsers::cache::{CachePath, Discovery, Format};
use youarel_cli::{commands, icons::local};
use youarel_core::browser_rules::BrowserFacts;
use youarel_core::home_rules::Host;
use youarel_core::icon_rules::{HOSTS_PER_TICK, RECORDS_PER_TICK};
use youarel_core::state::{Edit, ItemId};
use youarel_import::Item;
use youarel_link::Link;
use youarel_store::{Store, fs};

const NOW: u64 = 1_700_000_000_000;
const PAGE: &str = "https://cache.example/page";
const CHROMIUM: &[u8] = include_bytes!("../../youarel-import/tests/fixtures/chromium.sqlite");
const FIREFOX: &[u8] = include_bytes!("../../youarel-import/tests/fixtures/firefox.sqlite");
const SAFARI: &[u8] = include_bytes!("../../youarel-import/tests/fixtures/safari.sqlite");
const PNG: &[u8] = include_bytes!("../../../app/icons/32x32.png");
const SAFARI_UUID: &str = "12345678-1234-1234-1234-123456789ABC";

fn discovery(path: &Path, format: Format) -> Discovery {
    Discovery {
        caches: vec![CachePath {
            path: path.to_owned(),
            format,
        }],
        problems: Vec::new(),
    }
}

fn added(store: &Store, value: usize, link: String) -> ItemId {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&u64::try_from(value).unwrap().to_be_bytes());
    let id = ItemId(bytes);
    store
        .session(NOW, |session| {
            session.edit(Edit::AddShortcut {
                id,
                link,
                title: format!("Item {value}"),
                folder: None,
            })
        })
        .unwrap();
    id
}

#[test]
fn every_browser_fills_an_imported_host_and_keeps_images_out_of_sealed_records() {
    for (format, fixture) in [
        (Format::Chromium, CHROMIUM),
        (Format::Firefox, FIREFOX),
        (Format::Safari, SAFARI),
    ] {
        let home = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let path = cache.path().join("icons.sqlite");
        fs::replace(&path, fixture).unwrap();
        if format == Format::Safari {
            fs::create_dir_all(&cache.path().join("favicons")).unwrap();
            fs::replace(
                &cache
                    .path()
                    .join("favicons")
                    .join(youarel_import::icons::safari_file(SAFARI_UUID)),
                PNG,
            )
            .unwrap();
        }
        let store = Store::new(home.path().to_owned());
        let imported = commands::import_tree(
            &store,
            NOW,
            &[Item::Link {
                href: PAGE.to_owned(),
                title: "Cached".to_owned(),
            }],
        )
        .unwrap();
        assert_eq!(imported.shortcuts, 1);
        let before = store.session(NOW, |session| session.records()).unwrap();
        let report = local::fill_all(
            &store,
            NOW,
            discovery(&path, format),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(report.kept, 1);
        assert_eq!(report.problems, Vec::<String>::new());
        let icon = store
            .session(NOW, |session| session.icon("cache.example"))
            .unwrap()
            .unwrap();
        assert_eq!(
            store.image(icon.image).unwrap().unwrap(),
            youarel_icon::normalize(PNG).unwrap()
        );
        assert_eq!(
            store.session(NOW, |session| session.records()).unwrap(),
            before
        );
    }
}

#[test]
fn batches_continue_after_full_and_filtered_ranges_without_skipping_a_host() {
    let home = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = cache.path().join("Favicons");
    fs::replace(&path, CHROMIUM).unwrap();
    let store = Store::new(home.path().to_owned());
    let hidden = added(&store, 1, "https://hidden.example/".to_owned());
    store
        .session(NOW, |session| {
            session.edit(Edit::Trash {
                entry: youarel_core::state::Entry::Shortcut(hidden),
            })
        })
        .unwrap();
    for value in 2..=RECORDS_PER_TICK.checked_add(1).unwrap() {
        added(&store, value, format!("https://host{value}.example/"));
    }
    added(
        &store,
        RECORDS_PER_TICK.checked_add(2).unwrap(),
        PAGE.to_owned(),
    );
    let mut after = None;
    let mut hosts = std::collections::BTreeSet::new();
    loop {
        let batch = store
            .session(NOW, |session| session.icon_batch(after))
            .unwrap();
        assert!(batch.links.len() <= HOSTS_PER_TICK);
        for link in batch.links {
            hosts.insert(link.host().to_owned());
        }
        if !batch.more {
            break;
        }
        assert!(batch.after > after);
        after = batch.after;
    }
    assert_eq!(hosts.len(), RECORDS_PER_TICK.checked_add(1).unwrap());
    assert!(!hosts.contains("hidden.example"));
    let report = local::fill_all(
        &store,
        NOW,
        discovery(&path, Format::Chromium),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(report.kept, 1);
}

#[test]
fn broken_cache_reports_its_error_and_a_later_cache_can_still_fill_the_host() {
    let home = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let broken = cache.path().join("broken");
    let valid = cache.path().join("valid");
    fs::replace(&broken, b"not SQLite").unwrap();
    fs::replace(&valid, CHROMIUM).unwrap();
    let mut sources = discovery(&broken, Format::Chromium);
    sources
        .caches
        .extend(discovery(&valid, Format::Chromium).caches);
    let store = Store::new(home.path().to_owned());
    added(&store, 1, PAGE.to_owned());
    let report = local::fill_all(&store, NOW, sources, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!((report.kept, report.failures), (1, 1));
    assert_eq!(report.problems.len(), 1);
    let mut caches = local::Caches::new(Discovery::default(), Arc::new(AtomicBool::new(true)));
    caches
        .fill(&store, NOW, &[Link::parse(PAGE).unwrap()])
        .unwrap_err();
}

#[test]
fn discovery_reads_all_chromium_profiles_and_respects_explicit_environment_facts() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    for profile in ["Default", "Profile 1"] {
        let path = config.join("google-chrome").join(profile);
        fs::create_dir_all(&path).unwrap();
        fs::replace(&path.join("Favicons"), CHROMIUM).unwrap();
    }
    let facts = BrowserFacts {
        home: home.path().to_str(),
        local_app_data: None,
        app_data: None,
        xdg_config_home: config.to_str(),
        host: Host::Unix,
    };
    let found = youarel_cli::browsers::cache::from_facts(&facts);
    assert_eq!(found.problems, Vec::<String>::new());
    assert_eq!(found.caches.len(), 2);
    assert!(
        found
            .caches
            .iter()
            .all(|cache| cache.path.starts_with(&config))
    );
}

mod raw {
    #[expect(
        clippy::disallowed_methods,
        reason = "the end-to-end test starts its named built command with isolated browser roots"
    )]
    #[expect(
        clippy::redundant_pub_crate,
        reason = "unreachable_pub limits this private process adapter to the integration test crate"
    )]
    pub(super) fn command() -> std::process::Command {
        std::process::Command::new(env!("CARGO_BIN_EXE_youarel"))
    }
}

fn cli(root: &Path, args: &[&str]) -> String {
    let output = raw::command()
        .args(args)
        .env("YOUAREL_HOME", root.join("library"))
        .env("HOME", root.join("user"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("APPDATA", root.join("roaming"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_import_command_and_icons_command_use_only_isolated_browser_roots() {
    let root = tempfile::tempdir().unwrap();
    for profile in [
        "user/Library/Application Support/Google/Chrome/Default",
        "local/Google/Chrome/User Data/Default",
        "config/google-chrome/Default",
    ] {
        let path = root.path().join(profile);
        fs::create_dir_all(&path).unwrap();
        fs::replace(&path.join("Favicons"), CHROMIUM).unwrap();
    }
    let bookmarks = root.path().join("bookmarks.html");
    fs::replace(&bookmarks, b"<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF=\"https://cache.example/page\">Cached</A></DL>").unwrap();
    let imported = cli(root.path(), &["import", bookmarks.to_str().unwrap()]);
    assert!(imported.contains("Kept 1 local icons"), "{imported}");
    let store = Store::new(root.path().join("library"));
    assert!(
        store
            .session(NOW, |session| session.icon("cache.example"))
            .unwrap()
            .is_some()
    );
    let next = cli(root.path(), &["icons"]);
    assert!(next.contains("Kept 0 local icons"), "{next}");
    assert_eq!(youarel_icon::SIDE, youarel_core::icon_rules::SIDE);
}
