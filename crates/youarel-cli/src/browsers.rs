//! The bookmarks each browser keeps on this device, found through the core's decision and read by the browser readers.

use crate::home::{host, variable};
use anyhow::{Context, Result, anyhow};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use youarel_core::browser_rules::{
    BROWSERS, Browser, BrowserFacts, FIREFOX_BACKUP_EXTENSION, FIREFOX_BACKUPS, Holds, Kind, kind,
    place,
};
use youarel_html::bookmarks;
use youarel_import::{Item, LONGEST_FILE, chrome, firefox, safari};
use youarel_store::fs;

/// A browser as the command line and the window name it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum BrowserName {
    Safari,
    Chrome,
    Firefox,
}

impl From<BrowserName> for Browser {
    fn from(name: BrowserName) -> Self {
        match name {
            BrowserName::Safari => Self::Safari,
            BrowserName::Chrome => Self::Chrome,
            BrowserName::Firefox => Self::Firefox,
        }
    }
}

/// The name a person reads for a browser, which also names the folder its bookmarks arrive in.
#[must_use]
pub const fn title(browser: Browser) -> &'static str {
    match browser {
        Browser::Safari => "Safari",
        Browser::Chrome => "Chrome",
        Browser::Firefox => "Firefox",
    }
}

/// The browsers that keep bookmarks on this kind of host.
///
/// # Errors
///
/// Returns an error when a variable the decision reads is not UTF-8.
pub fn available() -> Result<Vec<Browser>> {
    let values = Values::read()?;
    let facts = values.facts();
    Ok(BROWSERS
        .into_iter()
        .filter(|browser| place(*browser, &facts).is_some())
        .collect())
}

/// The variables the decision reads.
struct Values {
    home: Option<String>,
    local_app_data: Option<String>,
    app_data: Option<String>,
    xdg_config_home: Option<String>,
}

impl Values {
    fn read() -> Result<Self> {
        Ok(Self {
            home: variable("HOME")?,
            local_app_data: variable("LOCALAPPDATA")?,
            app_data: variable("APPDATA")?,
            xdg_config_home: variable("XDG_CONFIG_HOME")?,
        })
    }

    fn facts(&self) -> BrowserFacts<'_> {
        BrowserFacts {
            home: self.home.as_deref(),
            local_app_data: self.local_app_data.as_deref(),
            app_data: self.app_data.as_deref(),
            xdg_config_home: self.xdg_config_home.as_deref(),
            host: host(),
        }
    }
}

/// The newest backup of bookmarks across the profiles under `profiles`, which Firefox names by date so the greatest name is the newest.
fn newest_backup(profiles: &Path) -> Result<PathBuf> {
    let mut newest: Option<(String, PathBuf)> = None;
    let listed =
        fs::list(profiles).map_err(|error| unreadable(Browser::Firefox, profiles, error))?;
    for profile in listed {
        let backups = profiles.join(&profile).join(FIREFOX_BACKUPS);
        // A profile without backups, or an entry that is no profile, holds nothing to read.
        let Ok(names) = fs::list(&backups) else {
            continue;
        };
        for name in names {
            let backup = Path::new(&name)
                .extension()
                .is_some_and(|extension| extension == FIREFOX_BACKUP_EXTENSION);
            if backup && newest.as_ref().is_none_or(|(held, _)| *held < name) {
                newest = Some((name.clone(), backups.join(&name)));
            }
        }
    }
    newest.map(|(_, path)| path).with_context(|| {
        format!(
            "no Firefox profile under {} holds a backup of its bookmarks",
            profiles.display()
        )
    })
}

