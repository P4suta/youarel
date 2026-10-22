use anyhow::{Context as _, Result};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore as _, SeedableRng as _};
use youarel_core::bench_rules::{Namespace, PURGED_PERIOD};
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::merge_rules::Register;
use youarel_core::position_rules::Key;
use youarel_core::record_rules::{Body, Record, encode_body};
use youarel_core::state::{Folder, ItemId, Presence, Shortcut};
use youarel_core::wire_rules::WireError;

pub(super) const NOW: u64 = 1_700_000_000_000;
pub(super) const WRITER: DeviceId = DeviceId([0x42; 16]);
pub(super) const SITES: u64 = 128;
const KEY_DIGITS: usize = 16;
pub(super) const ROOT_WEIGHT: usize = 4;
const FOLDER_WORDS: usize = 3;
const TITLE_BUCKETS: u64 = 20;
const LONG_TITLE_WORDS: usize = 48;
const MEDIUM_TITLE_WORDS: usize = 20;
const SHORT_TITLE_WORDS: usize = 6;
const LAST_MEDIUM_BUCKET: u64 = 5;
const NOTE_PERIOD: u64 = 8;
const NOTE_WORDS: usize = 120;
const ALIAS_PERIOD: u64 = 10;
const LINK_SCHEME: &str = "https://";
const SITE_DOMAIN: &str = "example";
const HISTORY: u64 = 5 * 365 * 86_400_000;
const WORDS: [&str; 16] = [
    "reference",
    "design",
    "meeting",
    "Rust",
    "guide",
    "notes",
    "release",
    "recipes",
    "Tokyo",
    "document",
    "project",
    "notebook",
    "photos",
    "東京",
    "manual",
    "research",
];

#[derive(Clone, Debug)]
pub(super) struct Generator {
    random: ChaCha20Rng,
}

const fn register<T>(value: T, stamp: Stamp) -> Register<T> {
    Register { value, stamp }
}

fn position(number: u64) -> Result<Key> {
    Key::parse(&format!("a{number:0KEY_DIGITS$x}1")).context("the synthetic position is invalid")
}

pub(super) fn site_host(site: u64) -> String {
    format!("site{site}.{SITE_DOMAIN}")
}

impl Generator {
    pub(super) fn new(seed: u64) -> Self {
        Self {
            random: ChaCha20Rng::seed_from_u64(seed),
        }
    }

    fn stamp(&mut self) -> Stamp {
        Stamp {
            wall: NOW.saturating_sub(self.random.next_u64() % HISTORY),
            counter: 0,
            device: WRITER,
        }
    }

    pub(super) fn pick(&mut self, count: usize) -> Result<usize> {
        if count == 0 {
            return Ok(0);
        }
        let modulus = u64::try_from(count)?;
        let index = self
            .random
            .next_u64()
            .checked_rem(modulus)
            .context("an empty random range")?;
        Ok(usize::try_from(index)?)
    }

    fn text(&mut self, count: usize) -> Result<String> {
        let mut words = Vec::with_capacity(count);
        for _ in 0..count {
            words.push(
                *WORDS
                    .get(self.pick(WORDS.len())?)
                    .context("the synthetic word is missing")?,
            );
        }
        Ok(words.join(" "))
    }

    pub(super) fn folder(
        &mut self,
        namespace: Namespace,
        number: u64,
        parent: Option<ItemId>,
    ) -> Result<Record> {
        let stamp = self.stamp();
        let id = namespace.id(number, stamp.wall);
        Ok(Record::Folder(
            id,
            Folder {
                name: register(format!("{} {number}", self.text(FOLDER_WORDS)?), stamp),
                mark: register(String::new(), stamp),
                parent: register(parent, stamp),
                position: register(position(number)?, stamp),
                presence: register(Presence::Live, stamp),
            },
        ))
    }

    pub(super) fn shortcut(
        &mut self,
        namespace: Namespace,
        number: u64,
        folder: Option<ItemId>,
    ) -> Result<Record> {
        let stamp = self.stamp();
        let id = namespace.id(number, stamp.wall);
        let site = self.random.next_u64() % SITES;
        let length = match self.random.next_u64() % TITLE_BUCKETS {
            0 => LONG_TITLE_WORDS,
            1..=LAST_MEDIUM_BUCKET => MEDIUM_TITLE_WORDS,
            _ => SHORT_TITLE_WORDS,
        };
        let title = self.text(length)?;
        let note = if self.random.next_u64().is_multiple_of(NOTE_PERIOD) {
            self.text(NOTE_WORDS)?
        } else {
            String::new()
        };
        let alias = if self.random.next_u64().is_multiple_of(ALIAS_PERIOD) {
            format!("shortcut{number}")
        } else {
            String::new()
        };
        let presence = if namespace == Namespace::Shortcut && number.is_multiple_of(PURGED_PERIOD) {
            Presence::Purged
        } else {
            Presence::Live
        };
        Ok(Record::Shortcut(
            id,
            Shortcut {
                link: register(
                    format!(
                        "{LINK_SCHEME}{}/documents/{number}/reference-{}",
                        site_host(site),
                        self.random.next_u64()
                    ),
                    stamp,
                ),
                title: register(title, stamp),
                alias: register(alias, stamp),
                note: register(note, stamp),
                folder: register(folder, stamp),
                position: register(position(number)?, stamp),
                presence: register(presence, stamp),
            },
        ))
    }
}

pub(super) fn bytes(record: &Record) -> Result<Vec<u8>, WireError> {
    let body = match record {
        Record::Folder(id, folder) => Body {
            devices: Vec::new(),
            folders: vec![(*id, folder.clone())],
            shortcuts: Vec::new(),
        },
        Record::Shortcut(id, shortcut) => Body {
            devices: Vec::new(),
            folders: Vec::new(),
            shortcuts: vec![(*id, shortcut.clone())],
        },
        Record::Device(id, name) => Body {
            devices: vec![(*id, name.clone())],
            ..Body::default()
        },
    };
    encode_body(&body)
}

#[cfg(test)]
mod tests {
    use super::{Generator, bytes};
    use youarel_core::bench_rules::Namespace;
    use youarel_core::record_rules::{Record, decode_body};
    use youarel_core::state::Presence;
    #[test]
    fn a_seed_reproduces_admitted_records_with_exactly_one_percent_purged() {
        let (mut first, mut second) = (Generator::new(17), Generator::new(17));
        let mut purged = 0;
        for number in 1..=200 {
            let left = first.shortcut(Namespace::Shortcut, number, None).unwrap();
            let right = second.shortcut(Namespace::Shortcut, number, None).unwrap();
            assert_eq!(left, right);
            let encoded = bytes(&left).unwrap();
            let decoded = decode_body(&encoded).unwrap();
            let Record::Shortcut(id, shortcut) = left else {
                panic!("the shortcut generator returned a folder")
            };
            assert_eq!(decoded.shortcuts, vec![(id, shortcut.clone())]);
            assert!(shortcut.link.value.starts_with("https://"));
            if shortcut.presence.value == Presence::Purged {
                purged += 1;
            }
        }
        assert_eq!(purged, 2);
        assert_ne!(
            first.shortcut(Namespace::Shortcut, 201, None).unwrap(),
            Generator::new(18)
                .shortcut(Namespace::Shortcut, 201, None)
                .unwrap()
        );
    }
}
