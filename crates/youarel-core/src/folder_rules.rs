//! Which files in a shared folder count as another device's bundle, so the window merges after a change to one and after nothing else.
//!
//! A device names its bundle with its identifier and [`BUNDLE_SUFFIX`].
//! Cloud drives and operating systems leave other files beside it: hidden sidecars such as `._` files and `.DS_Store`, downloads under a suffix of their own until they finish, and lock files that start with [`LOCK_PREFIX`].
//! The window's own reads of the folder and this device's own bundle never count, or each merge and each publish would bring on another.

/// The suffix of a bundle file in a shared folder.
pub const BUNDLE_SUFFIX: &str = ".youarel";
/// The first character of a hidden file, such as the sidecar macOS writes beside a file on a drive that keeps no extended attributes.
pub const HIDDEN_PREFIX: char = '.';
/// The start of a lock file, such as an office suite or a cloud drive keeps beside a file it holds open.
pub const LOCK_PREFIX: &str = "~$";

/// What the watch of a shared folder saw.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Seen<'name> {
    /// A file was read, which changes nothing.
    Read,
    /// The file of this name was created, written, renamed, or removed.
    Touched(&'name str),
    /// The watch lost track of what changed, so any file may have.
    Rescan,
}

/// Whether what the watch saw may bring another device's bundle: a rescan always, a read never, and a touched file exactly when its name ends with the suffix after at least one character, starts with neither the hidden nor the lock mark, and is not `own`, this device's own bundle.
#[must_use]
pub fn counts(seen: Seen<'_>, own: Option<&str>) -> bool {
    match seen {
        Seen::Read => false,
        Seen::Touched(name) => {
            name.strip_suffix(BUNDLE_SUFFIX)
                .is_some_and(|stem| !stem.is_empty())
                && !name.starts_with(HIDDEN_PREFIX)
                && !name.starts_with(LOCK_PREFIX)
                && own != Some(name)
        }
        Seen::Rescan => true,
    }
}

/// The properties Kani proves over every name that a short text holds.
#[cfg(any(test, kani))]
mod proofs {
    use super::{BUNDLE_SUFFIX, HIDDEN_PREFIX, LOCK_PREFIX, Seen, counts};

    /// What counting promises, restated over the name's bytes and characters: a read never counts, a rescan always does, and a touched name counts exactly when the suffix ends it after at least one byte, its first character is no hidden mark, its first two bytes are no lock mark, and its bytes differ from the own bundle's.
    pub(super) fn spec(seen: Seen<'_>, own: Option<&str>) -> bool {
        let counted = counts(seen, own);
        match seen {
            Seen::Read => !counted,
            Seen::Rescan => counted,
            Seen::Touched(name) => {
                let bytes = name.as_bytes();
                let suffix = BUNDLE_SUFFIX.as_bytes();
                let bundle = bytes.len() > suffix.len()
                    && bytes
                        .get(bytes.len().saturating_sub(suffix.len())..)
                        .is_some_and(|end| end == suffix);
                let hidden = bytes
                    .first()
                    .is_some_and(|first| char::from(*first) == HIDDEN_PREFIX);
                let locked = bytes.get(..LOCK_PREFIX.len()) == Some(LOCK_PREFIX.as_bytes());
                let mine = own.is_some_and(|held| held.as_bytes() == bytes);
                counted == (bundle && !hidden && !locked && !mine)
            }
        }
    }

    /// A text whose parts are every kind of name a shared folder holds: a lock mark, a hidden mark, a stem, the bundle suffix, and the suffix of a partial download.
    ///
    /// Its parts hold a bundle, a hidden sidecar and a lock file of it, a suffix with no stem, a partial download, and names with no suffix.
    pub(super) const TEXT: &str = "~$._a.youarel.tmp";

    /// Another device's bundle, which a part of the text and a drawn name spell too.
    pub(super) const OTHER: &str = "a.youarel";

