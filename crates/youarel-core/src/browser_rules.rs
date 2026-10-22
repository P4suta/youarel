//! Where each browser keeps its bookmarks on a device, decided from the environment's values alone.
//!
//! Safari keeps one property list under `HOME/Library/Safari`, only on macOS.
//! Chrome keeps the JSON file `Bookmarks` in its default profile, under `HOME/Library/Application Support` on macOS, `LOCALAPPDATA` on Windows, and `XDG_CONFIG_HOME` or `HOME/.config` elsewhere.
//! Firefox keeps a directory of profiles, under `HOME/Library/Application Support` on macOS, `APPDATA` on Windows, and `HOME/.mozilla` elsewhere, and each profile keeps dated backups of its bookmarks, the newest of which the adapter reads.
//! An empty value counts as unset, so an empty variable never names a place.
//!
//! A file a person hands over, such as one copied from another device, names its reader by its first bytes alone.

use crate::home_rules::Host;

/// A browser whose bookmarks youarel reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Browser {
    Safari,
    Chrome,
    Firefox,
}

/// Every browser, in the order the window offers them.
pub const BROWSERS: [Browser; 3] = [Browser::Safari, Browser::Chrome, Browser::Firefox];

/// What the environment says about where browsers keep their files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserFacts<'a> {
    pub home: Option<&'a str>,
    /// `LOCALAPPDATA`, the per-user local data base of Windows.
    pub local_app_data: Option<&'a str>,
    /// `APPDATA`, the per-user roaming data base of Windows.
    pub app_data: Option<&'a str>,
    /// `XDG_CONFIG_HOME`, the configuration base of Unix desktops.
    pub xdg_config_home: Option<&'a str>,
    pub host: Host,
}

/// Which variable a place stands on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Base {
    Home,
    LocalAppData,
    AppData,
    XdgConfigHome,
}

/// What lies at a place.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Holds {
    /// The browser's bookmarks file itself.
    File,
    /// A directory of profiles, each holding dated backups of its bookmarks.
    Profiles,
}

/// A browser's bookmarks: a base from the environment, the segments to join under it, and what lies there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Place<'a> {
    pub base: Base,
    pub root: &'a str,
    pub tail: &'static [&'static str],
    pub holds: Holds,
}

/// The directory inside a Firefox profile that holds its dated backups of bookmarks.
pub const FIREFOX_BACKUPS: &str = "bookmarkbackups";
/// The extension of a Firefox backup of bookmarks.
pub const FIREFOX_BACKUP_EXTENSION: &str = "jsonlz4";

const SAFARI_MAC: [&str; 3] = ["Library", "Safari", "Bookmarks.plist"];
const CHROME_MAC: [&str; 6] = [
    "Library",
    "Application Support",
    "Google",
    "Chrome",
    "Default",
    "Bookmarks",
];
const CHROME_WINDOWS: [&str; 5] = ["Google", "Chrome", "User Data", "Default", "Bookmarks"];
const CHROME_CONFIG: [&str; 3] = ["google-chrome", "Default", "Bookmarks"];
const CHROME_UNIX_HOME: [&str; 4] = [".config", "google-chrome", "Default", "Bookmarks"];
const FIREFOX_MAC: [&str; 4] = ["Library", "Application Support", "Firefox", "Profiles"];
const FIREFOX_WINDOWS: [&str; 3] = ["Mozilla", "Firefox", "Profiles"];
const FIREFOX_UNIX: [&str; 2] = [".mozilla", "firefox"];

/// A value that names something: present and not empty.
const fn usable(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        Some(_) | None => None,
    }
}

const fn at<'a>(
    base: Base,
    root: Option<&'a str>,
    tail: &'static [&'static str],
    holds: Holds,
) -> Option<Place<'a>> {
    match usable(root) {
        Some(root) => Some(Place {
            base,
            root,
            tail,
            holds,
        }),
        None => None,
    }
}

