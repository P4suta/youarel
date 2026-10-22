//! JSON with comments and trailing commas, as VS Code writes its themes, made plain JSON.
//!
//! The scan keeps every string as it stands, drops `//` and `/* */` comments outside strings, and drops a comma that only a closing bracket or brace follows.

/// Where the scan stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Place {
    Code,
    String,
    /// Inside a string, right after a backslash.
    Escape,
    LineComment,
    BlockComment,
}

/// The text without its comments.
fn uncommented(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut place = Place::Code;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        place = match (place, character) {
            (Place::Code, '"') => {
                kept.push(character);
                Place::String
            }
            (Place::Code, '/') if characters.peek() == Some(&'/') => {
                characters.next();
                Place::LineComment
            }
            (Place::Code, '/') if characters.peek() == Some(&'*') => {
                characters.next();
                Place::BlockComment
            }
            (Place::Code, _) | (Place::LineComment, '\n') => {
                kept.push(character);
                Place::Code
            }
            (Place::String, '\\') => {
                kept.push(character);
                Place::Escape
            }
            (Place::String, '"') | (Place::Escape, _) => {
                kept.push(character);
                if place == Place::Escape {
                    Place::String
                } else {
                    Place::Code
                }
            }
            (Place::String, _) => {
                kept.push(character);
                Place::String
            }
            (Place::LineComment, _) => Place::LineComment,
            (Place::BlockComment, '*') if characters.peek() == Some(&'/') => {
                characters.next();
                kept.push(' ');
                Place::Code
            }
            (Place::BlockComment, _) => Place::BlockComment,
        };
    }
    kept
}

/// The text without a comma that only blanks and a closing bracket or brace follow.
fn untrailed(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (at, character) in text.char_indices() {
        if in_string {
            kept.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
        }
        let closes = character == ',' && {
            let rest = text.get(at.saturating_add(1)..).unwrap_or_default();
            rest.trim_start().starts_with(['}', ']'])
        };
        if !closes {
            kept.push(character);
        }
    }
    kept
}

/// The plain JSON a VS Code theme writes.
#[must_use]
pub fn strip(text: &str) -> String {
    untrailed(&uncommented(text))
}

#[cfg(test)]
mod tests {
    use super::strip;

    #[test]
    fn comments_and_trailing_commas_fall_away_and_strings_stay() {
        let text = "{\n  // the name\n  \"name\": \"a // b /* c */\", /* inline */\n  \"list\": [1, 2,],\n  \"quote\": \"\\\"x\\\",]\",\n}";
        let plain = strip(text);
        let value: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(value["name"], "a // b /* c */");
        assert_eq!(value["list"], serde_json::json!([1, 2]));
        assert_eq!(value["quote"], "\"x\",]");
        assert_eq!(strip("[1, /* a */ 2 /* , */]"), "[1,   2  ]");
        assert_eq!(strip("/* never closed"), "");
        assert_eq!(strip("[\"\\\\\", 3]"), "[\"\\\\\", 3]");
        assert_eq!(strip("1/2"), "1/2", "a lone slash starts no comment");
        assert_eq!(
            strip("/* a * b */1"),
            " 1",
            "a star inside a comment ends it only before a slash"
        );
    }
}
