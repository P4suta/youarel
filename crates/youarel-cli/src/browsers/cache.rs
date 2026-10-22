use super::Values;
use anyhow::Result;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use youarel_core::icon_rules::{
    BrowserCache, CACHES, MOST_PROFILE_ENTRIES, MOST_PROFILES, cache_place,
};
pub use youarel_import::icons::Format;
use youarel_store::fs;

const CHROMIUM_FILE: &str = "Favicons";
const FIREFOX_FILE: &str = "favicons.sqlite";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePath {
    pub format: Format,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Discovery {
    pub caches: Vec<CachePath>,
    pub problems: Vec<String>,
}

fn present(path: &Path) -> Result<bool> {
    Ok(fs::exists(path)?)
}

fn collect(browser: BrowserCache, root: &Path, found: &mut Discovery) -> Result<()> {
    let format = match browser {
        BrowserCache::Chrome
        | BrowserCache::Chromium
        | BrowserCache::Edge
        | BrowserCache::Brave => Format::Chromium,
        BrowserCache::Firefox => Format::Firefox,
        BrowserCache::Safari => Format::Safari,
    };
    if browser == BrowserCache::Safari {
        if present(root)? {
            found.caches.push(CachePath {
                format,
                path: root.to_owned(),
            });
        }
        return Ok(());
    }
    let names = match fs::list_bounded(root, MOST_PROFILE_ENTRIES) {
        Ok(names) => names,
        Err(error) if error.source.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let file = if browser == BrowserCache::Firefox {
        FIREFOX_FILE
    } else {
        CHROMIUM_FILE
    };
    let mut profiles: usize = 0;
    for name in names {
        let profile = root.join(name);
        if !fs::is_directory(&profile)? {
            continue;
        }
        let path = profile.join(file);
        if !present(&path)? {
            continue;
        }
        if profiles >= MOST_PROFILES {
            found.problems.push(format!(
                "{} holds more than {MOST_PROFILES} icon profiles",
                root.display()
            ));
            break;
        }
        profiles = profiles.saturating_add(1);
        found.caches.push(CachePath { format, path });
    }
    Ok(())
}

/// Discovers local cache paths from explicit environment facts.
#[must_use]
pub fn from_facts(facts: &youarel_core::browser_rules::BrowserFacts<'_>) -> Discovery {
    from_choices(facts, &CACHES)
}

fn from_choices(
    facts: &youarel_core::browser_rules::BrowserFacts<'_>,
    choices: &[BrowserCache],
) -> Discovery {
    let mut found = Discovery::default();
    for &browser in choices {
        let Some(place) = cache_place(browser, facts) else {
            continue;
        };
        let mut root = PathBuf::from(place.root);
        root.extend(place.tail);
        if let Err(error) = collect(browser, &root, &mut found) {
            found
                .problems
                .push(format!("{}: {error:#}", root.display()));
        }
    }
    found
}

/// Discovers this device's caches without opening any database or contacting a site.
///
/// # Errors
///
/// Refuses environment variables that are not UTF-8.
pub fn discover() -> Result<Discovery> {
    Ok(from_facts(&Values::read()?.facts()))
}

/// Discovers only the browser whose bookmarks the person chose to import.
///
/// # Errors
///
/// Refuses environment variables that are not UTF-8.
pub fn discover_browser(browser: youarel_core::browser_rules::Browser) -> Result<Discovery> {
    use youarel_core::browser_rules::Browser;
    let chosen = match browser {
        Browser::Chrome => BrowserCache::Chrome,
        Browser::Firefox => BrowserCache::Firefox,
        Browser::Safari => BrowserCache::Safari,
    };
    Ok(from_choices(
        &Values::read()?.facts(),
        std::slice::from_ref(&chosen),
    ))
}
