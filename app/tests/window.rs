//! The launcher's window under a real webview: keys typed into it reach the machine through the script, the machine's effect reaches the store, and leaving hides the window.
//!
//! It takes the keyboard of whatever display it runs on, so it stays ignored except under the virtual display of `just app-linux`.
#![cfg(test)]

use std::path::Path;
use std::process::{Child, Output};
use std::time::Duration;
use youarel_store::Store;

/// How long the test waits for the window or for an effect, and how often it looks.
const WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);
/// How long the page takes to load its script once the window shows, before keys reach it.
const SETTLE: Duration = Duration::from_secs(3);
/// The window's title in `tauri.conf.json`, which `xdotool` finds it by.
const TITLE: &str = "^youarel$";
/// The folder the test names through the window.
const FOLDER: &str = "Home";
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

/// Whether the store under `home` holds a folder of that name.
fn holds_folder(home: &Path, name: &str) -> bool {
    Store::new(home.to_path_buf())
        .session(NOW, |session| {
            Ok(session
                .state()
                .folders
                .values()
                .any(|folder| !folder.removed.value && folder.name.value == name))
        })
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

#[test]
#[ignore = "drives a real window; `just app-linux` runs it under a virtual display"]
fn keys_typed_into_the_window_name_a_folder_and_escape_hides_it() {
    let home = tempfile::tempdir().unwrap();
    let _launcher = Running(
        command(env!("CARGO_BIN_EXE_youarel-launcher"))
            .arg("--toggle")
            .env("YOUAREL_HOME", home.path())
            .env("YOUAREL_KEY_STORE", "file")
            .spawn()
            .unwrap(),
    );
    assert!(eventually(visible), "the window never showed");
    pause(SETTLE);
    xdotool(&["key", "--clearmodifiers", "ctrl+o"]);
    xdotool(&["type", "--delay", "50", FOLDER]);
    xdotool(&["key", "Return"]);
    assert!(
        eventually(|| holds_folder(home.path(), FOLDER)),
        "the window's keys never named the folder"
    );
    xdotool(&["key", "Escape"]);
    assert!(eventually(|| !visible()), "escape left the window showing");
}
