//! Chrome's `Bookmarks` file: JSON whose `roots` hold the bookmarks bar, the other bookmarks, and the mobile bookmarks, each a tree of folders and links.

use crate::{ImportError, Item, bounded, folder};
use serde_json::{Map, Value, json};

/// The roots Chrome keeps, in the order it shows them, with the names a root without one takes.
const ROOTS: [(&str, &str); 3] = [
    ("bookmark_bar", "Bookmarks bar"),
    ("other", "Other bookmarks"),
    ("synced", "Mobile bookmarks"),
];
/// The type of a link node.
const URL: &str = "url";
/// The type of a folder node.
const FOLDER: &str = "folder";

fn text<'a>(node: &'a Map<String, Value>, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The folders and links of a node's children.
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

/// One node as a folder or a link, or none for a node of another type.
fn item(node: &Map<String, Value>) -> Option<Item> {
    match text(node, "type") {
        URL => Some(Item::Link {
            href: text(node, "url").to_owned(),
            title: text(node, "name").to_owned(),
        }),
        FOLDER => Some(Item::Folder {
            name: text(node, "name").to_owned(),
            items: children(node),
        }),
        _ => None,
    }
}

/// The bookmarks of a Chrome `Bookmarks` file, each root a folder that holds something.
///
/// # Errors
///
/// Returns [`ImportError::Large`] past the longest file, [`ImportError::Json`] for text that is no JSON, and [`ImportError::Shape`] for JSON without Chrome's roots.
pub fn parse(text: &str) -> Result<Vec<Item>, ImportError> {
    bounded(text.len())?;
    let document: Value = serde_json::from_str(text).map_err(ImportError::Json)?;
    let roots = document
        .get("roots")
        .and_then(Value::as_object)
        .ok_or(ImportError::Shape("the file holds no roots"))?;
    Ok(ROOTS
        .iter()
        .filter_map(|(key, default)| {
            let root = roots.get(*key)?.as_object()?;
            let name = Some(self::text(root, "name"))
                .filter(|name| !name.is_empty())
                .unwrap_or(default);
            folder(name.to_owned(), children(root))
        })
        .collect())
}

fn node(item: &Item) -> Value {
    match item {
        Item::Link { href, title } => json!({ "type": URL, "name": title, "url": href }),
        Item::Folder { name, items } => {
            json!({ "type": FOLDER, "name": name, "children": items.iter().map(node).collect::<Vec<_>>() })
        }
    }
}

/// A Chrome `Bookmarks` file whose roots hold `items` in order, which [`parse`] reads back.
///
/// Each item at the top must be a folder, which becomes a root under its own name.
#[must_use]
pub fn write(items: &[Item]) -> String {
    let mut roots = Map::new();
    for ((key, _), root) in ROOTS.iter().zip(items) {
        let (name, inside) = match root {
            Item::Folder { name, items: held } => (name.as_str(), held.as_slice()),
            Item::Link { .. } => ("", core::slice::from_ref(root)),
        };
        roots.insert(
            (*key).to_owned(),
            json!({ "type": FOLDER, "name": name, "children": inside.iter().map(node).collect::<Vec<_>>() }),
        );
    }
    json!({ "version": 1, "roots": roots }).to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse, write};
    use crate::{ImportError, Item};

    fn link(href: &str, title: &str) -> Item {
        Item::Link {
            href: href.to_owned(),
            title: title.to_owned(),
        }
    }

    #[test]
    fn the_roots_become_folders_and_their_trees_keep_their_order() {
        let text = r#"{
          "checksum": "x", "version": 1,
          "roots": {
            "bookmark_bar": { "type": "folder", "name": "Bookmarks bar", "children": [
              { "type": "url", "name": "URL Standard", "url": "https://url.spec.whatwg.org/" },
              { "type": "folder", "name": "Specs", "children": [
                { "type": "url", "name": "HTML", "url": "https://html.spec.whatwg.org/" }
              ] },
              { "type": "unknown" }
            ] },
            "other": { "type": "folder", "name": "", "children": [
              { "type": "url", "name": "", "url": "https://example.com/" }
            ] },
            "synced": { "type": "folder", "name": "Mobile bookmarks", "children": [] }
          }
        }"#;
        assert_eq!(
            parse(text).unwrap(),
            [
                Item::Folder {
                    name: "Bookmarks bar".to_owned(),
                    items: vec![
                        link("https://url.spec.whatwg.org/", "URL Standard"),
                        Item::Folder {
                            name: "Specs".to_owned(),
                            items: vec![link("https://html.spec.whatwg.org/", "HTML")],
                        },
                    ],
                },
                Item::Folder {
                    name: "Other bookmarks".to_owned(),
                    items: vec![link("https://example.com/", "")],
                },
            ]
        );
    }

    #[test]
    fn a_written_file_reads_back() {
        let items = vec![
            Item::Folder {
                name: "Bar".to_owned(),
                items: vec![
                    link("https://a.example/", "A"),
                    Item::Folder {
                        name: "Inner".to_owned(),
                        items: vec![],
                    },
                ],
            },
            Item::Folder {
                name: "Other".to_owned(),
                items: vec![link("https://b.example/", "B")],
            },
        ];
        assert_eq!(parse(&write(&items)).unwrap(), items);
        let loose = vec![link("https://c.example/", "C")];
        assert_eq!(
            parse(&write(&loose)).unwrap(),
            [Item::Folder {
                name: "Bookmarks bar".to_owned(),
                items: loose
            }]
        );
    }

    #[test]
    fn what_is_no_chrome_file_is_refused() {
        assert!(matches!(parse("not json"), Err(ImportError::Json(_))));
        assert!(matches!(parse("{}"), Err(ImportError::Shape(_))));
        assert!(matches!(
            parse(r#"{"roots": []}"#),
            Err(ImportError::Shape(_))
        ));
        assert_eq!(parse(r#"{"roots": {"other": 3}}"#).unwrap(), []);
    }
}
