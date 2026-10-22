//! `youarel add` reads a page's title and its icon from a site served on this host and keeps the icon on the device under the site's host, and an opening gathers the favicon of a host that shows none.
#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use youarel_link::Link;
use youarel_store::Store;

/// The icons the site serves: the launcher's own, from the repository.
const PNG: &[u8] = include_bytes!("../../../app/icons/32x32.png");
const ICO: &[u8] = include_bytes!("../../../app/icons/icon.ico");
/// The eight bytes that begin every PNG image, which the icons a device keeps are.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// A time for the store's session that the test reads under.
const NOW: u64 = 1_700_000_000_000;

/// A command for the built program.
#[expect(
    clippy::disallowed_methods,
    reason = "the end-to-end test starts the built program itself, which it names in one place"
)]
fn youarel() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_youarel"))
}

fn run(home: &Path, arguments: &[&str]) -> String {
    let output = youarel()
        .args(arguments)
        .env("YOUAREL_HOME", home)
        .env("YOUAREL_KEY_STORE", "file")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "youarel {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// A site on this host and how often it handed out its favicon.
struct Site {
    server: Arc<tiny_http::Server>,
    port: u16,
    favicons: Arc<AtomicUsize>,
}

impl Drop for Site {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

/// The site: a page that names its icon, a page that names a missing one, and a favicon at the root, or nothing at all when it keeps no icons.
fn serve(icons: bool) -> Site {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = server.server_addr().to_ip().unwrap().port();
    let favicons = Arc::new(AtomicUsize::new(0));
    let (serving, counting) = (Arc::clone(&server), Arc::clone(&favicons));
    std::thread::spawn(move || {
        for request in serving.incoming_requests() {
            let (body, kind): (&[u8], &str) = match request.url() {
                "/named" => (
                    b"<html><head><title>Named</title><link rel=icon href=/icon.png></head></html>",
                    "text/html; charset=utf-8",
                ),
                "/missing" => (
                    b"<html><head><title>Missing</title><link rel=icon href=/gone.png></head></html>",
                    "text/html",
                ),
                "/icon.png" if icons => (PNG, "image/png"),
                "/favicon.ico" if icons => {
                    counting.fetch_add(1, Ordering::SeqCst);
                    (ICO, "image/x-icon")
                }
                _ => {
                    drop(request.respond(tiny_http::Response::empty(404)));
                    continue;
                }
            };
            let header = tiny_http::Header::from_bytes(b"Content-Type", kind.as_bytes()).unwrap();
            drop(request.respond(tiny_http::Response::from_data(body).with_header(header)));
        }
    });
    Site {
        server,
        port,
        favicons,
    }
}

/// The length of the image each host shows on the device, by host.
fn icons(home: &Path) -> BTreeMap<String, Option<usize>> {
    let store = Store::new(home.to_path_buf());
    let index = store
        .session(NOW, |session| Ok(session.icons().clone()))
        .unwrap();
    index
        .into_iter()
        .map(|(host, icon)| {
            let image = store.image(icon.image).unwrap();
            (host, image.map(|bytes| bytes.len()))
        })
        .collect()
}

#[test]
fn an_addition_keeps_its_page_s_icon_or_its_site_s_favicon_under_the_host() {
    let site = serve(true);
    let home = tempfile::tempdir().unwrap();
    let port = site.port;
    let added = run(
        home.path(),
        &["add", &format!("http://127.0.0.1:{port}/named")],
    );
    assert!(added.contains("added Named"), "{added}");
    let missing = run(
        home.path(),
        &["add", &format!("http://localhost:{port}/missing")],
    );
    assert!(missing.contains("added Missing"), "{missing}");
    assert_eq!(site.favicons.load(Ordering::SeqCst), 1);
    run(
        home.path(),
        &[
            "add",
            &format!("http://localhost:{port}/later"),
            "--no-fetch",
        ],
    );
    let held = icons(home.path());
    assert_eq!(
        held.keys().map(String::as_str).collect::<Vec<_>>(),
        ["127.0.0.1", "localhost"],
        "one icon for each host, whatever its port and path"
    );
    assert!(held.values().all(Option::is_some), "{held:?}");
    let records =
        youarel_store::fs::read_bytes_if_present(&home.path().join(youarel_store::STATE_FILE))
            .unwrap()
            .unwrap();
    assert_eq!(
        youarel_store::records::decode(&records)
            .unwrap()
            .shortcuts
            .len(),
        3
    );
    assert!(
        !records
            .windows(PNG_SIGNATURE.len())
            .any(|window| window == PNG_SIGNATURE),
        "the state's records hold no image"
    );
}

#[test]
fn an_opening_gathers_the_favicon_of_a_host_that_shows_none_and_asks_no_more() {
    let site = serve(true);
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().to_path_buf());
    let link = Link::parse(&format!("http://127.0.0.1:{}/later", site.port)).unwrap();
    assert!(youarel_cli::icons::lacking(&store, NOW, &link).unwrap());
    youarel_cli::icons::opened(&store, NOW, &link).unwrap();
    assert_eq!(site.favicons.load(Ordering::SeqCst), 1);
    assert!(!youarel_cli::icons::lacking(&store, NOW, &link).unwrap());
    youarel_cli::icons::opened(&store, NOW, &link).unwrap();
    assert_eq!(
        site.favicons.load(Ordering::SeqCst),
        1,
        "a host that shows an icon asks its site nothing"
    );
    assert_eq!(
        icons(home.path()).into_keys().collect::<Vec<_>>(),
        ["127.0.0.1"]
    );
    let bare = serve(false);
    let empty = tempfile::tempdir().unwrap();
    let elsewhere = Store::new(empty.path().to_path_buf());
    let unknown = Link::parse(&format!("http://127.0.0.1:{}/", bare.port)).unwrap();
    youarel_cli::icons::opened(&elsewhere, NOW, &unknown).unwrap();
    assert_eq!(
        icons(empty.path()),
        BTreeMap::new(),
        "a site without a favicon keeps its tile"
    );
}
