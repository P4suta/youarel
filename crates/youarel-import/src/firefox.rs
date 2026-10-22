//! Firefox's backups of bookmarks: `mozLz4` files, a magic, the decompressed length, and one LZ4 block of the JSON tree of the bookmarks.

use crate::{ImportError, Item, LONGEST, bounded, folder};
use serde_json::{Map, Value, json};

/// The magic that starts a `mozLz4` file.
pub const MAGIC: &[u8; 8] = b"mozLz40\0";
/// The bytes of the decompressed length after the magic, little-endian.
const LENGTH_BYTES: usize = 4;
/// The type of a folder.
const CONTAINER: &str = "text/x-moz-place-container";
/// The type of a link.
const PLACE: &str = "text/x-moz-place";
/// The roots Firefox keeps, by their `root` field, with the names a root without a title takes.
const ROOTS: [(&str, &str); 4] = [
    ("toolbarFolder", "Bookmarks Toolbar"),
    ("bookmarksMenuFolder", "Bookmarks Menu"),
    ("unfiledBookmarksFolder", "Other Bookmarks"),
    ("mobileFolder", "Mobile Bookmarks"),
];

fn text<'a>(node: &'a Map<String, Value>, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn children(node: &Map<String, Value>) -> Vec<Item> {
    node.get("children")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_object)
                .filter_map(item)
                .collect()
        })
        .unwrap_or_default()
}

/// One node as a folder or a link, or none for a separator or another type.
fn item(node: &Map<String, Value>) -> Option<Item> {
    match text(node, "type") {
        PLACE => Some(Item::Link {
            href: text(node, "uri").to_owned(),
            title: text(node, "title").to_owned(),
        }),
        CONTAINER => Some(Item::Folder {
            name: text(node, "title").to_owned(),
            items: children(node),
        }),
        _ => None,
    }
}

/// The JSON a `mozLz4` file compresses.
///
/// # Errors
///
/// Returns [`ImportError::Compressed`] for a file without the magic, with a length past the longest, or with a block that does not decompress to its length.
fn decompressed(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    bounded(bytes.len())?;
    let rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or(ImportError::Compressed("the file lacks the mozLz4 magic"))?;
    let (length, block) = rest
        .split_first_chunk::<LENGTH_BYTES>()
        .ok_or(ImportError::Compressed("the file ends before its length"))?;
    let length = usize::try_from(u32::from_le_bytes(*length))
        .map_err(|_overflow| ImportError::Compressed("the length does not fit"))?;
    if length > LONGEST {
        return Err(ImportError::Large(length));
    }
    lz4_flex::block::decompress(block, length)
        .map_err(|_corrupt| ImportError::Compressed("the block does not decompress"))
}

/// The bookmarks of a Firefox backup, each root a folder that holds something.
///
/// # Errors
///
/// Returns [`ImportError::Compressed`] or [`ImportError::Large`] for a file that does not decompress, [`ImportError::Json`] for a tree that is no JSON, and [`ImportError::Shape`] for JSON without Firefox's roots.
pub fn decode(bytes: &[u8]) -> Result<Vec<Item>, ImportError> {
    let json = decompressed(bytes)?;
    let document: Value = serde_json::from_slice(&json).map_err(ImportError::Json)?;
    let roots = document
        .get("children")
        .and_then(Value::as_array)
        .ok_or(ImportError::Shape("the tree holds no roots"))?;
    Ok(ROOTS
        .iter()
        .filter_map(|(key, default)| {
            let root = roots
                .iter()
                .filter_map(Value::as_object)
                .find(|root| text(root, "root") == *key)?;
            let name = Some(text(root, "title"))
                .filter(|name| !name.is_empty())
                .unwrap_or(default);
            folder(name.to_owned(), children(root))
        })
        .collect())
}

fn node(item: &Item) -> Value {
    match item {
        Item::Link { href, title } => json!({ "type": PLACE, "title": title, "uri": href }),
        Item::Folder { name, items } => {
            json!({ "type": CONTAINER, "title": name, "children": items.iter().map(node).collect::<Vec<_>>() })
        }
    }
}

