//! Where youarel keeps its state on a device, decided from the environment's values alone.
//!
//! `YOUAREL_HOME` overrides every convention.
//! Otherwise Windows keeps data under `LOCALAPPDATA`, macOS under `HOME/Library/Application Support`, and every other host under `XDG_DATA_HOME` or `HOME/.local/share`.
//! An empty value counts as unset, so an empty variable never names a directory.

/// The kind of operating system, whose convention places the data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Host {
    Windows,
    MacOs,
    /// Linux and the other Unix desktops, which follow the XDG base directories.
    Unix,
}

/// What the environment says about where data lives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HomeFacts<'a> {
    /// `YOUAREL_HOME`, which overrides every convention.
    pub youarel_home: Option<&'a str>,
    /// `LOCALAPPDATA`, the per-user data base of Windows.
    pub local_app_data: Option<&'a str>,
    /// `XDG_DATA_HOME`, the data base of Unix desktops.
    pub xdg_data_home: Option<&'a str>,
    /// `HOME`, under which each Unix host has its conventional data base.
    pub home: Option<&'a str>,
    pub host: Host,
}

/// The directory of youarel under a base.
pub const DIRECTORY: &str = "youarel";
/// The segments of the macOS data base under the home directory.
pub const MACOS_DATA: [&str; 2] = ["Library", "Application Support"];
/// The segments of the Unix data base under the home directory, when `XDG_DATA_HOME` is unset.
pub const UNIX_DATA: [&str; 2] = [".local", "share"];

/// Which variable the data directory stands on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Base {
    YouarelHome,
    LocalAppData,
    XdgDataHome,
    Home,
}

/// The data directory: a base from the environment and the segments to join under it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataDir<'a> {
    pub base: Base,
    /// The value of the base variable.
    pub root: &'a str,
    /// The segments under the root, ending in youarel's directory unless `YOUAREL_HOME` names it.
    pub tail: &'static [&'static str],
}

/// The segments under `YOUAREL_HOME`, which already names the directory.
const DIRECT: [&str; 0] = [];
/// The segments under `LOCALAPPDATA` or `XDG_DATA_HOME`.
const UNDER_BASE: [&str; 1] = [DIRECTORY];
/// The segments under `HOME` on macOS.
const UNDER_MACOS_HOME: [&str; 3] = [MACOS_DATA[0], MACOS_DATA[1], DIRECTORY];
/// The segments under `HOME` on the other Unix hosts.
const UNDER_UNIX_HOME: [&str; 3] = [UNIX_DATA[0], UNIX_DATA[1], DIRECTORY];

/// A value that names something: present and not empty.
const fn usable(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        Some(_) | None => None,
    }
}

const fn under<'a>(
    base: Base,
    root: Option<&'a str>,
    tail: &'static [&'static str],
) -> Option<DataDir<'a>> {
    match root {
        Some(root) => Some(DataDir { base, root, tail }),
        None => None,
    }
}

/// The data directory the facts name, or `None` when no usable variable is set.
#[must_use]
pub const fn data_dir<'a>(facts: &HomeFacts<'a>) -> Option<DataDir<'a>> {
    if let Some(root) = usable(facts.youarel_home) {
        return Some(DataDir {
            base: Base::YouarelHome,
            root,
            tail: &DIRECT,
        });
    }
    match facts.host {
        Host::Windows => under(
            Base::LocalAppData,
            usable(facts.local_app_data),
            &UNDER_BASE,
        ),
        Host::MacOs => under(Base::Home, usable(facts.home), &UNDER_MACOS_HOME),
        Host::Unix => match usable(facts.xdg_data_home) {
            Some(root) => Some(DataDir {
                base: Base::XdgDataHome,
                root,
                tail: &UNDER_BASE,
            }),
            None => under(Base::Home, usable(facts.home), &UNDER_UNIX_HOME),
        },
    }
}

