use super::{Control, Pairing};
use youarel_core::{
    device_rules::{Event, MemberId, PAIRING_PATIENCE, Role, Stage},
    state::{Edit, ItemId},
};
use youarel_store::{Store, fs};

const NOW: u64 = 1_700_000_000_000;
const LINK: &str = "https://pairing.example/";
const LOOPBACK: &str = "127.0.0.1:0";
const LOCAL_RELAY: &str = "http://127.0.0.1";

#[test]
fn a_library_that_becomes_unreadable_does_not_consume_a_waiting_join() {
    const UNREADABLE: &[u8] = b"an unreadable library";
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let target = shared.path().to_str().unwrap();
    let source = Store::new(left.path().to_owned());
    let peer = Store::new(right.path().to_owned());
    let (mut first, code) = Pairing::invite(
        source.clone(),
        target.to_owned(),
        Control::new(Role::Invite, NOW),
        NOW,
    )
    .unwrap();
    let mut second = Pairing::join(
        peer,
        (target.to_owned(), &code),
        Control::new(Role::Join, NOW),
        NOW,
    )
    .unwrap();
    second.advance(NOW).unwrap();
    let members = super::account(&source).unwrap().members().unwrap();
    first.store = Store::new(left.path().to_owned());
    drop(source);
    fs::replace(&left.path().join(youarel_store::DATABASE_FILE), UNREADABLE).unwrap();
    first.advance(NOW).unwrap_err();
    assert!(first.transport.take(super::Packet::Join).unwrap().is_some());
    assert_eq!(
        super::account(&first.store).unwrap().members().unwrap(),
        members
    );
    first.cleanup().unwrap();
    second.cleanup().unwrap();
}

#[test]
fn bounded_pairing_uses_immediate_relay_mail_and_publishes_membership_before_admission_delivery() {
    let local = youarel_relay::local::serve(LOOPBACK, || NOW).unwrap();
    let target = format!("{LOCAL_RELAY}:{}", local.port());
    let first_home = tempfile::tempdir().unwrap();
    let second_home = tempfile::tempdir().unwrap();
    let source = Store::new(first_home.path().to_owned());
    let peer = Store::new(second_home.path().to_owned());
    let invite = Control::new(Role::Invite, NOW);
    let join = Control::new(Role::Join, NOW);
    let (mut first, code) =
        Pairing::invite(source.clone(), target.clone(), invite.clone(), NOW).unwrap();
    first.advance(NOW).unwrap();
    assert_eq!(invite.stage(), Stage::Waiting);
    let mut second = Pairing::join(peer.clone(), (target, &code), join.clone(), NOW).unwrap();
    for tick in 0..16 {
        first.advance(NOW + tick).unwrap();
        second.advance(NOW + tick).unwrap();
        if invite.stage().terminal() && join.stage().terminal() {
            break;
        }
    }
    assert_eq!((invite.stage(), join.stage()), (Stage::Done, Stage::Done));
    assert_eq!(
        super::account(&source).unwrap().status().unwrap().members,
        super::account(&peer).unwrap().status().unwrap().members
    );
    first.cleanup().unwrap();
    second.cleanup().unwrap();
}

#[test]
fn a_different_secret_under_the_same_number_adds_no_device() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let target = shared.path().to_str().unwrap();
    let source = Store::new(left.path().to_owned());
    let peer = Store::new(right.path().to_owned());
    let invite = Control::new(Role::Invite, NOW);
    let join = Control::new(Role::Join, NOW);
    let (mut first, code) =
        Pairing::invite(source.clone(), target.to_owned(), invite.clone(), NOW).unwrap();
    let parsed = youarel_crypto::pairing::Code::parse(&code).unwrap();
    let wrong = youarel_crypto::pairing::Code::for_number(parsed.number).unwrap();
    let mut second = Pairing::join(
        peer.clone(),
        (target.to_owned(), &wrong.text()),
        join.clone(),
        NOW,
    )
    .unwrap();
    second.advance(NOW).unwrap();
    first.advance(NOW).unwrap();
    second.advance(NOW).unwrap_err();
    assert_eq!(join.stage(), Stage::Refused);
    assert_eq!(
        super::account(&source)
            .unwrap()
            .status()
            .unwrap()
            .members
            .len(),
        1
    );
    assert!(!super::account(&peer).unwrap().has_account());
    invite.event(Event::Cancel, NOW);
    first.cleanup().unwrap();
    second.cleanup().unwrap();
}