/// Where a browser keeps its bookmarks on the host the facts describe, or `None` when the browser has no place there or no usable variable names it.
#[must_use]
pub const fn place<'a>(browser: Browser, facts: &BrowserFacts<'a>) -> Option<Place<'a>> {
    match (browser, facts.host) {
        (Browser::Safari, Host::MacOs) => at(Base::Home, facts.home, &SAFARI_MAC, Holds::File),
        (Browser::Safari, Host::Windows | Host::Unix) => None,
        (Browser::Chrome, Host::MacOs) => at(Base::Home, facts.home, &CHROME_MAC, Holds::File),
        (Browser::Chrome, Host::Windows) => at(
            Base::LocalAppData,
            facts.local_app_data,
            &CHROME_WINDOWS,
            Holds::File,
        ),
        (Browser::Chrome, Host::Unix) => match at(
            Base::XdgConfigHome,
            facts.xdg_config_home,
            &CHROME_CONFIG,
            Holds::File,
        ) {
            Some(found) => Some(found),
            None => at(Base::Home, facts.home, &CHROME_UNIX_HOME, Holds::File),
        },
        (Browser::Firefox, Host::MacOs) => {
            at(Base::Home, facts.home, &FIREFOX_MAC, Holds::Profiles)
        }
        (Browser::Firefox, Host::Windows) => at(
            Base::AppData,
            facts.app_data,
            &FIREFOX_WINDOWS,
            Holds::Profiles,
        ),
        (Browser::Firefox, Host::Unix) => {
            at(Base::Home, facts.home, &FIREFOX_UNIX, Holds::Profiles)
        }
    }
}

/// The reader a file of bookmarks needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Kind {
    /// The HTML bookmark file every browser exports.
    Html,
    /// The JSON `Bookmarks` file of Chrome and the browsers built on Chromium, such as Edge and Brave.
    Chromium,
    /// A compressed backup of Firefox's bookmarks.
    Firefox,
    /// Safari's property list, binary or XML.
    Safari,
}

/// The first bytes of a compressed Firefox backup.
pub const FIREFOX_MAGIC: &[u8] = b"mozLz40\0";
/// The first bytes of a binary property list.
pub const BINARY_PLIST: &[u8] = b"bplist00";
/// The element that opens an XML property list.
const PLIST_ELEMENT: &[u8] = b"<plist";
/// The bytes a file's kind depends on: room for an XML declaration and a document type before a property list opens.
pub const HEAD: usize = 512;
/// The byte order mark UTF-8 text may start with.
const BYTE_ORDER_MARK: &[u8] = b"\xef\xbb\xbf";
/// The byte that opens a JSON object.
const OPEN_OBJECT: u8 = b'{';

/// The kind of a file of bookmarks, from its first [`HEAD`] bytes: a magic number names a Firefox backup or a binary property list, a JSON object names Chromium's file, an XML property list names Safari's, and anything else reads as HTML.
#[must_use]
pub fn kind(bytes: &[u8]) -> Kind {
    let head = bytes.get(..HEAD).unwrap_or(bytes);
    if head.starts_with(FIREFOX_MAGIC) {
        return Kind::Firefox;
    }
    if head.starts_with(BINARY_PLIST) {
        return Kind::Safari;
    }
    let text = head.strip_prefix(BYTE_ORDER_MARK).unwrap_or(head);
    let first = text.iter().find(|byte| !byte.is_ascii_whitespace());
    if first == Some(&OPEN_OBJECT) {
        Kind::Chromium
    } else if text
        .windows(PLIST_ELEMENT.len())
        .any(|window| window == PLIST_ELEMENT)
    {
        Kind::Safari
    } else {
        Kind::Html
    }
}

/// The properties Kani proves over every combination of set, empty, and unset variables, every host, and every browser, and over every short file.
#[cfg(any(test, kani))]
mod proofs {
    use super::{
        BINARY_PLIST, BYTE_ORDER_MARK, Base, Browser, BrowserFacts, FIREFOX_MAGIC, Holds, Kind,
        OPEN_OBJECT, PLIST_ELEMENT, kind, place, usable,
    };
    use crate::home_rules::Host;

