use super::{CacheError, Format, Image, Snapshot, safari_file};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use youarel_link::Link;
use youarel_store::fs;

#[cfg(unix)]
#[path = "tests/fs.rs"]
mod files;

const PAGE: &str = "https://cache.example/page";
const OTHER: &str = "https://unrelated.example/";
const UUID: &str = "12345678-1234-1234-1234-123456789ABC";
const PNG: &[u8] = include_bytes!("../../../../app/icons/32x32.png");
const CHROMIUM_SCHEMA: &str = "CREATE TABLE favicons (id INTEGER PRIMARY KEY, url TEXT, icon_type INTEGER); CREATE TABLE icon_mapping (id INTEGER PRIMARY KEY, page_url TEXT, icon_id INTEGER); CREATE TABLE favicon_bitmaps (id INTEGER PRIMARY KEY, icon_id INTEGER, width INTEGER, height INTEGER, image_data BLOB);";
const FIREFOX_SCHEMA: &str = "CREATE TABLE moz_icons (id INTEGER PRIMARY KEY, icon_url TEXT, width INTEGER, data BLOB); CREATE TABLE moz_pages_w_icons (id INTEGER PRIMARY KEY, page_url TEXT); CREATE TABLE moz_icons_to_pages (page_id INTEGER, icon_id INTEGER);";
const SAFARI_SCHEMA: &str = "CREATE TABLE icon_info (uuid TEXT PRIMARY KEY, url TEXT, width INTEGER, height INTEGER); CREATE TABLE page_url (url TEXT, uuid TEXT);";

fn fixture(path: &Path, format: Format) -> Connection {
    let connection = Connection::open(path).unwrap();
    match format {
        Format::Chromium => {
            connection.execute_batch(CHROMIUM_SCHEMA).unwrap();
            connection
                .execute("INSERT INTO favicons VALUES(1,?1,1)", [PAGE])
                .unwrap();
            connection
                .execute("INSERT INTO icon_mapping VALUES(1,?1,1)", [PAGE])
                .unwrap();
            for size in [16, 32, 64, 128] {
                connection.execute("INSERT INTO favicon_bitmaps(icon_id,width,height,image_data) VALUES(1,?1,?1,?2)", rusqlite::params![size, PNG]).unwrap();
            }
        }
        Format::Firefox => {
            connection.execute_batch(FIREFOX_SCHEMA).unwrap();
            connection
                .execute("INSERT INTO moz_pages_w_icons VALUES(1,?1)", [PAGE])
                .unwrap();
            for size in [16, 32, 64, 128] {
                connection
                    .execute(
                        "INSERT INTO moz_icons(id,icon_url,width,data) VALUES(?1,?2,?1,?3)",
                        rusqlite::params![size, PAGE, PNG],
                    )
                    .unwrap();
                connection
                    .execute("INSERT INTO moz_icons_to_pages VALUES(1,?1)", [size])
                    .unwrap();
            }
        }
        Format::Safari => {
            connection.execute_batch(SAFARI_SCHEMA).unwrap();
            connection
                .execute("INSERT INTO icon_info VALUES(?1,?2,32,32)", [UUID, PAGE])
                .unwrap();
            connection
                .execute("INSERT INTO page_url VALUES(?1,?2)", [PAGE, UUID])
                .unwrap();
        }
    }
    connection
}

fn snapshot(path: &Path, format: Format) -> Snapshot {
    Snapshot::open(path, format, Arc::new(AtomicBool::new(false))).unwrap()
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read_bounded(path, youarel_core::icon_rules::MOST_DATABASE_BYTES).unwrap()
}

#[test]
fn each_browser_returns_only_mapped_host_images_in_size_order() {
    for format in [Format::Chromium, Format::Firefox, Format::Safari] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("icons.sqlite");
        drop(fixture(&path, format));
        let before = bytes(&path);
        let cache = snapshot(&path, format);
        let candidates = cache.candidates(&Link::parse(PAGE).unwrap()).unwrap();
        assert_ne!(candidates, []);
        assert!(
            candidates
                .windows(2)
                .all(|pair| pair[0].quality >= pair[1].quality)
        );
        assert_eq!(cache.candidates(&Link::parse(OTHER).unwrap()).unwrap(), []);
        assert_eq!(
            cache
                .candidates(&Link::parse("http://cache.example/another").unwrap())
                .unwrap(),
            candidates
        );
        match format {
            Format::Chromium | Format::Firefox => {
                assert_eq!(candidates.len(), 4);
                assert_eq!(
                    candidates[0].quality,
                    youarel_core::icon_rules::quality(64, 64).unwrap()
                );
                assert_eq!(candidates[0].image, Image::Bytes(PNG.to_vec()));
            }
            Format::Safari => assert_eq!(candidates[0].image, Image::SafariFile(safari_file(UUID))),
        }
        assert_eq!(bytes(&path), before);
    }
}

