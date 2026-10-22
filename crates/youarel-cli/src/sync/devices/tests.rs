use super::*;
use crate::sync::ACCOUNT_DIRECTORY;
use crate::sync::pairing::{Control, Pairing};
use youarel_core::device_rules::Role;
use youarel_store::{DATABASE_FILE, fs};

const NOW: u64 = 1_700_000_000_000;
const UNREADABLE: &[u8] = b"an unreadable library";
const CODE: &str = "4021-K7QM-2XRD";

#[derive(Clone, Copy, Debug)]
enum Action {
    Inspect,
    Invite,
    Join,
    Name,
    Remove,
    RecoveryKey,
    Recover,
}

fn act(action: Action, store: &Store, target: &str, key: &str) -> Result<()> {
    match action {
        Action::Inspect => inspect(store, NOW).map(drop),
        Action::Invite => Pairing::invite(
            store.clone(),
            target.to_owned(),
            Control::new(Role::Invite, NOW),
            NOW,
        )
        .map(drop),
        Action::Join => Pairing::join(
            store.clone(),
            (target.to_owned(), CODE),
            Control::new(Role::Join, NOW),
            NOW,
        )
        .map(drop),
        Action::Name => name(store, NOW, (MemberId([1; 32]), "Desk")),
        Action::Remove => remove(store, NOW, MemberId([1; 32])),
        Action::RecoveryKey => recovery_key(store, NOW).map(drop),
        Action::Recover => recover(store, NOW, (key, target)),
    }
}

#[test]
fn unreadable_libraries_refuse_every_interactive_device_action_before_side_effects() {
    let origin = tempfile::tempdir().unwrap();
    let valid = Store::new(origin.path().to_owned());
    crate::sync::create(&valid, NOW).unwrap();
    let key = recovery_key(&valid, NOW).unwrap();
    for action in [
        Action::Inspect,
        Action::Invite,
        Action::Join,
        Action::Name,
        Action::Remove,
        Action::RecoveryKey,
        Action::Recover,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_owned());
        let database = directory.path().join(DATABASE_FILE);
        fs::replace(&database, UNREADABLE).unwrap();
        let result = act(action, &store, shared.path().to_str().unwrap(), &key);
        assert!(
            !fs::exists(&directory.path().join(ACCOUNT_DIRECTORY)).unwrap(),
            "{action:?} created an identity before refusing the unreadable library"
        );
        assert!(fs::list(shared.path()).unwrap().is_empty(), "{action:?}");
        assert_eq!(crate::sync::read(&database).unwrap(), UNREADABLE);
        assert!(result.is_err(), "{action:?}: {result:?}");
    }
}
