//! The window launcher as the command line reaches it: `youarel toggle` shows or hides its window, for a desktop whose shortcuts a program cannot hold.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The window launcher's program, which installs beside `youarel`.
const LAUNCHER: &str = "youarel-launcher";
/// The argument that hands a toggle to the running launcher, or starts one with its window shown.
const TOGGLE: &str = "--toggle";

mod raw {
    #![expect(
        clippy::disallowed_methods,
        reason = "the command line starts the window launcher here and nowhere else"
    )]

    use std::path::Path;
    use std::process::{Child, Command};

    pub(super) fn spawn(program: &Path, argument: &str) -> std::io::Result<Child> {
        Command::new(program).arg(argument).spawn()
    }
}

/// The window launcher beside the program at `current`.
#[must_use]
pub fn beside(current: &Path) -> PathBuf {
    current.with_file_name(format!("{LAUNCHER}{}", std::env::consts::EXE_SUFFIX))
}

/// Shows the window launcher's window when it is hidden, and hides it otherwise, starting the launcher when none runs.
///
/// # Errors
///
/// Returns an error when the window launcher is not installed beside this program.
pub fn toggle() -> Result<String> {
    let current = std::env::current_exe().context("cannot find this program")?;
    let launcher = beside(&current);
    // A running launcher takes the toggle and this start ends at once; otherwise this start stays as the launcher.
    drop(raw::spawn(&launcher, TOGGLE).with_context(|| {
        format!(
            "cannot start {}; install the window launcher beside youarel",
            launcher.display()
        )
    })?);
    Ok(String::from("toggled the window launcher"))
}

#[cfg(test)]
mod tests {
    use super::{beside, toggle};
    use std::path::Path;

    #[test]
    fn the_launcher_lives_beside_this_program() {
        let found = beside(Path::new("/opt/youarel/bin/youarel"));
        assert_eq!(found.parent(), Some(Path::new("/opt/youarel/bin")));
        assert!(
            found
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("youarel-launcher"))
        );
        let missing = toggle().unwrap_err();
        assert!(format!("{missing:#}").contains("install the window launcher"));
    }
}