/// A Firefox backup whose roots hold `items` in order, which [`decode`] reads back.
///
/// Each item at the top must be a folder, which becomes a root under its own name.
///
/// # Errors
///
/// Returns [`ImportError::Large`] when the tree is longer than a backup's length can say.
pub fn encode(items: &[Item]) -> Result<Vec<u8>, ImportError> {
    let roots: Vec<Value> = ROOTS
        .iter()
        .zip(items)
        .map(|((key, _), root)| {
            let (name, inside) = match root {
                Item::Folder {
                    name,
                    items: held,
                } => (name.as_str(), held.as_slice()),
                Item::Link { .. } => ("", core::slice::from_ref(root)),
            };
            json!({ "type": CONTAINER, "root": key, "title": name, "children": inside.iter().map(node).collect::<Vec<_>>() })
        })
        .collect();
    let tree = json!({ "type": CONTAINER, "root": "placesRoot", "children": roots }).to_string();
    let length = u32::try_from(tree.len()).map_err(|_overflow| ImportError::Large(tree.len()))?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend(lz4_flex::block::compress(tree.as_bytes()));
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{MAGIC, decode, encode};
    use crate::{ImportError, Item};

    fn link(href: &str, title: &str) -> Item {
        Item::Link {
            href: href.to_owned(),
            title: title.to_owned(),
        }
    }

    fn compressed(json: &str) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
        bytes.extend(lz4_flex::block::compress(json.as_bytes()));
        bytes
    }

    #[test]
    fn the_roots_become_folders_and_separators_fall_away() {
        let json = r#"{ "type": "text/x-moz-place-container", "root": "placesRoot", "children": [
          { "type": "text/x-moz-place-container", "root": "bookmarksMenuFolder", "title": "menu", "children": [
            { "type": "text/x-moz-place", "title": "URL Standard", "uri": "https://url.spec.whatwg.org/" },
            { "type": "text/x-moz-place-separator" },
            { "type": "text/x-moz-place-container", "title": "Specs", "children": [] }
          ] },
          { "type": "text/x-moz-place-container", "root": "toolbarFolder", "title": "", "children": [
            { "type": "text/x-moz-place", "title": "Example", "uri": "https://example.com/" }
          ] },
          { "type": "text/x-moz-place-container", "root": "mobileFolder", "title": "mobile" }
        ] }"#;
        assert_eq!(
            decode(&compressed(json)).unwrap(),
            [
                Item::Folder {
                    name: "Bookmarks Toolbar".to_owned(),
                    items: vec![link("https://example.com/", "Example")],
                },
                Item::Folder {
                    name: "menu".to_owned(),
                    items: vec![
                        link("https://url.spec.whatwg.org/", "URL Standard"),
                        Item::Folder {
                            name: "Specs".to_owned(),
                            items: vec![]
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn an_encoded_backup_decodes_back() {
        let items = vec![
            Item::Folder {
                name: "Toolbar".to_owned(),
                items: vec![link("https://a.example/", "A")],
            },
            Item::Folder {
                name: "Menu".to_owned(),
                items: vec![link("https://b.example/", "B")],
            },
        ];
        assert_eq!(decode(&encode(&items).unwrap()).unwrap(), items);
    }

    #[test]
    fn what_is_no_backup_is_refused() {
        assert!(matches!(
            decode(b"plain json"),
            Err(ImportError::Compressed(_))
        ));
        assert!(matches!(
            decode(b"mozLz40\0\x01"),
            Err(ImportError::Compressed(_))
        ));
        let mut wrong = compressed("{}");
        if let Some(last) = wrong.last_mut() {
            *last ^= 0xff;
        }
        decode(&wrong).unwrap_err();
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&huge), Err(ImportError::Large(_))));
        // A length of exactly the longest is no refusal for size, and its empty block fails only to decompress.
        let mut longest = MAGIC.to_vec();
        longest.extend_from_slice(&u32::try_from(crate::LONGEST).unwrap().to_le_bytes());
        assert!(matches!(decode(&longest), Err(ImportError::Compressed(_))));
        assert!(matches!(
            decode(&compressed("[1")),
            Err(ImportError::Json(_))
        ));
        assert!(matches!(
            decode(&compressed("{}")),
            Err(ImportError::Shape(_))
        ));
    }
}
