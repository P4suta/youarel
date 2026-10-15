//! A recovery key brings a new device into its account after every other device is gone, and nothing else does.
#![cfg(test)]

use youarel_crypto::recovery::RecoveryKey;
use youarel_sync::SyncError;
use youarel_sync::account::Account;
use youarel_sync::keystore::KeyStore;

const NOW: u64 = 1_700_000_000_000;

struct Device {
    directory: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn account(&self) -> Account {
        Account::open(
            &self.directory.path().join("account"),
            &KeyStore::File(self.directory.path().join("sealing.key")),
        )
        .unwrap()
    }
}

fn refused(result: Result<impl core::fmt::Debug, SyncError>, reason: &str) {
    let error = result.unwrap_err();
    assert!(error.to_string().contains(reason), "{error}");
}

#[test]
fn a_recovery_key_brings_a_new_device_in_after_the_others_are_gone() {
    let (lost, replacement) = (Device::new(), Device::new());
    let mut account = lost.account();
    account.found(NOW).unwrap();
    let key = account.enroll_recovery(NOW).unwrap();
    let recovery = Account::recovery_member(&key).unwrap();
    let status = account.status().unwrap();
    assert!(status.members.contains(&recovery));
    assert_eq!(account.recovery_members().unwrap(), [recovery]);
    assert_eq!(
        status.epoch, 2,
        "the enrolment rotates so that the key's member holds a slot"
    );
    let bundle = account.export(NOW, b"the state").unwrap();
    let typed = RecoveryKey::parse(&key.text()).unwrap();
    let mut fresh = replacement.account();
    let imported = fresh.recover(NOW, &typed, &bundle).unwrap();
    assert_eq!(imported.state.as_slice(), b"the state");
    let joined = fresh.status().unwrap();
    assert!(joined.member);
    assert_eq!(joined.members.len(), 3);
    let next = fresh.export(NOW, b"newer").unwrap();
    assert_eq!(fresh.status().unwrap().epoch, 3);
    assert_eq!(account.import(&next).unwrap().state.as_slice(), b"newer");
    let lost_id = account.device();
    fresh.remove(NOW, lost_id).unwrap();
    assert!(!fresh.status().unwrap().members.contains(&lost_id));
}

#[test]
fn a_wrong_or_revoked_key_recovers_nothing_and_writes_nothing() {
    let (first, stranger) = (Device::new(), Device::new());
    let mut account = first.account();
    account.found(NOW).unwrap();
    let key = account.enroll_recovery(NOW).unwrap();
    let bundle = account.export(NOW, b"s").unwrap();
    let other = RecoveryKey::generate().unwrap();
    refused(
        stranger.account().recover(NOW, &other, &bundle),
        "no member",
    );
    assert!(!stranger.account().has_account());
    let recovery = Account::recovery_member(&key).unwrap();
    account.remove(NOW, recovery).unwrap();
    let after = account.export(NOW, b"s").unwrap();
    refused(stranger.account().recover(NOW, &key, &after), "no member");
    assert!(!stranger.account().has_account());
    refused(first.account().recover(NOW, &key, &bundle), "already");
    refused(stranger.account().enroll_recovery(NOW), "no account");
}
