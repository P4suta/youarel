//! The decisions of the optional relay: which requests it admits, how long it keeps an account, what fits an account, and which pairing nameplate it hands out.
//!
//! The relay stores only sealed bundles, so these rules protect its availability and its account's freshness, not the secrecy of the state.

use alloc::vec::Vec;

/// The milliseconds a request's stamp may differ from the relay's clock.
pub const WINDOW: u64 = 120_000;

/// The milliseconds in a day.
const DAY: u64 = 86_400_000;

/// The milliseconds the relay keeps an account after its last authenticated request.
pub const RETENTION: u64 = 400 * DAY;

/// The milliseconds after which a device that published nothing to the relay counts as gone, which a member hears of.
pub const INACTIVE: u64 = 180 * DAY;

/// The milliseconds a pairing mailbox stays open.
pub const MAILBOX_LIFETIME: u64 = 5 * 60 * 1000;

/// The most bytes an account's bundles may hold together.
pub const MOST_BYTES: usize = 64 * 1024 * 1024;

/// The most devices whose bundles an account keeps.
pub const MOST_SLOTS: usize = 64;

/// The most pairing mailboxes the relay keeps open at once.
pub const MOST_MAILBOXES: usize = 4096;

/// The most bytes one mailbox holds, enough for an entry that carries the largest state.
pub const MOST_MAIL_BYTES: usize = 64 * 1024 * 1024;

/// The bytes of a request's nonce.
pub const NONCE: usize = 16;

/// A request's nonce.
pub type Nonce = [u8; NONCE];

/// A nonce the relay accepted, with the stamp of its request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Seen {
    pub nonce: Nonce,
    pub stamped: u64,
}

/// Whether the relay admits a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Fresh,
    /// Its stamp lies outside the window around the relay's clock.
    Stale,
    /// Its nonce came before.
    Replayed,
}

/// Whether a request stamped at `stamped` lies within the window around `now`.
#[must_use]
pub const fn fresh(now: u64, stamped: u64) -> bool {
    now.abs_diff(stamped) <= WINDOW
}

/// Whether the relay still keeps the nonce of a request stamped at `stamped`; once it forgets the nonce, the request is stale anyway.
#[must_use]
pub const fn retained(now: u64, stamped: u64) -> bool {
    now <= stamped.saturating_add(WINDOW)
}

/// Whether the relay admits a request stamped at `stamped` with `nonce`, given the nonces it retains.
#[must_use]
pub fn admit(now: u64, stamped: u64, nonce: &Nonce, seen: &[Seen]) -> Admission {
    if !fresh(now, stamped) {
        Admission::Stale
    } else if seen.iter().any(|entry| entry.nonce == *nonce) {
        Admission::Replayed
    } else {
        Admission::Fresh
    }
}

/// The nonces the relay keeps at `now`.
#[must_use]
pub fn prune(now: u64, seen: &[Seen]) -> Vec<Seen> {
    seen.iter()
        .copied()
        .filter(|entry| retained(now, entry.stamped))
        .collect()
}

/// When the relay deletes an account whose last authenticated request came at `last`.
#[must_use]
pub const fn expires(last: u64) -> u64 {
    last.saturating_add(RETENTION)
}

/// Whether a device whose last bundle reached the relay at `last` counts as gone at `now`.
#[must_use]
pub const fn inactive(now: u64, last: u64) -> bool {
    now.saturating_sub(last) > INACTIVE
}

/// Whether an account holding `total` bytes may replace a bundle of `replaced` bytes with one of `incoming` bytes.
#[must_use]
pub const fn fits(total: usize, replaced: usize, incoming: usize) -> bool {
    match total.saturating_sub(replaced).checked_add(incoming) {
        Some(after) => after <= MOST_BYTES,
        None => false,
    }
}

/// Whether a mailbox holding `total` bytes may take a message of `incoming` bytes.
#[must_use]
pub const fn fits_mail(total: usize, incoming: usize) -> bool {
    match total.checked_add(incoming) {
        Some(after) => after <= MOST_MAIL_BYTES,
        None => false,
    }
}

/// Whether an account with `slots` bundles may take one from a device, which holds a slot already or not.
#[must_use]
pub const fn admits_slot(slots: usize, holds: bool) -> bool {
    holds || slots < MOST_SLOTS
}

/// Whether a mailbox opened at `opened` still takes and hands out messages at `now`.
#[must_use]
pub const fn open(opened: u64, now: u64) -> bool {
    match now.checked_sub(opened) {
        Some(age) => age <= MAILBOX_LIFETIME,
        None => false,
    }
}

