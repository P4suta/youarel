//! The library's window under a real webview: keys typed into its add sheet reach the library through the script, the shortcut reaches the store, and a second toggle hides the window.
//! A library longer than a page scrolls to its last row, and the delete key moves that row to the trash, the row that takes its place takes the selection, and select all takes every other row there.
//! The app lets one instance run at a time, so one test runs the scenarios in turn.
//!
//! It takes the keyboard of whatever display it runs on, so it stays ignored except under the virtual display of `just app-linux`.
#![cfg(test)]

use std::path::Path;
use std::process::{Child, Output};
use std::time::Duration;
use youarel_core::state::Edit;
use youarel_store::Store;

/// How long the test waits for the window or for an effect, and how often it looks.
const WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);
/// How long the page takes to load its script once the window shows, before keys reach it.
const SETTLE: Duration = Duration::from_secs(3);
/// The window's title in `tauri.conf.json`, which `xdotool` finds it by.
const TITLE: &str = "^youarel$";
/// The link the test adds through the window, on a port where nothing answers, so reading its page fails at once.
const LINK: &str = "http://127.0.0.1:9/";
/// How long the add sheet takes to open and read the clipboard before keys reach its field.
const SHEET: Duration = Duration::from_secs(1);
/// A time for the store's session that the test reads under.
const NOW: u64 = 1_700_000_000_000;

/// A command for a program the test runs on the display.
#[expect(
    clippy::disallowed_methods,
    reason = "the test starts the launcher and drives its window with xdotool, which it names in one place"
)]
fn command(program: &str) -> std::process::Command {
    std::process::Command::new(program)
}

/// Waits a while for a real window, which answers in its own time.
#[expect(
    clippy::disallowed_methods,
    reason = "the test waits on a real display, which no clock of its own controls"
)]
fn pause(duration: Duration) {
    std::thread::sleep(duration);
}

fn xdotool(arguments: &[&str]) -> Output {
    command("xdotool").args(arguments).output().unwrap()
}

/// Whether the launcher's window shows on the display.
fn visible() -> bool {
    xdotool(&["search", "--onlyvisible", "--name", TITLE])
        .status
        .success()
}

/// Waits until `holds` does, looking every poll for as long as the test waits, and says whether it did in time.
fn eventually(holds: impl Fn() -> bool) -> bool {
    let looks = WAIT.as_millis().checked_div(POLL.as_millis()).unwrap_or(1);
    for _ in 0..looks {
        if holds() {
            return true;
        }
        pause(POLL);
    }
    holds()
}

/// Whether the store under `home` holds a shortcut of that link.
fn holds_shortcut(home: &Path, link: &str) -> bool {
    Store::new(home.to_path_buf())
        .session(NOW, |session| session.holds_link(link))
        .unwrap()
}

/// Starts the library on the display, with its files under `home`.
fn launch(home: &Path) -> Child {
    command(env!("CARGO_BIN_EXE_youarel-launcher"))
        .arg("--toggle")
        .env("YOUAREL_HOME", home)
        .spawn()
        .unwrap()
}

/// Ends the launcher however the test ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

/// A link typed into the add sheet reaches the store, and a second toggle hides the window.
fn add_a_link_and_toggle() {
    let home = tempfile::tempdir().unwrap();
    let _library = Running(launch(home.path()));
    assert!(eventually(visible), "the window never showed");
    pause(SETTLE);
    xdotool(&["key", "--clearmodifiers", "ctrl+n"]);
    pause(SHEET);
    xdotool(&["type", "--delay", "50", LINK]);
    xdotool(&["key", "Return"]);
    assert!(
        eventually(|| holds_shortcut(home.path(), LINK)),
        "the add sheet never added the link"
    );
    let mut toggle = launch(home.path());
    assert!(toggle.wait().unwrap().success(), "the second start failed");
    assert!(
        eventually(|| !visible()),
        "the second toggle left the window showing"
    );
}

/// How many shortcuts the long library holds, three pages of the window and more.
const SEEDED: usize = 600;
/// The link the seeded shortcuts share before their numbers.
const SEEDED_LINK: &str = "https://seeded.example/";

/// Writes the long library's shortcuts at the top of the store under `home`.
fn seed(home: &Path) {
    Store::new(home.to_path_buf())
        .session(NOW, |session| {
            for number in 0..SEEDED {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: format!("{SEEDED_LINK}{number}"),
                    title: format!("Seeded {number}"),
                    folder: None,
                })?;
            }
            Ok(())
        })
        .unwrap();
}

/// How many shortcuts show in the store under `home`.
fn held(home: &Path) -> usize {
    Store::new(home.to_path_buf())
        .session(NOW, |session| Ok(session.counts()?.all))
        .unwrap()
}

/// How many items wait in the trash of the store under `home`.
fn trashed(home: &Path) -> usize {
    Store::new(home.to_path_buf())
        .session(NOW, |session| Ok(session.counts()?.trash))
        .unwrap()
}

/// Presses keys on the display, then waits for the window to answer them.
fn press(keys: &str) {
    xdotool(&["key", "--clearmodifiers", keys]);
    pause(SHEET);
}

/// The list of a long library reaches its last row, the delete key moves that row to the trash and selects the row that took its place, and select all takes every other row there.
fn trash_through_the_keys() {
    let home = tempfile::tempdir().unwrap();
    seed(home.path());
    let _library = Running(launch(home.path()));
    assert!(eventually(visible), "the window never showed");
    pause(SETTLE);
    // The list takes the keys once the search, which a summons focuses, gives the focus to the next control.
    press("ctrl+2");
    press("Tab");
    press("End");
    press("Delete");
    assert!(
        eventually(|| held(home.path()) == SEEDED - 1),
        "the last row never went to the trash"
    );
    press("Delete");
    assert!(
        eventually(|| held(home.path()) == SEEDED - 2),
        "the row that took the last row's place never took the selection"
    );
    press("ctrl+a");
    press("Delete");
    assert!(
        eventually(|| held(home.path()) == 0 && trashed(home.path()) == SEEDED),
        "select all never moved every row to the trash"
    );
}

/// The scenarios run in turn in one test, since the app lets one instance run at a time and a second start toggles the first.
#[test]
#[ignore = "drives a real window; `just app-linux` runs it under a virtual display"]
fn the_window_adds_toggles_and_trashes_what_the_keys_select() {
    add_a_link_and_toggle();
    trash_through_the_keys();
}