/// The properties Kani proves over every combination of set, empty, and unset variables on every kind of host.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Base, DataDir, HomeFacts, Host, data_dir, usable};

    /// A variable as the harness draws it: unset, empty, or a name.
    pub(super) const fn variable(choice: u8, name: &'static str) -> Option<&'static str> {
        match choice {
            0 => None,
            1 => Some(""),
            _ => Some(name),
        }
    }

    /// A host as the harness draws it.
    pub(super) const fn host(choice: u8) -> Host {
        match choice {
            0 => Host::Windows,
            1 => Host::MacOs,
            _ => Host::Unix,
        }
    }

    /// The decision restated: the first usable variable in the host's precedence, with the tail its base owns there.
    pub(super) fn spec(facts: &HomeFacts<'_>) -> bool {
        let own = usable(facts.youarel_home);
        let local = usable(facts.local_app_data);
        let xdg = usable(facts.xdg_data_home);
        let home = usable(facts.home);
        let conventional: Option<(Base, &str, &[&str])> = match facts.host {
            Host::Windows => local.map(|root| (Base::LocalAppData, root, &["youarel"][..])),
            Host::MacOs => home.map(|root| {
                (
                    Base::Home,
                    root,
                    &["Library", "Application Support", "youarel"][..],
                )
            }),
            Host::Unix => xdg.map_or_else(
                || home.map(|root| (Base::Home, root, &[".local", "share", "youarel"][..])),
                |root| Some((Base::XdgDataHome, root, &["youarel"][..])),
            ),
        };
        let expected = own.map_or(conventional, |root| {
            Some((Base::YouarelHome, root, &[][..]))
        });
        match (data_dir(facts), expected) {
            (None, None) => true,
            (
                Some(DataDir { base, root, tail }),
                Some((expected_base, expected_root, expected_tail)),
            ) => {
                base == expected_base
                    && root == expected_root
                    && tail == expected_tail
                    && !root.is_empty()
            }
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// The longest drawn name has ten bytes, so every byte comparison ends within eleven unwindings.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(21)]
    fn the_data_directory_follows_the_first_usable_variable_in_precedence() {
        let facts = HomeFacts {
            youarel_home: variable(kani::any(), "own-home"),
            local_app_data: variable(kani::any(), "local"),
            xdg_data_home: variable(kani::any(), "xdg"),
            home: variable(kani::any(), "home"),
            host: host(kani::any()),
        };
        kani::assert(spec(&facts), "the data directory follows the precedence");
        kani::cover!(data_dir(&facts).is_none(), "no variable names a directory");
        kani::cover!(
            matches!(
                data_dir(&facts),
                Some(DataDir {
                    base: Base::Home,
                    ..
                })
            ),
            "the home directory stands in"
        );
    }

    /// An empty `YOUAREL_HOME` must not name the directory; this probe claims it does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_an_empty_youarel_home() {
        let facts = HomeFacts {
            youarel_home: Some(""),
            local_app_data: None,
            xdg_data_home: None,
            home: None,
            host: Host::Unix,
        };
        kani::assert(data_dir(&facts).is_some(), "NEGATIVE_HOME_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{host, spec, variable};
    use super::{Base, DataDir, HomeFacts, Host, data_dir};

    const ALL: HomeFacts<'static> = HomeFacts {
        youarel_home: Some("/opt/youarel"),
        local_app_data: Some("C:\\Users\\me\\AppData\\Local"),
        xdg_data_home: Some("/xdg"),
        home: Some("/home/me"),
        host: Host::Unix,
    };

    #[test]
    fn youarel_home_wins_then_the_host_convention() {
        assert_eq!(
            data_dir(&ALL),
            Some(DataDir {
                base: Base::YouarelHome,
                root: "/opt/youarel",
                tail: &[],
            })
        );
        let unix = HomeFacts {
            youarel_home: None,
            ..ALL
        };
        assert_eq!(
            data_dir(&unix),
            Some(DataDir {
                base: Base::XdgDataHome,
                root: "/xdg",
                tail: &["youarel"],
            })
        );
        let home_only = HomeFacts {
            xdg_data_home: Some(""),
            ..unix
        };
        assert_eq!(
            data_dir(&home_only),
            Some(DataDir {
                base: Base::Home,
                root: "/home/me",
                tail: &[".local", "share", "youarel"],
            })
        );
        let mac = HomeFacts {
            host: Host::MacOs,
            ..unix
        };
        assert_eq!(
            data_dir(&mac),
            Some(DataDir {
                base: Base::Home,
                root: "/home/me",
                tail: &["Library", "Application Support", "youarel"],
            })
        );
        let windows = HomeFacts {
            host: Host::Windows,
            ..unix
        };
        assert_eq!(
            data_dir(&windows),
            Some(DataDir {
                base: Base::LocalAppData,
                root: "C:\\Users\\me\\AppData\\Local",
                tail: &["youarel"],
            })
        );
        let windows_without = HomeFacts {
            local_app_data: None,
            ..windows
        };
        assert_eq!(data_dir(&windows_without), None);
        let nothing = HomeFacts {
            youarel_home: None,
            local_app_data: None,
            xdg_data_home: None,
            home: None,
            host: Host::MacOs,
        };
        assert_eq!(data_dir(&nothing), None);
    }

    #[test]
    fn the_specification_agrees_over_every_drawn_combination() {
        for own in 0..3 {
            for local in 0..3 {
                for xdg in 0..3 {
                    for home in 0..3 {
                        for kind in 0..3 {
                            let facts = HomeFacts {
                                youarel_home: variable(own, "own-home"),
                                local_app_data: variable(local, "local"),
                                xdg_data_home: variable(xdg, "xdg"),
                                home: variable(home, "home"),
                                host: host(kind),
                            };
                            assert!(spec(&facts), "{facts:?}");
                        }
                    }
                }
            }
        }
    }
}