/// The smallest positive nameplate that no open mailbox holds, which keeps the codes people type short.
///
/// Among the first `active.len() + 1` positive numbers one is free, so the search marks only those and ends there.
#[must_use]
pub fn nameplate(active: &[u16]) -> Option<u16> {
    let Ok(count) = u16::try_from(active.len()) else {
        return None;
    };
    let last = count.checked_add(1)?;
    let mut taken = alloc::vec![false; usize::from(last).saturating_add(1)];
    for open in active {
        if let Some(mark) = taken.get_mut(usize::from(*open)) {
            *mark = true;
        }
    }
    (1..=last).find(|candidate| !taken.get(usize::from(*candidate)).copied().unwrap_or(true))
}

/// The properties Kani proves over every clock reading, stamp, size, and every short list of nonces and nameplates.
#[cfg(kani)]
mod proofs {
    use super::{
        Admission, INACTIVE, MAILBOX_LIFETIME, MOST_BYTES, MOST_MAIL_BYTES, NONCE, RETENTION, Seen,
        WINDOW, admit, expires, fits, fits_mail, fresh, inactive, nameplate, open, retained,
    };

    #[kani::proof]
    fn a_forgotten_nonce_belongs_to_a_stale_request() {
        let now: u64 = kani::any();
        let stamped: u64 = kani::any();
        kani::assert(
            fresh(now, stamped) == (now.abs_diff(stamped) <= WINDOW),
            "a request is fresh exactly within the window",
        );
        kani::assert(
            retained(now, stamped) || !fresh(now, stamped),
            "the relay forgets a nonce only once its request is stale",
        );
        kani::cover!(
            fresh(now, stamped) && !retained(now.wrapping_add(WINDOW), stamped),
            "a nonce forgotten later"
        );
    }

    /// Comparing two nonces compares their sixteen bytes, and the list holds three.
    #[kani::proof]
    #[kani::unwind(17)]
    fn a_request_passes_only_fresh_and_new() {
        let now: u64 = kani::any();
        let stamped: u64 = kani::any();
        let nonce: [u8; NONCE] = kani::any();
        let seen: [Seen; 3] = [
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
        ];
        let count: usize = kani::any();
        kani::assume(count <= seen.len());
        let Some(retained) = seen.get(..count) else {
            return;
        };
        let decision = admit(now, stamped, &nonce, retained);
        let known = retained.iter().any(|entry| entry.nonce == nonce);
        kani::assert(
            (decision == Admission::Fresh) == (fresh(now, stamped) && !known),
            "a request passes exactly when it is fresh and its nonce is new",
        );
        kani::assert(
            decision != Admission::Replayed || known,
            "only a known nonce counts as a replay",
        );
        kani::cover!(decision == Admission::Replayed, "a replay");
    }

    #[kani::proof]
    fn the_quota_and_the_clocks_never_overflow() {
        let (total, replaced, incoming): (usize, usize, usize) =
            (kani::any(), kani::any(), kani::any());
        kani::assert(
            fits(total, replaced, incoming)
                == (incoming <= MOST_BYTES
                    && total.saturating_sub(replaced) <= MOST_BYTES - incoming),
            "a bundle fits exactly when the total after it stays within the quota",
        );
        let (now, last): (u64, u64) = (kani::any(), kani::any());
        kani::assert(
            u128::from(expires(last))
                == (u128::from(last) + u128::from(RETENTION)).min(u128::from(u64::MAX)),
            "an account expires its retention after its last request",
        );
        kani::assert(
            inactive(now, last) == (now > last && now - last > INACTIVE),
            "a device is gone only after its inactivity",
        );
        let opened: u64 = kani::any();
        kani::assert(
            open(opened, now) == (now >= opened && now - opened <= MAILBOX_LIFETIME),
            "a mailbox stays open for its lifetime from its opening",
        );
        kani::assert(
            fits_mail(total, incoming)
                == (incoming <= MOST_MAIL_BYTES && total <= MOST_MAIL_BYTES - incoming),
            "a message fits exactly when the mailbox stays within its limit",
        );
        kani::cover!(
            fits(total, replaced, incoming) && incoming > 0,
            "a bundle fits"
        );
    }

