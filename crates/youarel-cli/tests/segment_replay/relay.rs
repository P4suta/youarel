use super::{Device, FIRST, NOW, added, pair};
use youarel_cli::{sync, web};
use youarel_core::record_rules::{Body, encode_body};
use youarel_core::segment_rules::{Header, Kind, Sequence};
use youarel_protocol::relay::Verb;
use youarel_protocol::segment::body_digest;
use youarel_relay::Status;

const LONGEST_ANSWER: u64 = 64 * 1024 * 1024;

#[test]
fn a_receiver_does_not_claim_sync_complete_while_a_snapshot_is_still_building() {
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    first.edit(NOW, added(FIRST));
    let local = youarel_relay::local::serve("127.0.0.1:0", || NOW).unwrap();
    let url = format!("http://127.0.0.1:{}", local.port());
    let endpoint = format!("{url}{}", youarel_relay::REQUESTS);
    let first_body = encode_body(&Body::of(&first.state())).unwrap();
    let last_body = encode_body(&Body::of(&youarel_core::state::State::default())).unwrap();
    let mut account = first.account();
    for (part, previous, body) in [
        (0, [0; 32], &first_body),
        (1, body_digest(&first_body), &last_body),
    ] {
        let sequence = Sequence::new(u64::from(part) + 1).unwrap();
        let header = Header::new(sequence, previous, Kind::Snapshot { part, parts: 2 }).unwrap();
        let bytes = account.publish(NOW, header, body).unwrap();
        let request = account.request(NOW, Verb::Publish, bytes).unwrap();
        assert_eq!(
            web::post(&endpoint, &request, LONGEST_ANSWER).unwrap().0,
            Status::Ok.code()
        );
        let read = sync::merge(&second.store, NOW, &url).unwrap();
        if part == 0 {
            assert!(read.more);
            read.checked_pending().unwrap_err();
            let waiting = sync::merge(&second.store, NOW, &url).unwrap();
            assert!(waiting.more);
            waiting.checked_pending().unwrap_err();
        } else {
            assert!(!read.more);
            assert!(read.checked_pending().unwrap());
        }
    }
    assert_eq!(first.state(), second.state());
}
