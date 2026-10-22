//! The bookmarks of Safari, Chrome, and Firefox, read from the files each browser keeps, as the folders and links of a bookmark file.
//!
//! Each reader keeps the folders and links in order and skips everything else, such as separators, history, and smart folders, so the import that follows merges them like any bookmark file.
//! Each writer writes a browser's file back from folders and links, which the tests and the fuzz target read again.

pub mod chrome;
pub mod firefox;
pub mod icons;
pub mod safari;

pub use youarel_html::bookmarks::Item;

/// The largest file of bookmarks a reader reads, and the largest Firefox backup once decompressed.
pub const LONGEST: usize = 64 * 1024 * 1024;
/// [`LONGEST`] as the file adapter counts bytes.
pub const LONGEST_FILE: u64 = 64 * 1024 * 1024;

/// Why a file is not a browser's bookmarks.
#[derive(Debug)]
pub enum ImportError {
    /// The file is larger than [`LONGEST`].
    Large(usize),
    /// The file is not the JSON the browser writes.
    Json(serde_json::Error),
    /// The file is not the property list Safari writes.
    Plist(plist::Error),
    /// The file is not the compressed backup Firefox writes.
    Compressed(&'static str),
    /// The file holds no bookmarks where the browser keeps them.
    Shape(&'static str),
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Large(length) => write!(
                formatter,
                "the bookmarks hold {length} bytes, more than the {LONGEST} an import reads"
            ),
            Self::Json(_) | Self::Plist(_) => formatter.write_str("not a browser's bookmarks"),
            Self::Compressed(why) => write!(formatter, "not a Firefox backup: {why}"),
            Self::Shape(why) => write!(formatter, "no bookmarks: {why}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Plist(error) => Some(error),
            Self::Large(_) | Self::Compressed(_) | Self::Shape(_) => None,
        }
    }
}

/// Refuses a file past [`LONGEST`].
const fn bounded(length: usize) -> Result<(), ImportError> {
    if length > LONGEST {
        Err(ImportError::Large(length))
    } else {
        Ok(())
    }
}

/// A folder of `items` named `name`, or none when it holds nothing.
fn folder(name: String, items: Vec<Item>) -> Option<Item> {
    (!items.is_empty()).then_some(Item::Folder { name, items })
}

#[cfg(test)]
mod tests {
    use super::{ImportError, LONGEST, bounded};

    #[test]
    fn errors_explain_themselves() {
        assert!(matches!(bounded(LONGEST + 1), Err(ImportError::Large(_))));
        bounded(LONGEST).unwrap();
        assert_eq!(u64::try_from(LONGEST).unwrap(), super::LONGEST_FILE);
        for error in [
            ImportError::Large(1),
            ImportError::Compressed("x"),
            ImportError::Shape("x"),
        ] {
            assert_ne!(error.to_string(), "");
            assert!(std::error::Error::source(&error).is_none());
        }
        let json = ImportError::Json(serde_json::from_str::<u8>("x").unwrap_err());
        assert!(std::error::Error::source(&json).is_some());
        assert_eq!(json.to_string(), "not a browser's bookmarks");
    }
}
