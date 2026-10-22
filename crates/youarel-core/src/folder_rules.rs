//! The names of peer chains and immutable segment files in a shared folder.

use crate::segment_rules::Sequence;
use alloc::format;
use alloc::string::String;

pub const SEGMENT_SUFFIX: &str = ".youarel";
pub const HEAD_FILE: &str = "head";
pub const SEQUENCE_DIGITS: usize = 20;
pub const PEER_DIGITS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SegmentName(String);

impl SegmentName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for SegmentName {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Seen<'name> {
    Read,
    /// A normalized path relative to the shared folder, with forward slashes.
    Touched(&'name str),
    Rescan,
}

#[must_use]
pub fn peer(name: &str) -> bool {
    name.len() == PEER_DIGITS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[must_use]
pub fn segment_name(sequence: Sequence) -> SegmentName {
    SegmentName(format!(
        "{:0SEQUENCE_DIGITS$}{SEGMENT_SUFFIX}",
        sequence.get()
    ))
}

#[must_use]
pub fn segment_sequence(name: &str) -> Option<Sequence> {
    let stem = name.strip_suffix(SEGMENT_SUFFIX)?;
    if stem.len() != SEQUENCE_DIGITS || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let Ok(number) = stem.parse::<u64>() else {
        return None;
    };
    Sequence::new(number)
}

/// Reads do not count, rescans always count, and only another peer's directory, head, or complete segment name counts as a change.
#[must_use]
pub fn counts(seen: Seen<'_>, own: Option<&str>) -> bool {
    match seen {
        Seen::Read => false,
        Seen::Rescan => true,
        Seen::Touched(name) => {
            let (device, file) = name
                .split_once('/')
                .map_or((name, None), |(device, file)| (device, Some(file)));
            peer(device)
                && own != Some(device)
                && file.is_none_or(|file| file == HEAD_FILE || segment_sequence(file).is_some())
        }
    }
}

#[cfg(any(test, kani))]
mod cases {
    pub(super) const OWN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    pub(super) const OTHER: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";
    pub(super) const HEAD: &str =
        "2222222222222222222222222222222222222222222222222222222222222222/head";
    pub(super) const SEGMENT: &str = "2222222222222222222222222222222222222222222222222222222222222222/00000000000000000001.youarel";
    pub(super) const NAMES: [(&str, bool); 10] = [
        (HEAD, true),
        (SEGMENT, true),
        (OTHER, true),
        (
            "2222222222222222222222222222222222222222222222222222222222222222/head.staging",
            false,
        ),
        (
            "2222222222222222222222222222222222222222222222222222222222222222/._00000000000000000001.youarel",
            false,
        ),
        (
            "2222222222222222222222222222222222222222222222222222222222222222/00000000000000000000.youarel",
            false,
        ),
        (
            "2222222222222222222222222222222222222222222222222222222222222222/18446744073709551616.youarel",
            false,
        ),
        (
            "2222222222222222222222222222222222222222222222222222222222222222.youarel",
            false,
        ),
        ("bad/00000000000000000001.youarel", false),
        (".DS_Store", false),
    ];
}

#[cfg(kani)]
mod proofs {
    use super::cases::{NAMES, OTHER, OWN, SEGMENT};
    use super::{Seen, counts, segment_name, segment_sequence};
    use crate::segment_rules::Sequence;

    #[kani::proof]
    #[kani::unwind(128)]
    fn only_another_device_s_chain_counts() {
        let index: usize = kani::any();
        kani::assume(index < NAMES.len());
        let (name, valid) = NAMES[index];
        let own = match kani::any::<u8>() {
            0 => None,
            1 => Some(OWN),
            _ => Some(OTHER),
        };
        kani::assert(!counts(Seen::Read, own), "a read changes nothing");
        kani::assert(counts(Seen::Rescan, own), "a rescan may hide a peer change");
        let expected = valid && own != Some(OTHER);
        kani::assert(
            counts(Seen::Touched(name), own) == expected,
            "only another chain's finished names count",
        );
        kani::cover!(valid && expected, "a peer change counts");
        kani::cover!(valid && !expected, "the own chain never counts");
        kani::cover!(!valid, "an unfinished or unrelated name never counts");
    }

    #[kani::proof]
    #[kani::unwind(128)]
    fn every_sequence_file_reads_back_canonically() {
        let value: u64 = kani::any();
        let Some(sequence) = Sequence::new(value) else {
            return;
        };
        let name = segment_name(sequence);
        kani::assert(
            segment_sequence(name.as_str()) == Some(sequence),
            "every sequence names one canonical file",
        );
        kani::cover!(value == u64::MAX, "the last sequence fits twenty digits");
        kani::cover!(value == 1, "the chain starts at one");
    }

    #[kani::proof]
    #[kani::unwind(128)]
    fn rejects_the_device_s_own_chain() {
        kani::assert(
            counts(Seen::Touched(SEGMENT), Some(OTHER)),
            "NEGATIVE_FOLDER_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::cases::{HEAD, NAMES, OTHER, OWN, SEGMENT};
    use super::{Seen, counts, peer, segment_name, segment_sequence};
    use crate::segment_rules::Sequence;
    use alloc::format;

    #[test]
    fn a_peer_head_and_segment_count_but_the_own_chain_and_sidecars_do_not() {
        assert!(counts(Seen::Touched(HEAD), None));
        assert!(counts(Seen::Touched(SEGMENT), Some(OWN)));
        assert!(!counts(Seen::Touched(SEGMENT), Some(OTHER)));
        assert!(!counts(Seen::Touched(HEAD), Some(OTHER)));
        for (name, valid) in NAMES {
            assert_eq!(counts(Seen::Touched(name), None), valid, "{name}");
            assert!(!counts(Seen::Touched(name), Some(OTHER)), "{name}");
            assert!(!counts(Seen::Read, None));
            assert!(counts(Seen::Rescan, Some(OTHER)));
        }
        for name in [
            format!("{OTHER}/1.youarel"),
            format!("{OTHER}/nested/head"),
            format!("{OTHER}/名.youarel"),
            format!("{OTHER}\\head"),
        ] {
            assert!(!counts(Seen::Touched(&name), None), "{name}");
        }
        assert!(!peer(&OTHER.replace('2', "A")));
    }

    #[test]
    fn sequence_files_preserve_numeric_order_and_refuse_aliases() {
        let mut previous = None;
        for number in [1, 9, 10, 99, 100, u64::MAX] {
            let sequence = Sequence::new(number).unwrap();
            let name = segment_name(sequence);
            assert_eq!(segment_sequence(name.as_str()), Some(sequence));
            assert!(previous.as_ref().is_none_or(|before| before < &name));
            previous = Some(name);
        }
        for name in [
            "00000000000000000000.youarel",
            "18446744073709551616.youarel",
            "+0000000000000000001.youarel",
            "1.youarel",
            "00000000000000000001.youarel.tmp",
        ] {
            assert_eq!(segment_sequence(name), None, "{name}");
        }
    }
}
