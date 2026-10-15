//! The title of a web page, as a shortcut shows it.

use crate::scan::{Token, tokens};

/// The most characters a title keeps.
pub const LONGEST: usize = 200;

/// Text with runs of white space collapsed to one space and trimmed, cut to [`LONGEST`] characters.
fn tidy(text: &str) -> Option<String> {
    let mut tidied = String::new();
    for word in text.split_whitespace() {
        if !tidied.is_empty() {
            tidied.push(' ');
        }
        tidied.push_str(word);
    }
    let cut: String = tidied.chars().take(LONGEST).collect();
    if cut.is_empty() { None } else { Some(cut) }
}

/// The title a page declares: its Open Graph title when it has one, and its `<title>` element otherwise.
#[must_use]
pub fn title_of(html: &str) -> Option<String> {
    let scanned = tokens(html);
    let open_graph = scanned.iter().find_map(|token| match token {
        Token::Tag(tag)
            if tag.name == "meta"
                && !tag.closing
                && tag.attribute("property") == Some("og:title") =>
        {
            tag.attribute("content").and_then(tidy)
        }
        Token::Tag(_) | Token::Text(_) => None,
    });
    if open_graph.is_some() {
        return open_graph;
    }
    let mut inside = false;
    let mut text = String::new();
    for token in &scanned {
        match token {
            Token::Tag(tag) if tag.name == "title" => {
                if tag.closing {
                    if inside {
                        return tidy(&text);
                    }
                } else {
                    inside = true;
                }
            }
            Token::Text(piece) if inside => text.push_str(piece),
            Token::Tag(_) | Token::Text(_) => {}
        }
    }
    if inside { tidy(&text) } else { None }
}

#[cfg(test)]
mod tests {
    use super::{LONGEST, title_of};

    #[test]
    fn the_open_graph_title_wins_over_the_title_element() {
        let page = "<html><head><title>Repo · GitHub</title><meta property=\"og:title\" content=\"Repo &amp; Co\"></head></html>";
        assert_eq!(title_of(page).as_deref(), Some("Repo & Co"));
        assert_eq!(
            title_of("<TITLE>\n  Rust   docs \n</TITLE>").as_deref(),
            Some("Rust docs")
        );
        assert_eq!(title_of("<title>unclosed").as_deref(), Some("unclosed"));
    }

    #[test]
    fn a_page_without_a_title_has_none() {
        assert_eq!(title_of(""), None);
        assert_eq!(title_of("<p>body</p>"), None);
        assert_eq!(title_of("<title>   </title>"), None);
        assert_eq!(
            title_of("<meta property=\"og:title\" content=\"\"><title>t</title>").as_deref(),
            Some("t")
        );
        assert_eq!(
            title_of("</title>stray<title>x</title>").as_deref(),
            Some("x")
        );
    }

    #[test]
    fn a_long_title_is_cut() {
        let long = format!("<title>{}</title>", "語".repeat(LONGEST + 50));
        assert_eq!(
            title_of(&long).map(|title| title.chars().count()),
            Some(LONGEST)
        );
    }
}
