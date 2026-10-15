//! Bookmark files of the Netscape format, which every desktop browser exports and imports.
//!
//! A folder is a `<DT><H3>` heading followed by a `<DL>` list of its items, and a link is a `<DT><A HREF>` element.
//! Reading keeps the folders and links in order and skips everything else, so a file from any browser reads.

use crate::scan::{Token, escape, tokens};

/// One entry of a bookmark file.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Item {
    Folder { name: String, items: Vec<Self> },
    Link { href: String, title: String },
}

/// Text with runs of white space collapsed to one space and trimmed.
fn tidy(text: &str) -> String {
    let mut tidied = String::new();
    for word in text.split_whitespace() {
        if !tidied.is_empty() {
            tidied.push(' ');
        }
        tidied.push_str(word);
    }
    tidied
}

/// What text the reader collects.
#[derive(Clone, Debug)]
enum Capture {
    Nothing,
    FolderName(String),
    LinkTitle { href: String, title: String },
}

/// The reader's place: the open folders, each with its name and the items read so far.
#[derive(Debug)]
struct Reader {
    frames: Vec<(String, Vec<Item>)>,
    top: Vec<Item>,
    /// A folder heading whose list has not opened yet.
    pending: Option<String>,
    capture: Capture,
}

impl Reader {
    fn current(&mut self) -> &mut Vec<Item> {
        match self.frames.last_mut() {
            Some((_, items)) => items,
            None => &mut self.top,
        }
    }

    /// Adds a heading that never opened a list as an empty folder.
    fn settle(&mut self) {
        if let Some(name) = self.pending.take() {
            self.current().push(Item::Folder {
                name,
                items: Vec::new(),
            });
        }
    }

    fn close_folder(&mut self) {
        if let Some((name, items)) = self.frames.pop() {
            self.current().push(Item::Folder { name, items });
        }
    }
}

/// The items a bookmark file holds, in order.
#[must_use]
pub fn parse(text: &str) -> Vec<Item> {
    let mut reader = Reader {
        frames: Vec::new(),
        top: Vec::new(),
        pending: None,
        capture: Capture::Nothing,
    };
    let mut opened_top = false;
    for token in tokens(text) {
        match token {
            Token::Text(piece) => match &mut reader.capture {
                Capture::FolderName(name) => name.push_str(&piece),
                Capture::LinkTitle { title, .. } => title.push_str(&piece),
                Capture::Nothing => {}
            },
            Token::Tag(tag) => match (tag.name.as_str(), tag.closing) {
                ("h3", false) => {
                    reader.settle();
                    reader.capture = Capture::FolderName(String::new());
                }
                ("h3", true) => {
                    if let Capture::FolderName(name) = &reader.capture {
                        reader.pending = Some(tidy(name));
                    }
                    reader.capture = Capture::Nothing;
                }
                ("a", false) => {
                    reader.settle();
                    reader.capture = Capture::LinkTitle {
                        href: tag.attribute("href").unwrap_or_default().to_owned(),
                        title: String::new(),
                    };
                }
                ("a", true) => {
                    if let Capture::LinkTitle { href, title } =
                        core::mem::replace(&mut reader.capture, Capture::Nothing)
                    {
                        let title = tidy(&title);
                        reader.current().push(Item::Link { href, title });
                    }
                }
                ("dl", false) => {
                    if let Some(name) = reader.pending.take() {
                        reader.frames.push((name, Vec::new()));
                    } else if opened_top {
                        reader.frames.push((String::new(), Vec::new()));
                    }
                    opened_top = true;
                }
                ("dl", true) => {
                    reader.settle();
                    reader.close_folder();
                }
                _ => {}
            },
        }
    }
    reader.settle();
    // A file cut short leaves folders open, and each close takes one off the stack.
    for _ in 0..reader.frames.len() {
        reader.close_folder();
    }
    reader.top
}

/// Appends one line at a depth of indentation.
fn line(out: &mut String, depth: usize, pieces: &[&str]) {
    out.push_str(&"    ".repeat(depth));
    for piece in pieces {
        out.push_str(piece);
    }
    out.push('\n');
}

fn write_items(items: &[Item], depth: usize, out: &mut String) {
    for item in items {
        match item {
            Item::Folder { name, items: held } => {
                line(out, depth, &["<DT><H3>", &escape(name), "</H3>"]);
                line(out, depth, &["<DL><p>"]);
                write_items(held, depth.saturating_add(1), out);
                line(out, depth, &["</DL><p>"]);
            }
            Item::Link { href, title } => {
                line(
                    out,
                    depth,
                    &[
                        "<DT><A HREF=\"",
                        &escape(href),
                        "\">",
                        &escape(title),
                        "</A>",
                    ],
                );
            }
        }
    }
}

/// A bookmark file holding `items`, which browsers import.
#[must_use]
pub fn write(items: &[Item]) -> String {
    let mut out = String::from(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n",
    );
    write_items(items, 1, &mut out);
    out.push_str("</DL><p>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{Item, parse, write};

    fn link(href: &str, title: &str) -> Item {
        Item::Link {
            href: href.to_owned(),
            title: title.to_owned(),
        }
    }

    fn folder(name: &str, items: Vec<Item>) -> Item {
        Item::Folder {
            name: name.to_owned(),
            items,
        }
    }

    #[test]
    fn a_chrome_export_reads_with_its_folders() {
        let exported = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<!-- This is an automatically generated file. -->
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1" LAST_MODIFIED="2" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://github.com/" ADD_DATE="3" ICON="data:image/png;base64,AAA">GitHub</A>
        <DT><H3>仕事</H3>
        <DL><p>
            <DT><A HREF="https://example.com/?a=1&amp;b=2">A &amp; B</A>
        </DL><p>
        <DT><H3>Empty</H3>
        <DL><p>
        </DL><p>
    </DL><p>
    <DT><A HREF="javascript:void(0)">Bookmarklet</A>
</DL><p>
"#;
        assert_eq!(
            parse(exported),
            [
                folder(
                    "Bookmarks bar",
                    vec![
                        link("https://github.com/", "GitHub"),
                        folder("仕事", vec![link("https://example.com/?a=1&b=2", "A & B")]),
                        folder("Empty", vec![]),
                    ]
                ),
                link("javascript:void(0)", "Bookmarklet"),
            ]
        );
    }

    #[test]
    fn written_items_read_back() {
        let items = vec![
            folder(
                "a \"quoted\" <name>",
                vec![link("https://x.example/?q=<>&", "t & u")],
            ),
            link("https://y.example/", ""),
            folder("empty", vec![]),
        ];
        assert_eq!(parse(&write(&items)), items);
        assert_eq!(parse(&write(&[])), []);
    }

    #[test]
    fn a_heading_without_a_list_and_a_truncated_file_still_read() {
        assert_eq!(
            parse("<DL><DT><H3>lonely</H3><DT><A HREF=\"h\">x</A>"),
            [folder("lonely", vec![]), link("h", "x")]
        );
        assert_eq!(
            parse("<DL><DT><H3>open</H3><DL><DT><A HREF=\"h\">x"),
            [folder("open", vec![])]
        );
        assert_eq!(parse("plain text"), []);
        assert_eq!(
            parse("<DL><DL><DT><A HREF=\"h\">nested</A></DL></DL>"),
            [folder("", vec![link("h", "nested")])]
        );
    }
}
