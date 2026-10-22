//! What a folder may show in place of its glyph: a mark, such as one emoji, short enough to sit where an icon sits.
//!
//! A mark is the person's text with the whitespace around it trimmed away.
//! An empty mark clears the folder's, and a mark with whitespace or a control character inside, or past the longest, is refused.

/// The most characters a mark holds: room for an emoji joined from several, such as a family's.
pub const MARK_CHARS: usize = 8;
/// The most bytes a mark holds.
pub const MARK_BYTES: usize = 32;

/// Why a mark is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MarkError {
    /// The mark holds more than [`MARK_CHARS`] characters or [`MARK_BYTES`] bytes.
    Long,
    /// The mark holds whitespace or a control character inside it.
    Spaced,
}

impl core::fmt::Display for MarkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Long => write!(
                formatter,
                "a mark holds at most {MARK_CHARS} characters, such as one emoji"
            ),
            Self::Spaced => formatter.write_str("a mark holds no space or control character"),
        }
    }
}

/// The mark `text` asks for, trimmed, or why it is refused; an empty mark clears the folder's.
///
/// # Errors
///
/// Returns [`MarkError::Long`] for a mark past the longest and [`MarkError::Spaced`] for one with whitespace or a control character inside.
pub fn admit(text: &str) -> Result<&str, MarkError> {
    let mark = text.trim();
    // Seeking a ninth character takes one step per character, where `count` takes one per byte, which keeps the Kani proof's unwinding short.
    if mark.len() > MARK_BYTES || mark.chars().nth(MARK_CHARS).is_some() {
        return Err(MarkError::Long);
    }
    if mark
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(MarkError::Spaced);
    }
    Ok(mark)
}

/// The properties Kani proves over every short text.
#[cfg(any(test, kani))]
mod proofs {
    use super::{MARK_BYTES, MARK_CHARS, MarkError, admit};

    /// What a mark promises, restated: an admitted mark is the trimmed text, holds no whitespace or control character, and fits; a refusal names what the trimmed text breaks.
    pub(super) fn spec(text: &str) -> bool {
        let trimmed = text.trim();
        let spaced = trimmed
            .chars()
            .any(|character| character.is_whitespace() || character.is_control());
        let long = trimmed.len() > MARK_BYTES || trimmed.chars().nth(MARK_CHARS).is_some();
        match admit(text) {
            Ok(mark) => core::ptr::eq(mark, trimmed) && !spaced && !long,
            Err(MarkError::Long) => long,
            Err(MarkError::Spaced) => spaced && !long,
        }
    }

    /// The characters a drawn text joins: nothing, an ASCII space, a tab, a letter, a letter beyond ASCII, an emoji, a joiner, and a wide space.
    pub(super) const PIECES: [&str; 8] = ["", " ", "\t", "a", "é", "📚", "\u{200d}", "\u{3000}"];

    /// How many pieces a drawn text joins.
    #[cfg(kani)]
    const DRAWN: usize = 3;

    /// Every text of three pieces, which holds every kind of character a mark meets, at its start, its middle, and its end.
    ///
    /// Drawing the characters from pieces keeps Unicode's tables of spaces to the characters drawn, which the solver could not span for every byte.
    /// The pieces join in a buffer of a fixed size, which the solver keeps as a few bytes, where a buffer as long as the text would leave it an array of any length.
    /// Every loop takes at most one turn per character, and three pieces hold three characters at most, so four unwindings end them all.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(4)]
    fn a_mark_is_the_trimmed_text_without_space_inside() {
        let mut bytes = [0; DRAWN * char::MAX_LEN_UTF8];
        let mut length = 0;
        for _ in 0..DRAWN {
            let choice: usize = kani::any();
            kani::assume(choice < PIECES.len());
            let piece = PIECES.get(choice).copied().unwrap_or_default().as_bytes();
            let end = length + piece.len();
            if let Some(slot) = bytes.get_mut(length..end) {
                slot.copy_from_slice(piece);
            }
            length = end;
        }
        kani::assert(length <= bytes.len(), "the pieces fit the buffer");
        let joined = core::str::from_utf8(bytes.get(..length).unwrap_or_default());
        kani::assert(joined.is_ok(), "the pieces join into a text");
        let Ok(text) = joined else {
            return;
        };
        kani::assert(spec(text), "the mark keeps its promise");
        let admitted = admit(text);
        kani::cover!(
            admitted.is_ok_and(|mark| mark.chars().any(|character| !character.is_ascii())),
            "a mark beyond ASCII"
        );
        kani::cover!(
            admitted == Err(MarkError::Spaced),
            "a mark with a space inside"
        );
        kani::cover!(
            admitted == Ok("") && !text.is_empty(),
            "a mark of spaces clears"
        );
    }

    /// A tab inside a mark refuses it; this probe claims such a mark is admitted and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_mark_with_a_tab_inside() {
        kani::assert(admit("a\tb").is_ok(), "NEGATIVE_MARK_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{PIECES, spec};
    use super::{MARK_BYTES, MARK_CHARS, MarkError, admit};
    use alloc::string::String;

    /// A family of four, one emoji that seven characters join, written with its joiners escaped.
    const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";

    #[test]
    fn a_mark_is_trimmed_and_refused_when_spaced_or_long() {
        assert_eq!(admit(" 📚 "), Ok("📚"));
        assert_eq!(admit(FAMILY), Ok(FAMILY), "a joined emoji fits");
        assert_eq!(admit("🇯🇵"), Ok("🇯🇵"));
        assert_eq!(admit("   "), Ok(""), "only spaces clear the mark");
        assert_eq!(admit("a b"), Err(MarkError::Spaced));
        assert_eq!(admit("a\u{7}"), Err(MarkError::Spaced));
        assert_eq!(admit("nine char"), Err(MarkError::Long));
        let wide: String = "🧭".repeat(MARK_BYTES);
        assert_eq!(admit(&wide), Err(MarkError::Long));
    }

    #[test]
    fn a_mark_holds_eight_characters_and_refuses_a_ninth() {
        let eight = "a".repeat(MARK_CHARS);
        assert_eq!(admit(&eight), Ok(eight.as_str()));
        let nine = "a".repeat(MARK_CHARS + 1);
        assert_eq!(admit(&nine), Err(MarkError::Long));
        let widest = "🧭".repeat(MARK_CHARS);
        assert_eq!(
            widest.len(),
            MARK_BYTES,
            "eight emoji of four bytes fill a mark"
        );
        assert_eq!(admit(&widest), Ok(widest.as_str()));
    }

    #[test]
    fn the_specification_agrees_over_drawn_texts() {
        for first in PIECES {
            for second in PIECES {
                for third in PIECES {
                    let text = [first, second, third].concat();
                    assert!(spec(&text), "{text:?}");
                }
            }
        }
        assert!(spec(&"x".repeat(MARK_BYTES)));
    }
}
