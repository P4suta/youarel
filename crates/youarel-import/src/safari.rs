//! Safari's `Bookmarks.plist`: a property list, binary or XML, whose children are the favorites bar, the bookmarks menu, the reading list, and the history's proxy.

use crate::{ImportError, Item, bounded, folder};
use plist::{Dictionary, Value};

/// The type of a folder.
const LIST: &str = "WebBookmarkTypeList";
/// The type of a link.
const LEAF: &str = "WebBookmarkTypeLeaf";
/// The names Safari keeps for its own folders, with the names a person sees.
const OWN_FOLDERS: [(&str, &str); 3] = [
    ("BookmarksBar", "Favorites"),
    ("BookmarksMenu", "Bookmarks Menu"),
    ("com.apple.ReadingList", "Reading List"),
];

fn text<'a>(node: &'a Dictionary, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_string).unwrap_or_default()
}

/// The name a person sees for a folder's title.
fn shown(title: &str) -> String {
    OWN_FOLDERS
        .iter()
        .find_map(|(own, name)| (*own == title).then_some(*name))
        .unwrap_or(title)
        .to_owned()
}

fn children(node: &Dictionary) -> Vec<Item> {
    node.get("Children")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_dictionary)
                .filter_map(item)
                .collect()
        })
        .unwrap_or_default()
}

/// One node as a folder or a link, or none for the history's proxy or another type.
fn item(node: &Dictionary) -> Option<Item> {
    match text(node, "WebBookmarkType") {
        LEAF => {
            let title = node
                .get("URIDictionary")
                .and_then(Value::as_dictionary)
                .map(|uri| text(uri, "title"))
                .unwrap_or_default();
            Some(Item::Link {
                href: text(node, "URLString").to_owned(),
                title: title.to_owned(),
            })
        }
        LIST => Some(Item::Folder {
            name: shown(text(node, "Title")),
            items: children(node),
        }),
        _ => None,
    }
}

/// The bookmarks of a Safari `Bookmarks.plist`, each folder at the top one that holds something.
///
/// # Errors
///
/// Returns [`ImportError::Large`] past the longest file, [`ImportError::Plist`] for bytes that are no property list, and [`ImportError::Shape`] for a list whose top is no folder.
pub fn parse(bytes: &[u8]) -> Result<Vec<Item>, ImportError> {
    bounded(bytes.len())?;
    let document = Value::from_reader(std::io::Cursor::new(bytes)).map_err(ImportError::Plist)?;
    let root = document
        .as_dictionary()
        .filter(|root| text(root, "WebBookmarkType") == LIST)
        .ok_or(ImportError::Shape("the list's top is no folder"))?;
    Ok(children(root)
        .into_iter()
        .filter_map(|top| match top {
            Item::Folder { name, items } => folder(name, items),
            Item::Link { .. } => Some(top),
        })
        .collect())
}

fn node(item: &Item) -> Value {
    let mut node = Dictionary::new();
    match item {
        Item::Link { href, title } => {
            let mut uri = Dictionary::new();
            uri.insert("title".to_owned(), Value::String(title.clone()));
            node.insert("WebBookmarkType".to_owned(), Value::String(LEAF.to_owned()));
            node.insert("URLString".to_owned(), Value::String(href.clone()));
            node.insert("URIDictionary".to_owned(), Value::Dictionary(uri));
        }
        Item::Folder { name, items } => {
            node.insert("WebBookmarkType".to_owned(), Value::String(LIST.to_owned()));
            node.insert("Title".to_owned(), Value::String(name.clone()));
            node.insert(
                "Children".to_owned(),
                Value::Array(items.iter().map(self::node).collect()),
            );
        }
    }
    Value::Dictionary(node)
}

/// A binary `Bookmarks.plist` that holds `items` at its top, which [`parse`] reads back.
///
/// # Errors
///
/// Returns [`ImportError::Plist`] when the property list cannot be written.
pub fn write(items: &[Item]) -> Result<Vec<u8>, ImportError> {
    let root = node(&Item::Folder {
        name: String::new(),
        items: items.to_vec(),
    });
    let mut bytes = Vec::new();
    root.to_writer_binary(&mut bytes)
        .map_err(ImportError::Plist)?;
    Ok(bytes)
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
    fn safari_s_own_folders_take_their_names_and_the_history_falls_away() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>WebBookmarkType</key><string>WebBookmarkTypeList</string>
  <key>Children</key><array>
    <dict><key>WebBookmarkType</key><string>WebBookmarkTypeProxy</string><key>Title</key><string>History</string></dict>
    <dict><key>WebBookmarkType</key><string>WebBookmarkTypeList</string><key>Title</key><string>BookmarksBar</string>
      <key>Children</key><array>
        <dict><key>WebBookmarkType</key><string>WebBookmarkTypeLeaf</string>
          <key>URLString</key><string>https://url.spec.whatwg.org/</string>
          <key>URIDictionary</key><dict><key>title</key><string>URL Standard</string></dict></dict>
      </array></dict>
    <dict><key>WebBookmarkType</key><string>WebBookmarkTypeList</string><key>Title</key><string>BookmarksMenu</string></dict>
    <dict><key>WebBookmarkType</key><string>WebBookmarkTypeLeaf</string><key>URLString</key><string>https://example.com/</string></dict>
  </array>
</dict></plist>"#;
        assert_eq!(
            parse(xml.as_bytes()).unwrap(),
            [
                Item::Folder {
                    name: "Favorites".to_owned(),
                    items: vec![link("https://url.spec.whatwg.org/", "URL Standard")],
                },
                link("https://example.com/", ""),
            ]
        );
    }

    #[test]
    fn a_written_list_reads_back() {
        let items = vec![
            Item::Folder {
                name: "Work".to_owned(),
                items: vec![link("https://a.example/", "A")],
            },
            link("https://b.example/", "B"),
        ];
        assert_eq!(parse(&write(&items).unwrap()).unwrap(), items);
    }

    #[test]
    fn what_is_no_safari_list_is_refused() {
        assert!(matches!(parse(b"not a plist"), Err(ImportError::Plist(_))));
        let array = r#"<?xml version="1.0"?><plist version="1.0"><array/></plist>"#;
        assert!(matches!(
            parse(array.as_bytes()),
            Err(ImportError::Shape(_))
        ));
        let source = parse(b"not a plist").unwrap_err();
        assert!(std::error::Error::source(&source).is_some());
    }
}