#[test]
fn fresh_stores_pair_in_bounded_steps_and_sync_names_and_records_through_a_folder() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let target = shared.path().to_str().unwrap();
    let source = Store::new(left.path().to_owned());
    let peer = Store::new(right.path().to_owned());
    let invite = Control::new(Role::Invite, NOW);
    let joining = Control::new(Role::Join, NOW);
    let (mut first, code) =
        Pairing::invite(source.clone(), target.to_owned(), invite.clone(), NOW).unwrap();
    let id = MemberId(super::account(&source).unwrap().device());
    source
        .session(NOW, |session| {
            session.edit(Edit::NameDevice {
                id,
                name: "Mac".to_owned(),
            })?;
            session.edit(Edit::AddShortcut {
                id: ItemId([1; 16]),
                link: LINK.to_owned(),
                title: "Paired".to_owned(),
                folder: None,
            })
        })
        .unwrap();
    let mut second = Pairing::join(
        peer.clone(),
        (target.to_owned(), &code),
        joining.clone(),
        NOW,
    )
    .unwrap();
    for tick in 0..16 {
        first.advance(NOW + tick).unwrap();
        second.advance(NOW + tick).unwrap();
        if invite.stage().terminal() && joining.stage().terminal() {
            break;
        }
    }
    assert_eq!(
        (invite.stage(), joining.stage()),
        (Stage::Done, Stage::Done)
    );
    let source_status = super::account(&source).unwrap().status().unwrap();
    let peer_status = super::account(&peer).unwrap().status().unwrap();
    assert_eq!(source_status.account, peer_status.account);
    assert_eq!(source_status.members, peer_status.members);
    assert_eq!(source_status.members.len(), 2);
    assert_eq!(source_status.epoch, peer_status.epoch);
    assert_eq!(
        source.session(NOW, |session| session.records()).unwrap(),
        peer.session(NOW, |session| session.records()).unwrap()
    );
    first.cleanup().unwrap();
    second.cleanup().unwrap();
    assert_eq!(
        fs::list(&shared.path().join("pairing")).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn a_cancelled_or_expired_invitation_never_admits_an_awaited_joiner() {
    let directory = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let control = Control::new(Role::Invite, NOW);
    let (mut pairing, _code) = Pairing::invite(
        store.clone(),
        shared.path().to_str().unwrap().to_owned(),
        control.clone(),
        NOW,
    )
    .unwrap();
    control.event(Event::Cancel, NOW);
    assert_eq!(pairing.advance(NOW).unwrap(), Stage::Cancelled);
    let mut ran = false;
    assert_eq!(
        control
            .local((Stage::Waiting, Event::Peer), NOW, || {
                ran = true;
                Ok(())
            })
            .unwrap(),
        None
    );
    assert!(!ran);
    pairing.cleanup().unwrap();
    let expiry = Control::new(Role::Invite, NOW);
    let (mut expired, _expired_code) = Pairing::invite(
        store.clone(),
        shared.path().to_str().unwrap().to_owned(),
        expiry,
        NOW,
    )
    .unwrap();
    assert_eq!(
        expired.advance(NOW + PAIRING_PATIENCE).unwrap(),
        Stage::Expired
    );
    expired.cleanup().unwrap();
    assert_eq!(
        super::account(&store)
            .unwrap()
            .status()
            .unwrap()
            .members
            .len(),
        1
    );
}
