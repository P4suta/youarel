//! The decoders of every sync and relay message never stop the reader, and whatever one reads it writes back byte for byte, so each message has one encoding.
#![no_main]

use youarel_protocol::message::{Admission, DevicePublic, Entry, Join, Joined, Operation, Welcome};
use youarel_protocol::relay::{
    Locate, Mail, Registration, Request, located_bytes, nameplate_bytes, read_located,
    read_nameplate,
};
use youarel_protocol::segment::{Cursors, Delivered, Head, Segment};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(segment) = Segment::decode(data) {
        assert_eq!(
            segment.encode().as_deref(),
            Ok(data),
            "a segment has one encoding"
        );
    }
    if let Ok(cursors) = Cursors::decode(data) {
        assert_eq!(
            cursors.encode().as_deref(),
            Ok(data),
            "cursors have one encoding"
        );
    }
    if let Ok(delivered) = Delivered::decode(data) {
        assert_eq!(
            delivered.encode().as_deref(),
            Ok(data),
            "a delivery has one encoding"
        );
    }
    if let Ok(head) = Head::decode(data) {
        assert_eq!(head.encode(), data, "a head has one encoding");
    }
    if let Ok(operation) = Operation::decode(data) {
        assert_eq!(
            operation.encode().as_deref(),
            Ok(data),
            "an operation has one encoding"
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
    if let Ok(request) = Request::decode(data) {
        assert_eq!(
            request.encode().as_deref(),
            Ok(data),
            "a relay request has one encoding"
        );
    }
    if let Ok(mail) = Mail::decode(data) {
        assert_eq!(
            mail.encode().as_deref(),
            Ok(data),
            "a piece of mail has one encoding"
        );
    }
    if let Ok(locate) = Locate::decode(data) {
        assert_eq!(locate.encode(), data, "a location has one encoding");
    }
    if let Ok(registration) = Registration::decode(data) {
        assert_eq!(
            registration.encode().as_deref(),
            Ok(data),
            "a registration has one encoding"
        );
    }
    if let Ok(account) = read_located(data) {
        assert_eq!(located_bytes(&account), data, "an answer has one encoding");
    }
    if let Ok(nameplate) = read_nameplate(data) {
        assert_eq!(
            nameplate_bytes(nameplate),
            data,
            "a nameplate has one encoding"
        );
    }
});
