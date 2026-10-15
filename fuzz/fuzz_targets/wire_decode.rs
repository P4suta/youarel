//! The decoders of every sync message never stop the reader, and whatever one reads it writes back byte for byte, so each message has one encoding.
#![no_main]

use youarel_protocol::message::{
    Admission, Bundle, DevicePublic, Entry, Join, Joined, Operation, Welcome,
};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(operation) = Operation::decode(data) {
        assert_eq!(
            operation.encode().as_deref(),
            Ok(data),
            "an operation has one encoding"
        );
    }
    if let Ok(bundle) = Bundle::decode(data) {
        assert_eq!(
            bundle.encode().as_deref(),
            Ok(data),
            "a bundle has one encoding"
        );
    }
    if let Ok(join) = Join::decode(data) {
        assert_eq!(
            join.encode().as_deref(),
            Ok(data),
            "a join has one encoding"
        );
    }
    if let Ok(welcome) = Welcome::decode(data) {
        assert_eq!(
            welcome.encode().as_deref(),
            Ok(data),
            "a welcome has one encoding"
        );
    }
    if let Ok(joined) = Joined::decode(data) {
        assert_eq!(joined.encode(), data, "a confirmation has one encoding");
    }
    if let Ok(entry) = Entry::decode(data) {
        assert_eq!(
            entry.encode().as_deref(),
            Ok(data),
            "an entry has one encoding"
        );
    }
    if let Ok(admission) = Admission::decode(data) {
        assert_eq!(
            admission.encode().as_deref(),
            Ok(data),
            "an admission has one encoding"
        );
    }
    if let Ok(public) = DevicePublic::decode(data) {
        assert_eq!(public.encode(), data, "public keys have one encoding");
    }
});
