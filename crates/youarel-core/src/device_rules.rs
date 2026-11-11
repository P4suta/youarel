//! Admission of device-name records retained by the persisted state format.

pub const LONGEST_NAME: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MemberId(pub [u8; 32]);

const fn admitted_name_length(bytes: usize) -> bool {
    bytes <= LONGEST_NAME
}

#[must_use]
pub fn admitted_name(name: &str) -> bool {
    admitted_name_length(name.len()) && name.trim() == name && !name.chars().any(char::is_control)
}

#[cfg(kani)]
mod proofs {
    #[kani::proof]
    #[kani::unwind(7)]
    fn short_name_admission_matches_edges_and_control_code_points() {
        const LAST_C0: char = '\u{1f}';
        const FIRST_C1: char = '\u{7f}';
        const LAST_C1: char = '\u{9f}';
        let bytes: [u8; 4] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= bytes.len());
        let Some(prefix) = bytes.get(..length) else {
            return;
        };
        let Ok(text) = core::str::from_utf8(prefix) else {
            return;
        };
        let edges = text
            .chars()
            .next()
            .is_none_or(|first| !first.is_whitespace())
            && text
                .chars()
                .next_back()
                .is_none_or(|last| !last.is_whitespace());
        let control = text
            .chars()
            .any(|character| character <= LAST_C0 || (FIRST_C1..=LAST_C1).contains(&character));
        let admitted = super::admitted_name(text);
        kani::assert(
            admitted == (edges && !control),
            "short names admit exactly unpadded text without C0 or C1 controls",
        );
        kani::cover!(admitted, "an admitted name");
        kani::cover!(control, "a control code point");
        kani::cover!(!edges, "a padded name");
    }

    #[kani::proof]
    #[kani::unwind(1)]
    fn name_length_admission_matches_the_byte_limit() {
        const EXPECTED_BYTE_LIMIT: usize = 128;
        let length: usize = kani::any();
        kani::assert(
            super::admitted_name_length(length) == (length <= EXPECTED_BYTE_LIMIT),
            "name length admission enforces the 128-byte contract",
        );
        kani::cover!(length == EXPECTED_BYTE_LIMIT, "the largest admitted length");
        kani::cover!(
            length == EXPECTED_BYTE_LIMIT.saturating_add(1),
            "the first excessive length"
        );
    }

    #[kani::proof]
    fn rejects_a_name_with_a_control_character() {
        kani::assert(super::admitted_name("a\nb"), "NEGATIVE_DEVICE_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn name_length_counts_utf8_bytes_instead_of_characters() {
        const CHARACTER: &str = "é";
        let count = super::LONGEST_NAME.checked_div(CHARACTER.len()).unwrap();
        let mut name = CHARACTER.repeat(count);
        assert_eq!(name.len(), super::LONGEST_NAME);
        assert!(super::admitted_name(&name));
        name.push_str(CHARACTER);
        assert!(!super::admitted_name(&name));
    }

    #[test]
    fn retained_names_admit_unicode_and_clear_but_refuse_padding_controls_and_oversize() {
        for name in ["", "Desktop", "仕事の Mac 🖥️"] {
            assert!(super::admitted_name(name));
        }
        for name in [" padded", "padded ", "two\nlines", "nul\0"] {
            assert!(!super::admitted_name(name));
        }
        assert!(!super::admitted_name(&"x".repeat(super::LONGEST_NAME + 1)));
        assert!(super::admitted_name(&"x".repeat(super::LONGEST_NAME)));
    }
}
