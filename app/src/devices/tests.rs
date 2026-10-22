use super::{Devices, Phase, Request, Update, inspect};
use youarel_cli::sync;
use youarel_core::device_rules::MemberId;
use youarel_store::Store;

const NOW: u64 = 1_700_000_000_000;
const KEPT_LINK: &str = "https://recovery.example/kept";

#[test]
fn qr_follows_only_the_current_code_and_disappears_when_the_invitation_ends() {
    use youarel_core::device_rules::{Event, PAIRING_PATIENCE};
    const CODE: &str = "4021-K7QM-2XRD";
    for cancelled in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_owned());
        let mut devices = Devices::default();
        let job = devices
            .begin(
                store.clone(),
                Request::Invite {
                    target: shared.path().to_str().unwrap().to_owned(),
                },
                NOW,
            )
            .unwrap()
            .unwrap();
        let update = |token, code: &str| Update {
            token,
            phase: Phase::Waiting,
            code: Some(code.to_owned()),
        };
        devices.update(update(job.token, CODE));
        let shown = devices.progress();
        assert_eq!(shown.qr, Some(super::CodeImage::encode(CODE).unwrap()));
        devices.update(update(job.token + 1, "stale"));
        assert_eq!(devices.progress(), shown);
        if cancelled {
            devices.begin(store, Request::CancelPairing, NOW).unwrap();
        } else {
            devices
                .control
                .as_ref()
                .unwrap()
                .event(Event::Poll, NOW + PAIRING_PATIENCE);
        }
        devices.update(update(job.token, CODE));
        assert_eq!(devices.progress().code, None);
        assert_eq!(devices.progress().qr, None);
        devices.finish(job.token, Err(anyhow::anyhow!("stopped")));
        assert_eq!(devices.progress().qr, None);
        devices.update(update(job.token, "late"));
        assert_eq!(devices.progress().qr, None);
    }
}

#[test]
fn completed_invitations_and_new_runs_never_reuse_an_old_qr() {
    let directory = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let request = || Request::Invite {
        target: shared.path().to_str().unwrap().to_owned(),
    };
    let mut devices = Devices::default();
    let job = devices
        .begin(store.clone(), request(), NOW)
        .unwrap()
        .unwrap();
    devices.update(Update {
        token: job.token,
        phase: Phase::Waiting,
        code: Some("4021-K7QM-2XRD".to_owned()),
    });
    assert!(devices.progress().qr.is_some());
    devices.finish(job.token, Ok(super::Outcome::default()));
    assert_eq!(devices.progress().code, None);
    assert_eq!(devices.progress().qr, None);
    let next = devices.begin(store, request(), NOW).unwrap().unwrap();
    assert!(next.token > job.token);
    devices.update(Update {
        token: job.token,
        phase: Phase::Waiting,
        code: Some("late".to_owned()),
    });
    assert_eq!(devices.progress().code, None);
    assert_eq!(devices.progress().qr, None);
}

fn run_device(store: &Store, request: Request) {
    let mut controller = Devices::default();
    let mut job = controller
        .begin(store.clone(), request, NOW)
        .unwrap()
        .unwrap();
    let result = job.run(|update| controller.update(update));
    controller.finish(job.token, result);
    assert_eq!(
        controller.progress().phase,
        Phase::Done,
        "{:?}",
        controller.progress().error
    );
}

fn pair_stores(
    source: &Store,
    peer: &Store,
    target: &str,
) -> (sync::pairing::Pairing, sync::pairing::Pairing) {
    use youarel_cli::sync::pairing::{Control, Pairing};
    use youarel_core::device_rules::{Role, Stage};
    let invited = Control::new(Role::Invite, NOW);
    let joined = Control::new(Role::Join, NOW);
    let (mut invitation, code) =
        Pairing::invite(source.clone(), target.to_owned(), invited.clone(), NOW).unwrap();
    let mut newcomer = Pairing::join(
        peer.clone(),
        (target.to_owned(), &code),
        joined.clone(),
        NOW,
    )
    .unwrap();
    for tick in 0..16 {
        let now = NOW.checked_add(tick).unwrap();
        invitation.advance(now).unwrap();
        newcomer.advance(now).unwrap();
        if invited.stage().terminal() && joined.stage().terminal() {
            break;
        }
    }
    assert_eq!(
        (invited.stage(), joined.stage()),
        (Stage::Done, Stage::Done)
    );
    (invitation, newcomer)
}