    /// A variable as the harness draws it: unset, empty, or a name.
    pub(super) const fn variable(choice: u8, name: &'static str) -> Option<&'static str> {
        match choice {
            0 => None,
            1 => Some(""),
            _ => Some(name),
        }
    }

    /// The base a browser stands on, restated: the first usable variable the host's convention names for it.
    pub(super) fn expected(browser: Browser, facts: &BrowserFacts<'_>) -> Option<(Base, Holds)> {
        let home = usable(facts.home).map(|_| Base::Home);
        match (browser, facts.host) {
            (Browser::Safari | Browser::Chrome, Host::MacOs) => {
                home.map(|base| (base, Holds::File))
            }
            (Browser::Safari, Host::Windows | Host::Unix) => None,
            (Browser::Chrome, Host::Windows) => {
                usable(facts.local_app_data).map(|_| (Base::LocalAppData, Holds::File))
            }
            (Browser::Chrome, Host::Unix) => usable(facts.xdg_config_home)
                .map(|_| Base::XdgConfigHome)
                .or(home)
                .map(|base| (base, Holds::File)),
            (Browser::Firefox, Host::Windows) => {
                usable(facts.app_data).map(|_| (Base::AppData, Holds::Profiles))
            }
            (Browser::Firefox, Host::MacOs | Host::Unix) => {
                home.map(|base| (base, Holds::Profiles))
            }
        }
    }

    /// What a place promises: the base and the kind of place the convention names, a root that is the base's own nonempty value, and a tail that leaves the root.
    pub(super) fn spec(browser: Browser, facts: &BrowserFacts<'_>) -> bool {
        let found = place(browser, facts);
        let base_of = |base: Base| match base {
            Base::Home => facts.home,
            Base::LocalAppData => facts.local_app_data,
            Base::AppData => facts.app_data,
            Base::XdgConfigHome => facts.xdg_config_home,
        };
        found.map(|found| (found.base, found.holds)) == expected(browser, facts)
            && found.is_none_or(|found| {
                !found.root.is_empty()
                    && base_of(found.base) == Some(found.root)
                    && !found.tail.is_empty()
            })
    }

    /// Every browser on every host under every combination of the four variables.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(4)]
    fn each_browser_stands_on_the_first_usable_variable_its_host_names() {
        let host = match kani::any::<u8>() {
            0 => Host::Windows,
            1 => Host::MacOs,
            _ => Host::Unix,
        };
        let browser = match kani::any::<u8>() {
            0 => Browser::Safari,
            1 => Browser::Chrome,
            _ => Browser::Firefox,
        };
        let facts = BrowserFacts {
            home: variable(kani::any(), "/h"),
            local_app_data: variable(kani::any(), "/l"),
            app_data: variable(kani::any(), "/a"),
            xdg_config_home: variable(kani::any(), "/x"),
            host,
        };
        kani::assert(
            spec(browser, &facts),
            "the place follows the host's convention",
        );
        let found = place(browser, &facts);
        kani::cover!(
            found.is_some_and(|found| found.holds == Holds::Profiles),
            "a profile directory"
        );
        kani::cover!(
            found.is_some_and(|found| found.base == Base::XdgConfigHome),
            "the configuration base"
        );
        kani::cover!(
            found.is_none() && browser == Browser::Safari,
            "no Safari off macOS"
        );
    }

    /// What a file's kind promises, restated by place: a magic number decides first, an XML property list names Safari unless a JSON object opens the text, a JSON object after blanks names Chromium, and anything else leaves HTML.
    pub(super) fn kind_spec(bytes: &[u8]) -> bool {
        let text = if bytes.starts_with(BYTE_ORDER_MARK) {
            bytes.get(BYTE_ORDER_MARK.len()..).unwrap_or_default()
        } else {
            bytes
        };
        let leading = text.iter().position(|byte| !byte.is_ascii_whitespace());
        let object = leading.and_then(|at| text.get(at)) == Some(&OPEN_OBJECT);
        let element = (0..text.len()).any(|at| {
            text.get(at..)
                .is_some_and(|rest| rest.starts_with(PLIST_ELEMENT))
        });
        let expected = if bytes.starts_with(FIREFOX_MAGIC) {
            Kind::Firefox
        } else if bytes.starts_with(BINARY_PLIST) || (element && !object) {
            Kind::Safari
        } else if object {
            Kind::Chromium
        } else {
            Kind::Html
        };
        kind(bytes) == expected
    }

    /// Every file of up to eight bytes, which holds each magic number and the opening of a property list.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(10)]
    fn a_file_s_first_bytes_name_its_reader() {
        let bytes: [u8; 8] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= bytes.len());
        let file = bytes.get(..length).unwrap_or_default();
        kani::assert(kind_spec(file), "the first bytes name the reader");
        let found = kind(file);
        kani::cover!(found == Kind::Firefox, "a Firefox backup");
        kani::cover!(
            found == Kind::Chromium && file.first() != Some(&OPEN_OBJECT),
            "a JSON object after blanks"
        );
        kani::cover!(
            found == Kind::Safari && !file.starts_with(BINARY_PLIST),
            "an XML property list"
        );
        kani::cover!(found == Kind::Html && !file.is_empty(), "an HTML file");
    }