    /// Every part of the text as a name, with every own bundle that matters: none, the name itself, and another device's.
    ///
    /// A part of a fixed text keeps the name's bytes known and only its ends unknown, where bytes joined at run time would leave the solver to check them as text.
    /// The longest loop compares the name with the own bundle byte by byte, and the text holds seventeen bytes, so eighteen unwindings end every loop.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(18)]
    fn only_another_device_s_bundle_counts() {
        let start: usize = kani::any();
        let end: usize = kani::any();
        let Some(name) = TEXT.get(start..end) else {
            return;
        };
        let own = match kani::any::<u8>() {
            0 => None,
            1 => Some(name),
            _ => Some(OTHER),
        };
        for seen in [Seen::Read, Seen::Touched(name), Seen::Rescan] {
            kani::assert(spec(seen, own), "only another device's bundle counts");
        }
        kani::cover!(
            counts(Seen::Touched(name), own),
            "another device's bundle counts"
        );
        kani::cover!(
            own == Some(name) && counts(Seen::Touched(name), None),
            "the own bundle counts for nothing"
        );
        kani::cover!(
            name.starts_with(HIDDEN_PREFIX)
                && name.ends_with(BUNDLE_SUFFIX)
                && name.len() > BUNDLE_SUFFIX.len() + 1,
            "a hidden sidecar of a bundle counts for nothing"
        );
        kani::cover!(
            name.starts_with(LOCK_PREFIX) && name.ends_with(BUNDLE_SUFFIX),
            "a lock file of a bundle counts for nothing"
        );
    }

    /// A device's own bundle never counts, or its every publish would bring on a merge; this probe claims it counts and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_the_device_s_own_bundle() {
        kani::assert(
            counts(Seen::Touched("ab.youarel"), Some("ab.youarel")),
            "NEGATIVE_FOLDER_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{OTHER, TEXT, spec};
    use super::{BUNDLE_SUFFIX, LOCK_PREFIX, Seen, counts};
    use alloc::format;
    use alloc::string::String;

    /// The bundle of another device, named by its identifier.
    const BUNDLE: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0.youarel";

    /// The pieces a drawn name joins: nothing, a letter, a dot, the lock mark, the suffix, and the suffix of a partial download.
    const PIECES: [&str; 6] = ["", "a", ".", LOCK_PREFIX, BUNDLE_SUFFIX, ".tmp"];

    #[test]
    fn another_device_s_bundle_counts_and_what_lies_beside_it_does_not() {
        assert!(counts(Seen::Touched(BUNDLE), None));
        assert!(counts(Seen::Touched(BUNDLE), Some(OTHER)));
        assert!(
            !counts(Seen::Touched(BUNDLE), Some(BUNDLE)),
            "this device's own bundle"
        );
        let sidecars = [
            format!("._{BUNDLE}"),
            format!("{BUNDLE}.sdtmp"),
            format!("~${BUNDLE}"),
            ".DS_Store".into(),
            "x.staging".into(),
            ".youarel".into(),
            "youarel".into(),
            String::new(),
        ];
        for name in &sidecars {
            assert!(!counts(Seen::Touched(name), None), "{name:?}");
        }
        assert!(!counts(Seen::Read, None), "a read changes nothing");
        assert!(
            counts(Seen::Rescan, Some(BUNDLE)),
            "a rescan may hide anything"
        );
    }

    /// Whether the specification holds for every change to `name` with every own bundle that matters.
    fn agrees(name: &str) -> bool {
        [None, Some(name), Some(OTHER)].into_iter().all(|own| {
            [Seen::Read, Seen::Touched(name), Seen::Rescan]
                .into_iter()
                .all(|seen| spec(seen, own))
        })
    }

    #[test]
    fn the_specification_agrees_over_every_part_of_the_text() {
        for start in 0..=TEXT.len() {
            for end in start..=TEXT.len() {
                let name = TEXT.get(start..end).unwrap();
                assert!(agrees(name), "{name:?}");
            }
        }
    }

    #[test]
    fn the_specification_agrees_over_drawn_names() {
        for first in PIECES {
            for second in PIECES {
                for third in PIECES {
                    let name = [first, second, third].concat();
                    assert!(agrees(&name), "{name:?}");
                }
            }
        }
    }
}
