//! Browser-cache bitmap preference and bounded snapshot admission.

pub const SIDE: u32 = 64;
pub const LARGEST_SIDE: u32 = 4096;
pub const MOST_DATABASE_BYTES: u64 = 256 * 1024 * 1024;
pub const LEAST_PAGE: u64 = 512;
pub const LARGEST_PAGE: u64 = 65536;
pub const BACKUP_PAGES: i32 = 64;
pub const MOST_BACKUP_STEPS: usize = 8192;
pub const MOST_BUSY: usize = 3;
pub const MOST_CANDIDATES: usize = 16;
pub const MOST_PROFILES: usize = 32;
pub const RECORDS_PER_TICK: usize = 256;
pub const HOSTS_PER_TICK: usize = 32;
pub const MOST_PROFILE_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserCache {
    Chrome,
    Chromium,
    Edge,
    Brave,
    Firefox,
    Safari,
}

pub const CACHES: [BrowserCache; 6] = [
    BrowserCache::Chrome,
    BrowserCache::Chromium,
    BrowserCache::Edge,
    BrowserCache::Brave,
    BrowserCache::Firefox,
    BrowserCache::Safari,
];

#[must_use]
pub fn cache_place<'a>(
    browser: BrowserCache,
    facts: &crate::browser_rules::BrowserFacts<'a>,
) -> Option<crate::browser_rules::Place<'a>> {
    use crate::browser_rules::{Base, Holds, Place};
    use crate::home_rules::Host;
    const CHROME_MAC: &[&str] = &["Library", "Application Support", "Google", "Chrome"];
    const CHROMIUM_MAC: &[&str] = &["Library", "Application Support", "Chromium"];
    const EDGE_MAC: &[&str] = &["Library", "Application Support", "Microsoft Edge"];
    const BRAVE_MAC: &[&str] = &[
        "Library",
        "Application Support",
        "BraveSoftware",
        "Brave-Browser",
    ];
    const FIREFOX_MAC: &[&str] = &["Library", "Application Support", "Firefox", "Profiles"];
    const SAFARI_MAC: &[&str] = &["Library", "Safari", "Favicon Cache", "favicons.db"];
    const CHROME_WINDOWS: &[&str] = &["Google", "Chrome", "User Data"];
    const CHROMIUM_WINDOWS: &[&str] = &["Chromium", "User Data"];
    const EDGE_WINDOWS: &[&str] = &["Microsoft", "Edge", "User Data"];
    const BRAVE_WINDOWS: &[&str] = &["BraveSoftware", "Brave-Browser", "User Data"];
    const FIREFOX_WINDOWS: &[&str] = &["Mozilla", "Firefox", "Profiles"];
    const FIREFOX_UNIX: &[&str] = &[".mozilla", "firefox"];
    let mut tail = match (browser, facts.host) {
        (BrowserCache::Chrome, Host::MacOs) => CHROME_MAC,
        (BrowserCache::Chromium, Host::MacOs) => CHROMIUM_MAC,
        (BrowserCache::Edge, Host::MacOs) => EDGE_MAC,
        (BrowserCache::Brave, Host::MacOs) => BRAVE_MAC,
        (BrowserCache::Firefox, Host::MacOs) => FIREFOX_MAC,
        (BrowserCache::Safari, Host::MacOs) => SAFARI_MAC,
        (BrowserCache::Chrome, Host::Windows) => CHROME_WINDOWS,
        (BrowserCache::Chromium, Host::Windows) => CHROMIUM_WINDOWS,
        (BrowserCache::Edge, Host::Windows) => EDGE_WINDOWS,
        (BrowserCache::Brave, Host::Windows) => BRAVE_WINDOWS,
        (BrowserCache::Firefox, Host::Windows) => FIREFOX_WINDOWS,
        (BrowserCache::Safari, Host::Windows | Host::Unix) => return None,
        (BrowserCache::Firefox, Host::Unix) => FIREFOX_UNIX,
        (BrowserCache::Chrome, Host::Unix) => &["google-chrome"],
        (BrowserCache::Chromium, Host::Unix) => &["chromium"],
        (BrowserCache::Edge, Host::Unix) => &["microsoft-edge"],
        (BrowserCache::Brave, Host::Unix) => &["BraveSoftware", "Brave-Browser"],
    };
    let configured = facts.host == Host::Unix && browser != BrowserCache::Firefox;
    let (base, value) = if facts.host == Host::Windows {
        if browser == BrowserCache::Firefox {
            (Base::AppData, facts.app_data)
        } else {
            (Base::LocalAppData, facts.local_app_data)
        }
    } else if configured && facts.xdg_config_home.is_some_and(|value| !value.is_empty()) {
        (Base::XdgConfigHome, facts.xdg_config_home)
    } else {
        (Base::Home, facts.home)
    };
    let root = value.filter(|value| !value.is_empty())?;
    if configured && base == Base::Home {
        tail = unix_home_tail(browser)?;
    }
    Some(Place {
        base,
        root,
        tail,
        holds: if browser == BrowserCache::Safari {
            Holds::File
        } else {
            Holds::Profiles
        },
    })
}

