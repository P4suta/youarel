//! A tolerant scan of HTML into tags and text, which reads what browsers write without building a document tree.

/// The radix of a numeric reference that starts with `x`.
const HEX_RADIX: u32 = 16;
/// How far past its `&` the scan looks for the `;` that ends a reference, which every reference it knows fits.
const REFERENCE_REACH: usize = 12;
/// What ends a comment.
const COMMENT_END: &str = "-->";

/// One tag: its lowercase name, whether it closes, and its attributes with lowercase names and decoded values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub name: String,
    pub closing: bool,
    pub attributes: Vec<(String, String)>,
}

impl Tag {
    /// The decoded value of an attribute, by its lowercase name.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A piece of the scanned text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Token {
    Tag(Tag),
    /// Text between tags, with its character references decoded.
    Text(String),
}

/// The character a named or numeric reference stands for.
fn reference(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let number = name.strip_prefix('#')?;
            let value = number.strip_prefix(['x', 'X']).map_or_else(
                || number.parse::<u32>(),
                |hex| u32::from_str_radix(hex, HEX_RADIX),
            );
            value
                .into_iter()
                .find_map(|code| char::from_u32(code).filter(|character| *character != '\0'))
        }
    }
}

/// Text with its character references decoded; a reference that is not one stays as written.
#[must_use]
pub fn decode(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        let (before, from) = rest.split_at(start);
        decoded.push_str(before);
        let name_end = from
            .char_indices()
            .skip(1)
            .take(REFERENCE_REACH)
            .find(|(_, character)| *character == ';')
            .map(|(index, _)| index);
        let replaced = name_end.and_then(|end| {
            let name = from.get(1..end)?;
            Some((reference(name)?, end))
        });
        if let Some((character, end)) = replaced {
            decoded.push(character);
            rest = from.get(end.saturating_add(1)..).unwrap_or_default();
        } else {
            decoded.push('&');
            rest = from.get(1..).unwrap_or_default();
        }
    }
    decoded.push_str(rest);
    decoded
}

/// Text with the characters that would end it escaped, for an attribute or an element body.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// An attribute's value after its equals sign, quoted or bare, and the text after it.
fn attribute_value(after_equals: &str) -> (&str, &str) {
    let start = after_equals.trim_start();
    if let Some(quote) = start
        .chars()
        .next()
        .filter(|first| *first == '"' || *first == '\'')
    {
        let body = start.get(1..).unwrap_or_default();
        let end = body.find(quote).unwrap_or(body.len());
        let (value, tail) = body.split_at(end);
        return (value, tail.get(1..).unwrap_or_default());
    }
    let end = start.find(char::is_whitespace).unwrap_or(start.len());
    start.split_at(end)
}

/// The attributes of a tag's inside, after its name.
fn attributes(inside: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = inside.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|character: char| {
                character.is_whitespace() || character == '=' || character == '/'
            })
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(name_end);
        let after = after.trim_start();
        let (value, remaining) = after.strip_prefix('=').map_or(("", after), attribute_value);
        if name.is_empty() {
            rest = remaining.get(1..).unwrap_or_default().trim_start();
            continue;
        }
        found.push((name.to_ascii_lowercase(), decode(value)));
        rest = remaining.trim_start();
    }
    found
}

/// The tag a `<...>` inside spells, or `None` for a comment, a declaration, or a stray angle bracket.
fn tag(inside: &str) -> Option<Tag> {
    let (closing, body) = inside
        .strip_prefix('/')
        .map_or((false, inside), |body| (true, body));
    let name_end = body
        .find(|character: char| character.is_whitespace() || character == '/')
        .unwrap_or(body.len());
    let (name, rest) = body.split_at(name_end);
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(Tag {
        name: name.to_ascii_lowercase(),
        closing,
        attributes: attributes(rest),
    })
}

/// The tokens of an HTML text, in order; nothing the scan does not understand stops it.
#[must_use]
pub fn tokens(html: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            found.push(Token::Text(decode(rest)));
            break;
        };
        let (text, from) = rest.split_at(open);
        if !text.is_empty() {
            found.push(Token::Text(decode(text)));
        }
        if let Some(comment) = from.strip_prefix("<!--") {
            let end = comment
                .find(COMMENT_END)
                .map_or(comment.len(), |end| end.saturating_add(COMMENT_END.len()));
            rest = comment.get(end..).unwrap_or_default();
            continue;
        }
        let Some(close) = from.find('>') else {
            found.push(Token::Text(decode(from)));
            break;
        };
        let inside = from.get(1..close).unwrap_or_default();
        if let Some(parsed) = tag(inside) {
            found.push(Token::Tag(parsed));
        }
        rest = from.get(close.saturating_add(1)..).unwrap_or_default();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{Tag, Token, decode, escape, tokens};

    #[test]
    fn references_decode_and_unknown_ones_stay() {
        assert_eq!(
            decode("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#x27;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(
            decode("&unknown; & &#0; &#xZZ; &"),
            "&unknown; & &#0; &#xZZ; &"
        );
        assert_eq!(decode("日本&#26085;"), "日本日");
        assert_eq!(decode(&escape("<\"&>")), "<\"&>");
    }

    #[test]
    fn tags_and_text_come_out_in_order() {
        let found = tokens(
            "<DT><A HREF=\"https://a.example/?x=1&amp;y=2\" ADD_DATE=1>A &amp; B</A><!-- skip --></dl>",
        );
        assert_eq!(
            found,
            [
                Token::Tag(Tag {
                    name: "dt".to_owned(),
                    closing: false,
                    attributes: Vec::new()
                }),
                Token::Tag(Tag {
                    name: "a".to_owned(),
                    closing: false,
                    attributes: vec![
                        ("href".to_owned(), "https://a.example/?x=1&y=2".to_owned()),
                        ("add_date".to_owned(), "1".to_owned())
                    ]
                }),
                Token::Text("A & B".to_owned()),
                Token::Tag(Tag {
                    name: "a".to_owned(),
                    closing: true,
                    attributes: Vec::new()
                }),
                Token::Tag(Tag {
                    name: "dl".to_owned(),
                    closing: true,
                    attributes: Vec::new()
                }),
            ]
        );
    }

    #[test]
    fn broken_markup_never_stops_the_scan() {
        assert_eq!(
            tokens("a < b"),
            [Token::Text("a ".to_owned()), Token::Text("< b".to_owned())]
        );
        assert_eq!(tokens("<!-- open"), []);
        assert_eq!(
            tokens("<!DOCTYPE x><p single='q' bare=v flag>t"),
            [
                Token::Tag(Tag {
                    name: "p".to_owned(),
                    closing: false,
                    attributes: vec![
                        ("single".to_owned(), "q".to_owned()),
                        ("bare".to_owned(), "v".to_owned()),
                        ("flag".to_owned(), String::new())
                    ]
                }),
                Token::Text("t".to_owned())
            ]
        );
        let tag = Tag {
            name: "a".to_owned(),
            closing: false,
            attributes: vec![("href".to_owned(), "x".to_owned())],
        };
        assert_eq!(tag.attribute("href"), Some("x"));
        assert_eq!(tag.attribute("src"), None);
    }
}