    /// An empty `HOME` names no directory; this probe claims it places Safari and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_place_under_an_empty_home() {
        let facts = BrowserFacts {
            home: Some(""),
            local_app_data: None,
            app_data: None,
            xdg_config_home: None,
            host: Host::MacOs,
        };
        kani::assert(
            place(Browser::Safari, &facts).is_some(),
            "NEGATIVE_BROWSER_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{kind_spec, spec, variable};
    use super::{BROWSERS, Base, Browser, BrowserFacts, HEAD, Holds, Kind, kind, place};
    use crate::home_rules::Host;
    use alloc::vec::Vec;

    #[test]
    fn each_file_names_its_reader_by_its_first_bytes() {
        let netscape = b"<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>";
        assert_eq!(kind(netscape), Kind::Html);
        assert_eq!(kind(b"\xef\xbb\xbf\r\n {\n \"roots\": {}"), Kind::Chromium);
        assert_eq!(kind(b"mozLz40\0\x10\0\0\0"), Kind::Firefox);
        assert_eq!(kind(b"bplist00\xd1\x01"), Kind::Safari);
        let xml = b"<?xml version=\"1.0\"?>\n<!DOCTYPE plist>\n<plist version=\"1.0\">";
        assert_eq!(kind(xml), Kind::Safari);
        assert_eq!(kind(b""), Kind::Html);
        assert_eq!(kind(b"{ \"title\": \"<plist\" }"), Kind::Chromium);
        let mut late: Vec<u8> = b"<html>".repeat(HEAD);
        late.extend_from_slice(b"<plist>");
        assert_eq!(kind(&late), Kind::Html, "only the head counts");
    }

    #[test]
    fn the_kind_s_specification_agrees_over_drawn_heads() {
        let pieces: [&[u8]; 8] = [
            b"",
            b" ",
            b"\xef\xbb\xbf",
            b"{",
            b"<plist",
            b"mozLz40\0",
            b"bplist00",
            b"<a>",
        ];
        for first in pieces {
            for second in pieces {
                for third in pieces {
                    let file = [first, second, third].concat();
                    assert!(kind_spec(&file), "{file:?}");
                }
            }
        }
    }

    fn facts(host: Host) -> BrowserFacts<'static> {
        BrowserFacts {
            home: Some("/home/me"),
            local_app_data: Some("C:\\Users\\me\\AppData\\Local"),
            app_data: Some("C:\\Users\\me\\AppData\\Roaming"),
            xdg_config_home: None,
            host,
        }
    }

    #[test]
    fn each_browser_has_its_place_on_each_host() {
        let safari = place(Browser::Safari, &facts(Host::MacOs)).unwrap();
        assert_eq!(
            (safari.base, safari.tail.last()),
            (Base::Home, Some(&"Bookmarks.plist"))
        );
        assert_eq!(place(Browser::Safari, &facts(Host::Windows)), None);
        assert_eq!(place(Browser::Safari, &facts(Host::Unix)), None);
        let chrome = place(Browser::Chrome, &facts(Host::Windows)).unwrap();
        assert_eq!(
            (chrome.base, chrome.holds),
            (Base::LocalAppData, Holds::File)
        );
        let chrome_unix = place(Browser::Chrome, &facts(Host::Unix)).unwrap();
        assert_eq!(chrome_unix.tail.first(), Some(&".config"));
        let configured = BrowserFacts {
            xdg_config_home: Some("/home/me/.cfg"),
            ..facts(Host::Unix)
        };
        assert_eq!(
            place(Browser::Chrome, &configured).map(|found| found.base),
            Some(Base::XdgConfigHome)
        );
        let firefox = place(Browser::Firefox, &facts(Host::Windows)).unwrap();
        assert_eq!(
            (firefox.base, firefox.holds),
            (Base::AppData, Holds::Profiles)
        );
        assert_eq!(BROWSERS.len(), 3);
    }

    #[test]
    fn the_specification_agrees_over_every_variable_and_host() {
        for host in [Host::Windows, Host::MacOs, Host::Unix] {
            for browser in BROWSERS {
                for choices in 0_u8..81 {
                    let digit = |place: u32| choices / 3_u8.pow(place) % 3;
                    let drawn = BrowserFacts {
                        home: variable(digit(0), "/h"),
                        local_app_data: variable(digit(1), "/l"),
                        app_data: variable(digit(2), "/a"),
                        xdg_config_home: variable(digit(3), "/x"),
                        host,
                    };
                    assert!(spec(browser, &drawn), "{browser:?} {drawn:?}");
                }
            }
        }
    }
}
