//! `youarel add` reads a page's title and its icon from a site served on this host, and `youarel icons` finds the favicon of a shortcut added without one.
#![cfg(test)]

use std::path::Path;
use std::sync::Arc;
use youarel_store::Store;

/// The icons the site serves: the launcher's own, from the repository.
const PNG: &[u8] = include_bytes!("../../../app/icons/32x32.png");
const ICO: &[u8] = include_bytes!("../../../app/icons/icon.ico");
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

/// The site: a page that names its icon, a page that names a missing one, and a favicon at the root.
fn serve() -> (Arc<tiny_http::Server>, String) {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = server.server_addr().to_ip().unwrap().port();
    let serving = Arc::clone(&server);
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
                "/icon.png" => (PNG, "image/png"),
                "/favicon.ico" => (ICO, "image/x-icon"),
                _ => {
                    drop(request.respond(tiny_http::Response::empty(404)));
                    continue;
                }
            };
            let header = tiny_http::Header::from_bytes(b"Content-Type", kind.as_bytes()).unwrap();
            drop(request.respond(tiny_http::Response::from_data(body).with_header(header)));
        }
    });
    (server, format!("http://127.0.0.1:{port}"))
}

/// The icon images of the shortcuts the store holds, by link.
fn icons(home: &Path) -> Vec<(String, Option<usize>)> {
    Store::new(home.to_path_buf())
        .session(NOW, |session| {
            let state = session.state();
            Ok(state
                .shortcuts
                .iter()
                .map(|(id, shortcut)| {
                    (
                        shortcut.link.value.clone(),
                        state.icon(*id).map(<[u8]>::len),
                    )
                })
                .collect())
        })
        .unwrap()
}

#[test]
fn a_shortcut_takes_its_page_s_icon_or_its_site_s_favicon() {
    let (server, site) = serve();
    let home = tempfile::tempdir().unwrap();
    let added = run(home.path(), &["add", &format!("{site}/named")]);
    assert!(added.contains("added Named"), "{added}");
    let missing = run(home.path(), &["add", &format!("{site}/missing")]);
    assert!(missing.contains("added Missing"), "{missing}");
    run(
        home.path(),
        &["add", &format!("{site}/later"), "--no-fetch"],
    );
    let before = icons(home.path());
    assert!(
        before
            .iter()
            .all(|(link, icon)| icon.is_some() != link.ends_with("/later")),
        "the page's icon or the favicon, and none without a fetch: {before:?}"
    );
    let filled = run(home.path(), &["icons"]);
    assert!(filled.contains("fetched 1 of 1 missing icons"), "{filled}");
    assert!(icons(home.path()).iter().all(|(_, icon)| icon.is_some()));
    let again = run(home.path(), &["icons"]);
    assert!(
        again.contains("every shortcut shows an icon already"),
        "{again}"
    );
    server.unblock();
}