#[test]
fn committed_wal_records_survive_a_consistent_read_only_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Favicons");
    let writer = fixture(&path, Format::Chromium);
    writer
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    let recent = "https://wal.example/only-in-wal";
    writer
        .execute(
            "INSERT INTO icon_mapping(page_url,icon_id) VALUES(?1,1)",
            [recent],
        )
        .unwrap();
    let wal = path.with_file_name("Favicons-wal");
    let before = (bytes(&path), bytes(&wal));
    writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO icon_mapping(page_url,icon_id) VALUES('https://uncommitted.example/',1);").unwrap();
    let cache = snapshot(&path, Format::Chromium);
    assert_ne!(cache.candidates(&Link::parse(recent).unwrap()).unwrap(), []);
    assert_eq!(
        cache
            .candidates(&Link::parse("https://uncommitted.example/").unwrap())
            .unwrap(),
        []
    );
    assert_eq!((bytes(&path), bytes(&wal)), before);
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn absent_broken_unsupported_and_cancelled_databases_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    Snapshot::open(&path, Format::Firefox, Arc::new(AtomicBool::new(false))).unwrap_err();
    assert!(!fs::exists(&path).unwrap());
    fs::replace(&path, b"not a SQLite database").unwrap();
    Snapshot::open(&path, Format::Firefox, Arc::new(AtomicBool::new(false))).unwrap_err();
    fs::remove_if_present(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE other (value TEXT)")
        .unwrap();
    assert!(matches!(
        Snapshot::open(&path, Format::Firefox, Arc::new(AtomicBool::new(false))),
        Err(CacheError::Unsupported)
    ));
    let stopped = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        Snapshot::open(&path, Format::Firefox, stopped),
        Err(CacheError::Stopped)
    ));
}

#[test]
fn cancellation_after_snapshot_refuses_queries_and_drop_removes_the_private_copy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    drop(fixture(&path, Format::Safari));
    let stopped = Arc::new(AtomicBool::new(false));
    let cache = Snapshot::open(&path, Format::Safari, Arc::clone(&stopped)).unwrap();
    let temporary = cache.temporary_directory().to_owned();
    stopped.store(true, Ordering::Relaxed);
    assert!(matches!(
        cache.candidates(&Link::parse(PAGE).unwrap()),
        Err(CacheError::Stopped)
    ));
    drop(cache);
    assert!(!fs::exists(&temporary).unwrap());
}

#[test]
fn safari_names_are_uppercase_md5_file_keys_and_never_paths_from_the_database() {
    assert_eq!(safari_file(""), "D41D8CD98F00B204E9800998ECF8427E");
    assert_eq!(safari_file("abc"), "900150983CD24FB0D6963F7D28E17F72");
    let name = safari_file("../../outside");
    assert_eq!(name.len(), 32);
    assert!(name.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[cfg(unix)]
#[test]
fn an_unreadable_browser_database_is_refused_without_changing_its_contents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("icons.sqlite");
    drop(fixture(&path, Format::Chromium));
    let before = bytes(&path);
    let permissions = files::Unreadable::new(&path);
    Snapshot::open(&path, Format::Chromium, Arc::new(AtomicBool::new(false))).unwrap_err();
    drop(permissions);
    assert_eq!(bytes(&path), before);
}

#[test]
#[ignore = "writes synthetic browser database fixtures and the fuzz corpus"]
fn seeds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::create_dir_all(&root.join("tests/fixtures")).unwrap();
    fs::create_dir_all(&root.join("../../fuzz/corpus/browser_icons")).unwrap();
    for (format, name) in [
        (Format::Chromium, "chromium"),
        (Format::Firefox, "firefox"),
        (Format::Safari, "safari"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("icons.sqlite");
        drop(fixture(&path, format));
        let bytes = bytes(&path);
        fs::replace(
            &root.join("tests/fixtures").join(format!("{name}.sqlite")),
            &bytes,
        )
        .unwrap();
        fs::replace(
            &root.join("../../fuzz/corpus/browser_icons").join(name),
            &bytes,
        )
        .unwrap();
    }
}
