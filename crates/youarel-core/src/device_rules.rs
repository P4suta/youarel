//! Admission of device-name records retained by the persisted state format.

pub const LONGEST_NAME: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MemberId(pub [u8; 32]);

#[must_use]
pub fn admitted_name(name: &str) -> bool {
    name.len() <= LONGEST_NAME && name.trim() == name && !name.chars().any(char::is_control)
}

#[cfg(kani)]
mod checks {
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(129)]
    fn admitted_short_names_round_trip_through_the_production_codec() {
        use crate::{
            clock_rules::{DeviceId, Stamp},
            merge_rules::Register,
            record_rules::{DeviceTable, decode_device, encode_device},
            wire_rules::{Reader, Writer},
        };
        let bytes: [u8; 4] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= bytes.len());
        let Some(prefix) = bytes.get(..length) else {
            return;
        };
        let Ok(text) = core::str::from_utf8(prefix) else {
            return;
        };
        let admitted = super::admitted_name(text);
        let writer = DeviceId([3; 16]);
        let name = Register {
            value: alloc::string::String::from(text),
            stamp: Stamp {
                wall: kani::any(),
                counter: kani::any(),
                device: writer,
            },
        };
        let Some(table) = DeviceTable::ascending(alloc::vec![writer]) else {
            return;
        };
        let id = super::MemberId(kani::any());
        let mut encoded = Writer::new();
        let result = encode_device(&mut encoded, &table, (id, &name));
        kani::assert(
            result.is_ok() == admitted,
            "the writer enforces the name contract",
        );
        if admitted {
            let bytes = encoded.finish();
            let mut reader = Reader::new(&bytes);
            kani::assert(
                decode_device(&mut reader, &table) == Ok((id, name)),
                "an admitted name and every stamp bit read back",
            );
            kani::assert(
                reader.finish().is_ok(),
                "the codec consumes exactly its record",
            );
        }
        kani::cover!(admitted, "an admitted name");
        kani::cover!(!admitted, "a refused name");
    }

    #[kani::proof]
    fn rejects_a_name_with_a_control_character() {
        kani::assert(super::admitted_name("a\nb"), "NEGATIVE_DEVICE_CONTROL");
    }
}

#[cfg(test)]
mod tests {
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