const fn unix_home_tail(browser: BrowserCache) -> Option<&'static [&'static str]> {
    match browser {
        BrowserCache::Chrome => Some(&[".config", "google-chrome"]),
        BrowserCache::Chromium => Some(&[".config", "chromium"]),
        BrowserCache::Edge => Some(&[".config", "microsoft-edge"]),
        BrowserCache::Brave => Some(&[".config", "BraveSoftware", "Brave-Browser"]),
        BrowserCache::Firefox | BrowserCache::Safari => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Quality {
    within: bool,
    score: u32,
}

#[must_use]
pub const fn quality(width: u32, height: u32) -> Option<Quality> {
    let side = if width >= height { width } else { height };
    if width == 0 || height == 0 || side > LARGEST_SIDE {
        None
    } else {
        Some(Quality {
            within: side <= SIDE,
            score: if side <= SIDE {
                side
            } else {
                u32::MAX.saturating_sub(side)
            },
        })
    }
}

#[must_use]
pub const fn snapshot_fits(pages: u64, page_size: u64) -> bool {
    if pages == 0
        || page_size < LEAST_PAGE
        || page_size > LARGEST_PAGE
        || !page_size.is_power_of_two()
    {
        return false;
    }
    matches!(pages.checked_mul(page_size), Some(bytes) if bytes <= MOST_DATABASE_BYTES)
}

#[cfg(kani)]
mod proofs {
    use super::{LARGEST_SIDE, MOST_DATABASE_BYTES, SIDE, quality, snapshot_fits};

    #[kani::proof]
    #[kani::unwind(8)]
    fn cache_roots_preserve_the_hosts_environment_conventions() {
        use super::{BrowserCache, cache_place};
        use crate::browser_rules::{Base, BrowserFacts, Holds};
        use crate::home_rules::Host;
        let browser = match kani::any::<u8>() {
            0 => BrowserCache::Chrome,
            1 => BrowserCache::Chromium,
            2 => BrowserCache::Edge,
            3 => BrowserCache::Brave,
            4 => BrowserCache::Firefox,
            _ => BrowserCache::Safari,
        };
        let host = match kani::any::<u8>() {
            0 => Host::Windows,
            1 => Host::MacOs,
            _ => Host::Unix,
        };
        let value = |tag: u8| match tag {
            0 => None,
            1 => Some(""),
            _ => Some("v"),
        };
        let facts = BrowserFacts {
            host,
            home: value(kani::any()),
            local_app_data: value(kani::any()),
            app_data: value(kani::any()),
            xdg_config_home: value(kani::any()),
        };
        let found = cache_place(browser, &facts);
        let expected = if browser == BrowserCache::Safari && host != Host::MacOs {
            None
        } else if host == Host::Windows {
            if browser == BrowserCache::Firefox {
                facts.app_data.map(|v| (Base::AppData, v))
            } else {
                facts.local_app_data.map(|v| (Base::LocalAppData, v))
            }
        } else if host == Host::Unix
            && browser != BrowserCache::Firefox
            && facts.xdg_config_home == Some("v")
        {
            Some((Base::XdgConfigHome, "v"))
        } else {
            facts.home.map(|v| (Base::Home, v))
        };
        kani::assert(
            found.map(|place| (place.base, place.root))
                == expected.filter(|(_, root)| !root.is_empty()),
            "only the host's nonempty environment root is used",
        );
        if let Some(place) = found {
            kani::assert(
                !place.tail.is_empty()
                    && (place.holds == Holds::File) == (browser == BrowserCache::Safari),
                "each cache names its own file or profile directory",
            );
        }
        kani::cover!(found.is_none(), "an unavailable cache");
        kani::cover!(
            found.is_some_and(|p| p.base == Base::XdgConfigHome),
            "an explicit configuration root"
        );
        kani::cover!(
            found.is_some() && host == Host::Windows,
            "a Windows profile root"
        );
    }

    #[kani::proof]
    fn bitmap_preference_keeps_the_largest_small_image_or_the_smallest_large_one() {
        let width: u32 = kani::any();
        let height: u32 = kani::any();
        let admitted = quality(width, height).is_some();
        kani::assert(
            admitted
                == (width > 0 && width <= LARGEST_SIDE && height > 0 && height <= LARGEST_SIDE),
            "only bounded nonempty dimensions are admitted",
        );
        kani::cover!(admitted, "admitted dimensions");
        kani::cover!(!admitted, "refused dimensions");
        let first: u32 = kani::any();
        let second: u32 = kani::any();
        kani::assume(first > 0 && first <= LARGEST_SIDE);
        kani::assume(second > 0 && second <= LARGEST_SIDE);
        let expected = if first <= SIDE && second <= SIDE {
            first.cmp(&second)
        } else if first > SIDE && second > SIDE {
            second.cmp(&first)
        } else {
            (first <= SIDE).cmp(&(second <= SIDE))
        };
        kani::assert(
            quality(first, first).cmp(&quality(second, second)) == expected,
            "the size preference is exact",
        );
        kani::cover!(first <= SIDE && second > SIDE, "prefer a small image");
        kani::cover!(first > SIDE && second > SIDE, "shrink a large image");
    }

    #[kani::proof]
    fn admitted_database_sizes_never_overflow_or_pass_the_snapshot_limit() {
        let pages: u64 = kani::any();
        let size: u64 = kani::any();
        let fits = snapshot_fits(pages, size);
        if fits {
            kani::assert(
                pages > 0
                    && pages
                        .checked_mul(size)
                        .is_some_and(|n| n <= MOST_DATABASE_BYTES),
                "an admitted snapshot fits without overflow",
            );
        }
        kani::cover!(fits, "an admitted snapshot");
        kani::cover!(!fits, "a refused snapshot");
    }

    #[kani::proof]
    fn rejects_preferring_a_larger_than_needed_bitmap() {
        kani::assert(quality(128, 128) > quality(64, 64), "NEGATIVE_ICON_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::{LARGEST_PAGE, MOST_DATABASE_BYTES, quality, snapshot_fits};

    #[test]
    fn browser_cache_paths_keep_platform_roots_and_configuration_fallbacks() {
        use super::{BrowserCache, CACHES, cache_place};
        use crate::browser_rules::{Base, BrowserFacts};
        use crate::home_rules::Host;
        for host in [Host::Windows, Host::MacOs, Host::Unix] {
            let facts = BrowserFacts {
                host,
                home: Some("/user"),
                local_app_data: Some("C:\\Users\\me\\AppData\\Local"),
                app_data: Some("C:\\Users\\me\\AppData\\Roaming"),
                xdg_config_home: Some("/cfg"),
            };
            for cache in CACHES {
                let found = cache_place(cache, &facts);
                if cache == BrowserCache::Safari && host != Host::MacOs {
                    assert_eq!(found, None);
                    continue;
                }
                let found = found.unwrap();
                let (base, root) = match (host, cache) {
                    (Host::Windows, BrowserCache::Firefox) => {
                        (Base::AppData, facts.app_data.unwrap())
                    }
                    (Host::Windows, _) => (Base::LocalAppData, facts.local_app_data.unwrap()),
                    (
                        Host::Unix,
                        BrowserCache::Chrome
                        | BrowserCache::Chromium
                        | BrowserCache::Edge
                        | BrowserCache::Brave,
                    ) => (Base::XdgConfigHome, "/cfg"),
                    _ => (Base::Home, "/user"),
                };
                assert_eq!((found.base, found.root), (base, root));
                let empty = BrowserFacts {
                    home: Some(""),
                    local_app_data: None,
                    app_data: Some(""),
                    xdg_config_home: None,
                    host,
                };
                assert_eq!(cache_place(cache, &empty), None);
            }
        }
        let unix = BrowserFacts {
            host: Host::Unix,
            home: Some("/user"),
            local_app_data: None,
            app_data: None,
            xdg_config_home: Some(""),
        };
        assert_eq!(
            cache_place(BrowserCache::Chrome, &unix).unwrap().tail,
            &[".config", "google-chrome"]
        );
    }

    #[test]
    fn image_and_snapshot_boundaries_are_exact() {
        assert!(quality(64, 64) > quality(32, 32));
        assert!(quality(32, 32) > quality(128, 128));
        assert!(quality(128, 128) > quality(256, 256));
        assert_eq!(quality(0, 32), None);
        assert_eq!(quality(32, 0), None);
        assert_eq!(quality(65535, 65535), None);
        assert!(snapshot_fits(
            MOST_DATABASE_BYTES / LARGEST_PAGE,
            LARGEST_PAGE
        ));
        assert!(!snapshot_fits(
            MOST_DATABASE_BYTES / LARGEST_PAGE + 1,
            LARGEST_PAGE
        ));
        assert!(!snapshot_fits(u64::MAX, LARGEST_PAGE));
        assert!(!snapshot_fits(0, 4096));
        assert!(!snapshot_fits(1, 513));
    }
}