#[test]
fn two_locked_libraries_pair_exchange_names_and_remove_a_device_without_terminal_commands() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let target = shared.path().to_str().unwrap();
    let source = Store::new(left.path().to_owned());
    let peer = Store::new(right.path().to_owned());
    source
        .session(NOW, |session| {
            session.edit(youarel_core::state::Edit::AddShortcut {
                id: youarel_core::state::ItemId([1; 16]),
                link: KEPT_LINK.to_owned(),
                title: "Kept".to_owned(),
                folder: None,
            })
        })
        .unwrap();
    let source_library = std::sync::Mutex::new(crate::library::Library::new(source.clone()));
    let peer_library = std::sync::Mutex::new(crate::library::Library::new(peer.clone()));
    let mut source_window = source_library.lock().unwrap();
    let mut peer_window = peer_library.lock().unwrap();
    let (invitation, newcomer) = pair_stores(&source, &peer, target);
    let own_id = inspect(&source).unwrap().this_device;
    let peer_id = inspect(&peer).unwrap().this_device;
    run_device(
        &source,
        Request::NameDevice {
            id: own_id,
            name: "Mac".to_owned(),
        },
    );
    run_device(
        &peer,
        Request::NameDevice {
            id: peer_id.clone(),
            name: "Windows".to_owned(),
        },
    );
    sync::sync_checked(&peer, NOW, target).unwrap();
    sync::sync_checked(&source, NOW, target).unwrap();
    assert!(
        inspect(&source)
            .unwrap()
            .devices
            .iter()
            .any(|device| device.name == "Windows")
    );
    run_device(
        &source,
        Request::RemoveDevice {
            id: peer_id.clone(),
            confirmed: true,
        },
    );
    sync::sync_checked(&source, NOW, target).unwrap();
    sync::sync_checked(&peer, NOW, target).unwrap_err();
    assert!(!inspect(&peer).unwrap().member);
    assert!(
        !inspect(&source)
            .unwrap()
            .devices
            .iter()
            .any(|device| device.id == peer_id)
    );
    let cards = || crate::intent::Intent::Layout {
        layout: crate::intent::LayoutName::Cards,
    };
    assert_eq!(source_window.act(cards()).unwrap().listing.rows.len(), 1);
    assert_eq!(peer_window.act(cards()).unwrap().listing.rows.len(), 1);
    drop((source_window, peer_window));
    invitation.cleanup().unwrap();
    newcomer.cleanup().unwrap();
}

#[test]
fn a_key_created_after_an_existing_snapshot_recovers_the_whole_library() {
    let directory = tempfile::tempdir().unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let target = shared.path().to_str().unwrap().to_owned();
    sync::create(&store, NOW).unwrap();
    store
        .session(NOW, |session| {
            session.edit(youarel_core::state::Edit::AddShortcut {
                id: youarel_core::state::ItemId([1; 16]),
                link: KEPT_LINK.to_owned(),
                title: "Kept".to_owned(),
                folder: None,
            })
        })
        .unwrap();
    sync::sync_checked(&store, NOW, &target).unwrap();
    let mut devices = Devices::default();
    let mut key_job = devices
        .begin(store.clone(), Request::ShowRecoveryKey, NOW)
        .unwrap()
        .unwrap();
    let enrollment = key_job.run(|_update| {});
    devices.finish(key_job.token, enrollment);
    let key = devices.take_recovery().unwrap();
    sync::sync_checked(&store, NOW + 1, &target).unwrap();
    let recovered = Store::new(fresh.path().to_owned());
    let mut newcomer = Devices::default();
    let mut job = newcomer
        .begin(recovered.clone(), Request::Recover { target, key }, NOW)
        .unwrap()
        .unwrap();
    let result = job.run(|_update| {});
    assert!(result.is_ok(), "{result:?}");
    newcomer.finish(job.token, result);
    assert_eq!(
        store.session(NOW, |session| session.records()).unwrap(),
        recovered.session(NOW, |session| session.records()).unwrap()
    );
}

