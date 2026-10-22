//! The icons a device gathered, one image for each host, which never leave the device.
//!
//! The store's database keeps the icon of each host with the time the device fetched it, and each image under the digest that names it.

use crate::codec::{DecodeError, from_hex, hex_digits};

/// The most bytes of one image the device keeps, which a small square icon fits many times over.
pub const LONGEST_IMAGE: u64 = 32 * 1024;

/// The identifier of an icon's image: the digest of its bytes, which the shell computes, so one image keeps one name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IconId(pub [u8; 32]);

impl IconId {
    /// The identifier that sixty-four lowercase hexadecimal digits name.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Identifier`] for any other text.
    pub fn parse(text: &str) -> Result<Self, DecodeError> {
        from_hex(text)
            .map(Self)
            .ok_or_else(|| DecodeError::Identifier(text.to_owned()))
    }

    /// The identifier as sixty-four lowercase hexadecimal digits, which [`IconId::parse`] reads back.
    #[must_use]
    pub fn name(self) -> String {
        hex_digits(&self.0)
    }
}

/// The icon a device holds for a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Icon {
    /// The image the host shows.
    pub image: IconId,
    /// When the device fetched the image, in milliseconds since the Unix epoch.
    pub fetched: u64,
}

#[cfg(test)]
mod tests {
    use super::IconId;
    use crate::codec::DecodeError;

    #[test]
    fn an_image_holds_at_most_thirty_two_kibibytes() {
        assert_eq!(super::LONGEST_IMAGE, 32 * 1024);
    }

    #[test]
    fn an_identifier_is_thirty_two_bytes_of_hexadecimal_and_reads_back_from_its_name() {
        let id = IconId([0xcd; 32]);
        assert_eq!(id.name(), "cd".repeat(32));
        assert_eq!(IconId::parse(&id.name()).unwrap(), id);
        for text in [
            "",
            "cd",
            &"CD".repeat(32),
            &"cd".repeat(33),
            &"zz".repeat(32),
        ] {
            assert!(
                matches!(IconId::parse(text), Err(DecodeError::Identifier(_))),
                "{text}"
            );
        }
    }

    /// Writes names of images into the seed corpus of their fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    fn seeds() {
        let corpus =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/icon_name");
        crate::fs::create_dir_all(&corpus).unwrap();
        let name = IconId([0xab; 32]).name();
        let seeds = [
            ("name", name.clone()),
            ("uppercase", name.to_uppercase()),
            ("one-byte", "ab".to_owned()),
            ("one-byte-more", format!("{name}ab")),
        ];
        for (file, text) in seeds {
            crate::fs::replace(&corpus.join(file), text.as_bytes()).unwrap();
        }
    }
}
