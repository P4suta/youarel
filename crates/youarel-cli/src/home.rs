//! The directory of youarel's files, from the environment through the core's decision.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use youarel_core::home_rules::{HomeFacts, Host, data_dir};

/// The variable that overrides every convention.
pub const OVERRIDE: &str = "YOUAREL_HOME";

/// The kind of host this binary runs on.
#[must_use]
pub const fn host() -> Host {
    if cfg!(windows) {
        Host::Windows
    } else if cfg!(target_os = "macos") {
        Host::MacOs
    } else {
        Host::Unix
    }
}

/// A variable's value, or `None` when it is unset.
pub(crate) fn variable(name: &str) -> Result<Option<String>> {
    std::env::var_os(name)
        .map(|value| {
            value
                .into_string()
                .map_err(|value| anyhow!("{name} holds {}, which is not UTF-8", value.display()))
        })
        .transpose()
}

/// The directory youarel keeps its files in on this device.
///
/// # Errors
///
/// Returns an error when no variable names a directory, or one is not UTF-8.
pub fn data_directory() -> Result<PathBuf> {
    let youarel_home = variable(OVERRIDE)?;
    let local_app_data = variable("LOCALAPPDATA")?;
    let xdg_data_home = variable("XDG_DATA_HOME")?;
    let home = variable("HOME")?;
    let facts = HomeFacts {
        youarel_home: youarel_home.as_deref(),
        local_app_data: local_app_data.as_deref(),
        xdg_data_home: xdg_data_home.as_deref(),
        home: home.as_deref(),
        host: host(),
    };
    let directory = data_dir(&facts).with_context(|| {
        format!("no variable names a home for youarel's files; set {OVERRIDE} to a directory")
    })?;
    let mut path = PathBuf::from(directory.root);
    path.extend(directory.tail);
    Ok(path)
}