#[test]
fn names_use_full_membership_identities_and_old_results_leave_a_new_action_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let snapshot = inspect(&store).unwrap();
    assert!(!snapshot.joined);
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].id.len(), 64);
    let id = snapshot.this_device;
    let mut devices = Devices::default();
    let mut job = devices
        .begin(
            store.clone(),
            Request::NameDevice {
                id: id.clone(),
                name: "Desk 🖥️".to_owned(),
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    assert!(devices.busy());
    devices
        .begin(
            store.clone(),
            Request::NameDevice {
                id,
                name: String::new(),
            },
            NOW,
        )
        .unwrap_err();
    devices.update(Update {
        token: job.token + 1,
        phase: Phase::Done,
        code: Some("stale".to_owned()),
    });
    assert_eq!(devices.progress().phase, Phase::Preparing);
    let outcome = job.run(|update| devices.update(update));
    devices.finish(job.token, outcome);
    assert!(!devices.busy());
    assert_eq!(inspect(&store).unwrap().devices[0].name, "Desk 🖥️");
    devices.update(Update {
        token: job.token,
        phase: Phase::Waiting,
        code: Some("late".to_owned()),
    });
    assert_eq!(devices.progress().phase, Phase::Done);
    assert_eq!(devices.progress().code, None);
    job.run(|_update| {}).unwrap_err();
}

#[test]
fn recovery_material_reads_once_stays_out_of_progress_and_requires_acknowledgment() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    sync::create(&store, NOW).unwrap();
    let mut devices = Devices::default();
    let mut job = devices
        .begin(store.clone(), Request::ShowRecoveryKey, NOW)
        .unwrap()
        .unwrap();
    let result = job.run(|_update| {});
    devices.finish(job.token, result);
    let progress = devices.progress();
    assert!(progress.recovery_ready && progress.recovery_acknowledgment);
    let key = devices.take_recovery().unwrap();
    assert_ne!(key, "");
    assert!(
        !serde_json::to_string(&devices.progress())
            .unwrap()
            .contains(&key)
    );
    assert!(!format!("{devices:?}").contains(&key));
    devices.take_recovery().unwrap_err();
    devices
        .begin(store.clone(), Request::ShowRecoveryKey, NOW)
        .unwrap_err();
    assert!(
        devices
            .begin(store, Request::AcknowledgeRecovery, NOW)
            .unwrap()
            .is_none()
    );
    assert!(!devices.progress().recovery_acknowledgment);
}

#[test]
fn cancelling_before_the_worker_starts_preserves_cancellation_and_cleans_its_offer() {
    let directory = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let mut devices = Devices::default();
    let mut job = devices
        .begin(
            store.clone(),
            Request::Invite {
                target: shared.path().to_str().unwrap().to_owned(),
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    devices.begin(store, Request::CancelPairing, NOW).unwrap();
    let result = job.run(|update| devices.update(update));
    assert!(result.is_err());
    devices.finish(job.token, result);
    assert_eq!(devices.progress().phase, Phase::Cancelled);
    assert_eq!(devices.progress().error, None);
    assert!(!devices.busy());
    assert_eq!(
        youarel_store::fs::list(&shared.path().join("pairing")).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn device_requests_reject_extra_fields_and_membership_ids_never_accept_item_ids() {
    serde_json::from_str::<Request>(r#"{"kind":"invite","target":"shared","code":"unexpected"}"#)
        .unwrap_err();
    sync::devices::identity(&"a".repeat(32)).unwrap_err();
    let id = MemberId([0xab; 32]);
    assert_eq!(
        sync::devices::identity(&sync::devices::identity_text(id)).unwrap(),
        id
    );
}