    /// The search visits at most four candidates, and each candidate scans the three nameplates.
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_nameplate_is_the_smallest_free_one() {
        let active: [u16; 3] = kani::any();
        let count: usize = kani::any();
        kani::assume(count <= active.len());
        let Some(open) = active.get(..count) else {
            return;
        };
        let chosen = nameplate(open);
        kani::assert(
            chosen.is_some(),
            "a few open mailboxes leave a nameplate free",
        );
        if let Some(chosen) = chosen {
            kani::assert(
                chosen >= 1 && !open.contains(&chosen),
                "the nameplate is positive and free",
            );
            kani::assert(
                (1..chosen).all(|smaller| open.contains(&smaller)),
                "every smaller nameplate is taken",
            );
        }
        kani::cover!(chosen == Some(2), "the first nameplate is taken");
    }

    /// A request from beyond the window would let a recorded request come back once its nonce is gone; this probe claims one passes and must fail.
    #[kani::proof]
    fn rejects_a_request_from_beyond_the_window() {
        let now: u64 = kani::any();
        kani::assume(now > WINDOW);
        kani::assert(fresh(now, now - WINDOW - 1), "NEGATIVE_RELAY_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Admission, INACTIVE, MAILBOX_LIFETIME, MOST_BYTES, MOST_MAIL_BYTES, MOST_SLOTS, RETENTION,
        Seen, WINDOW, admit, admits_slot, expires, fits, fits_mail, fresh, inactive, nameplate,
        open, prune,
    };
    use alloc::vec::Vec;

    #[test]
    fn a_request_passes_within_the_window_once() {
        let now = 1_000_000;
        let seen = [Seen {
            nonce: [1; 16],
            stamped: now,
        }];
        assert_eq!(admit(now, now + WINDOW, &[2; 16], &seen), Admission::Fresh);
        assert_eq!(admit(now, now - WINDOW, &[2; 16], &seen), Admission::Fresh);
        assert_eq!(
            admit(now, now + WINDOW + 1, &[2; 16], &seen),
            Admission::Stale
        );
        assert_eq!(admit(now, now, &[1; 16], &seen), Admission::Replayed);
        assert!(fresh(u64::MAX, u64::MAX - WINDOW));
        assert_eq!(prune(now + WINDOW, &seen), seen);
        assert_eq!(prune(now + WINDOW + 1, &seen), []);
    }

    #[test]
    fn accounts_devices_and_mailboxes_age_by_their_limits() {
        assert_eq!(expires(5), 5 + RETENTION);
        assert_eq!(expires(u64::MAX), u64::MAX);
        assert!(!inactive(INACTIVE, 0));
        assert!(inactive(INACTIVE + 1, 0));
        assert!(!inactive(0, 5));
        assert!(open(10, 10 + MAILBOX_LIFETIME));
        assert!(!open(10, 11 + MAILBOX_LIFETIME));
        assert!(!open(10, 9));
    }

    #[test]
    fn an_account_holds_its_quota_and_its_slots() {
        assert!(fits_mail(MOST_MAIL_BYTES - 1, 1));
        assert!(!fits_mail(MOST_MAIL_BYTES, 1));
        assert!(!fits_mail(usize::MAX, 1));
        assert!(fits(MOST_BYTES, 10, 10));
        assert!(!fits(MOST_BYTES, 10, 11));
        assert!(!fits(0, 0, usize::MAX));
        assert!(fits(0, 5, MOST_BYTES));
        assert!(admits_slot(MOST_SLOTS - 1, false));
        assert!(!admits_slot(MOST_SLOTS, false));
        assert!(admits_slot(MOST_SLOTS, true));
    }

    /// The limits that `specs/PROTOCOL.md` states, in the units it states them in.
    #[test]
    fn the_limits_are_those_of_the_protocol() {
        const MINUTE: u64 = 60_000;
        const MEBIBYTE: usize = 1 << 20;
        assert!(expires(0) > 399 * super::DAY && expires(0) <= 400 * super::DAY);
        assert!(!inactive(180 * super::DAY, 0) && inactive(181 * super::DAY, 0));
        assert!(open(0, 4 * MINUTE) && !open(0, 6 * MINUTE));
        assert!(fits(0, 0, 64 * MEBIBYTE) && !fits(0, 0, 64 * MEBIBYTE + 1));
        assert!(fits_mail(0, 64 * MEBIBYTE) && !fits_mail(0, 64 * MEBIBYTE + 1));
        assert!(fresh(2 * MINUTE, 0) && !fresh(3 * MINUTE, 0));
    }

    #[test]
    fn the_nameplate_is_the_smallest_free_one() {
        assert_eq!(nameplate(&[]), Some(1));
        assert_eq!(nameplate(&[1, 3]), Some(2));
        assert_eq!(nameplate(&[2, 1]), Some(3));
        let full: Vec<u16> = (1..=u16::MAX).collect();
        assert_eq!(nameplate(&full), None);
    }
}