/// What a failed read means for a person, with the way around a file the system keeps private.
fn unreadable(browser: Browser, path: &Path, error: fs::FileError) -> anyhow::Error {
    let kind = error.source.kind();
    if kind == ErrorKind::PermissionDenied && browser == Browser::Safari {
        anyhow!(
            "macOS keeps Safari's bookmarks private; give youarel Full Disk Access in System Settings, or export them from Safari as an HTML file"
        )
    } else if kind == ErrorKind::NotFound {
        anyhow!(
            "{} keeps no bookmarks at {}; is it installed and has it run?",
            title(browser),
            path.display()
        )
    } else {
        anyhow!(error)
    }
}

/// The bookmarks a browser keeps on this device, in a folder named after the browser.
///
/// # Errors
///
/// Returns an error when the browser has no place on this device, its file cannot be read, or it holds no bookmarks the reader reads.
pub fn read(browser: Browser) -> Result<Item> {
    let values = Values::read()?;
    let found = place(browser, &values.facts())
        .with_context(|| format!("{} keeps no bookmarks on this device", title(browser)))?;
    let mut path = PathBuf::from(found.root);
    path.extend(found.tail);
    if found.holds == Holds::Profiles {
        path = newest_backup(&path)?;
    }
    let bytes =
        fs::read_bounded(&path, LONGEST_FILE).map_err(|error| unreadable(browser, &path, error))?;
    let items = match browser {
        Browser::Safari => safari::parse(&bytes)?,
        Browser::Chrome => chrome::parse(&String::from_utf8_lossy(&bytes))?,
        Browser::Firefox => firefox::decode(&bytes)?,
    };
    Ok(Item::Folder {
        name: title(browser).to_owned(),
        items,
    })
}

/// The bookmarks of a file a person hands over, such as one copied from another device, read by the reader its first bytes name.
///
/// # Errors
///
/// Returns an error when the file is not the kind its first bytes name.
pub fn read_file(bytes: &[u8]) -> Result<Vec<Item>> {
    Ok(match kind(bytes) {
        Kind::Html => bookmarks::parse(&String::from_utf8_lossy(bytes)),
        Kind::Chromium => chrome::parse(&String::from_utf8_lossy(bytes))?,
        Kind::Firefox => firefox::decode(bytes)?,
        Kind::Safari => safari::parse(bytes)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{BrowserName, newest_backup, title};
    use youarel_core::browser_rules::Browser;

    #[test]
    fn the_newest_backup_is_the_greatest_name_across_profiles() {
        let root = tempfile::tempdir().unwrap();
        let profiles = root.path();
        for (profile, name) in [
            ("a.default", "bookmarks-2026-10-01_10_x.jsonlz4"),
            ("b.work", "bookmarks-2026-10-17_12_y.jsonlz4"),
            ("b.work", "notes.txt"),
        ] {
            let backups = profiles.join(profile).join("bookmarkbackups");
            youarel_store::fs::create_dir_all(&backups).unwrap();
            youarel_store::fs::replace(&backups.join(name), b"x").unwrap();
        }
        youarel_store::fs::replace(&profiles.join("profiles.ini"), b"x").unwrap();
        let newest = newest_backup(profiles).unwrap();
        assert!(
            newest.ends_with("b.work/bookmarkbackups/bookmarks-2026-10-17_12_y.jsonlz4"),
            "{}",
            newest.display()
        );
        let empty = tempfile::tempdir().unwrap();
        assert!(
            newest_backup(empty.path())
                .unwrap_err()
                .to_string()
                .contains("no Firefox profile")
        );
        let missing = newest_backup(&empty.path().join("none"))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("Firefox keeps no bookmarks"), "{missing}");
    }

    #[test]
    fn each_browser_has_its_name() {
        assert_eq!(Browser::from(BrowserName::Safari), Browser::Safari);
        assert_eq!(Browser::from(BrowserName::Chrome), Browser::Chrome);
        assert_eq!(Browser::from(BrowserName::Firefox), Browser::Firefox);
        assert_eq!(
            [Browser::Safari, Browser::Chrome, Browser::Firefox].map(title),
            ["Safari", "Chrome", "Firefox"]
        );
    }
}
