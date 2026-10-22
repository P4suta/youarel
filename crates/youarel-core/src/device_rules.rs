//! Device-screen progress and the admission of sealed device names.

pub const PAIRING_PATIENCE: u64 = 5 * 60 * 1000;
pub const POLL_MILLIS: u64 = 500;
pub const LONGEST_NAME: usize = 128;
pub const MOST_RUN: u64 = 9_007_199_254_740_991;
pub const LATEST_DISPLAY_TIME: u64 = 8_640_000_000_000_000;
pub const LONGEST_QR_TEXT: usize = 512;
pub const SMALLEST_QR_SIDE: u32 = 21;
pub const LARGEST_QR_SIDE: u32 = 177;
pub const QR_SIDE_STEP: u32 = 4;

#[must_use]
pub const fn admitted_qr_text(bytes: usize) -> bool {
    bytes > 0 && bytes <= LONGEST_QR_TEXT
}

#[must_use]
pub const fn admitted_qr_side(side: u32) -> bool {
    side >= SMALLEST_QR_SIDE
        && side <= LARGEST_QR_SIDE
        && side % QR_SIDE_STEP == SMALLEST_QR_SIDE % QR_SIDE_STEP
}

#[must_use]
pub const fn next_run(current: u64) -> Option<u64> {
    match current.checked_add(1) {
        Some(next) if next <= MOST_RUN => Some(next),
        Some(_) | None => None,
    }
}

#[must_use]
pub const fn displayed_time(time: u64) -> Option<u64> {
    if time == 0 || time > LATEST_DISPLAY_TIME {
        None
    } else {
        Some(time)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MemberId(pub [u8; 32]);

#[must_use]
pub fn admitted_name(name: &str) -> bool {
    name.len() <= LONGEST_NAME && name.trim() == name && !name.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Invite,
    Join,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Offered,
    Waiting,
    Confirming,
    Syncing,
    Done,
    Expired,
    Refused,
    Cancelled,
}

impl Stage {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Expired | Self::Refused | Self::Cancelled
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Poll,
    Ready,
    Peer,
    Joined,
    Synced,
    Refuse,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Run {
    began: u64,
    stage: Stage,
}

impl Run {
    #[must_use]
    pub const fn begin(role: Role, now: u64) -> Self {
        Self {
            began: now,
            stage: match role {
                Role::Invite => Stage::Offered,
                Role::Join => Stage::Waiting,
            },
        }
    }

    #[must_use]
    pub const fn stage(self) -> Stage {
        self.stage
    }

    #[must_use]
    pub const fn step(self, event: Event, now: u64) -> Self {
        let stage = if self.stage.terminal() {
            self.stage
        } else if matches!(self.stage, Stage::Syncing) {
            match event {
                Event::Synced => Stage::Done,
                Event::Refuse => Stage::Refused,
                Event::Poll | Event::Ready | Event::Peer | Event::Joined | Event::Cancel => {
                    Stage::Syncing
                }
            }
        } else if matches!(event, Event::Cancel) {
            Stage::Cancelled
        } else if now.saturating_sub(self.began) >= PAIRING_PATIENCE {
            Stage::Expired
        } else if matches!(event, Event::Refuse) {
            Stage::Refused
        } else {
            match (self.stage, event) {
                (Stage::Offered, Event::Ready) => Stage::Waiting,
                (Stage::Waiting, Event::Peer) => Stage::Confirming,
                (Stage::Confirming, Event::Joined) => Stage::Syncing,
                (Stage::Syncing, Event::Synced) => Stage::Done,
                _ => self.stage,
            }
        };
        Self { stage, ..self }
    }
}

#[cfg(any(test, kani))]
mod checks {
    #[cfg(kani)]
    #[kani::proof]
    fn qr_bounds_admit_only_nonempty_bounded_text_and_standard_squares() {
        let bytes: usize = kani::any();
        let side: u32 = kani::any();
        let text = super::admitted_qr_text(bytes);
        let square = super::admitted_qr_side(side);
        kani::assert(text == (bytes != 0 && bytes <= 512), "bounded QR payloads");
        kani::assert(
            square == ((21..=177).contains(&side) && side % 4 == 1),
            "standard QR dimensions without arithmetic overflow",
        );
        kani::cover!(text, "a QR payload");
        kani::cover!(!text, "a refused QR payload");
        kani::cover!(square, "a QR square");
        kani::cover!(!square, "a refused QR square");
    }

    #[cfg(kani)]
    #[kani::proof]
    fn display_integers_never_round_or_overflow() {
        let value: u64 = kani::any();
        let next = super::next_run(value);
        kani::assert(
            next.is_some() == (value < super::MOST_RUN),
            "run identifiers stop before JavaScript rounds them",
        );
        if let Some(next) = next {
            kani::assert(
                next > value && next <= super::MOST_RUN,
                "run identifiers increase without aliasing",
            );
        }
        let date = super::displayed_time(value);
        kani::assert(
            date.is_some() == (value > 0 && value <= super::LATEST_DISPLAY_TIME),
            "only exact representable dates reach the page",
        );
        kani::cover!(next.is_some(), "a run identifier");
        kani::cover!(next.is_none(), "the identifier bound");
        kani::cover!(date.is_some(), "a display date");
        kani::cover!(date.is_none(), "an unavailable date");
    }
    use super::{Event, PAIRING_PATIENCE, Role, Run, Stage};

    const fn stage(tag: u8) -> Stage {
        match tag {
            0 => Stage::Offered,
            1 => Stage::Waiting,
            2 => Stage::Confirming,
            3 => Stage::Syncing,
            4 => Stage::Done,
            5 => Stage::Expired,
            6 => Stage::Refused,
            _ => Stage::Cancelled,
        }
    }
    const fn event(tag: u8) -> Event {
        match tag {
            0 => Event::Poll,
            1 => Event::Ready,
            2 => Event::Peer,
            3 => Event::Joined,
            4 => Event::Synced,
            5 => Event::Refuse,
            _ => Event::Cancel,
        }
    }
    const fn rank(stage: Stage) -> u8 {
        match stage {
            Stage::Offered => 0,
            Stage::Waiting => 1,
            Stage::Confirming => 2,
            Stage::Syncing => 3,
            Stage::Done | Stage::Expired | Stage::Refused | Stage::Cancelled => 4,
        }
    }
    fn forward(run: Run, event: Event, now: u64) -> bool {
        let next = run.step(event, now);
        rank(next.stage) >= rank(run.stage)
            && (!run.stage.terminal() || next == run)
            && (run.stage.terminal()
                || matches!(run.stage, Stage::Syncing)
                || now.saturating_sub(run.began) < PAIRING_PATIENCE
                || matches!(next.stage, Stage::Expired | Stage::Cancelled))
    }
    #[cfg(kani)]
    #[kani::proof]
    fn screen_steps_only_advance_and_terminal_runs_never_restart() {
        let run = Run {
            began: kani::any(),
            stage: stage(kani::any()),
        };
        let event = event(kani::any());
        let now = kani::any();
        kani::assert(
            forward(run, event, now),
            "progress is forward and terminal runs stay final",
        );
        let next = run.step(event, now);
        kani::cover!(next.stage == Stage::Done, "a successful run");
        kani::cover!(next.stage == Stage::Expired, "an expired run");
        kani::cover!(next.stage == Stage::Refused, "a refused run");
        kani::cover!(next.stage == Stage::Cancelled, "a cancelled run");
    }
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
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_an_expired_invitation_completing() {
        let run = Run::begin(Role::Invite, 0)
            .step(Event::Ready, 0)
            .step(Event::Peer, 0)
            .step(Event::Joined, PAIRING_PATIENCE);
        kani::assert(
            run.step(Event::Synced, PAIRING_PATIENCE).stage == Stage::Done,
            "NEGATIVE_DEVICE_CONTROL",
        );
    }
    #[test]
    fn every_screen_state_obeys_the_transition_contract() {
        for state in 0..8 {
            for fact in 0..7 {
                for now in [0, PAIRING_PATIENCE - 1, PAIRING_PATIENCE, u64::MAX] {
                    assert!(forward(
                        Run {
                            began: 0,
                            stage: stage(state)
                        },
                        event(fact),
                        now
                    ));
                }
            }
        }
        let joined = Run::begin(Role::Join, 0)
            .step(Event::Peer, 0)
            .step(Event::Joined, 0)
            .step(Event::Synced, 1);
        assert_eq!(joined.stage(), Stage::Done);
        assert_eq!(
            Run::begin(Role::Invite, 0)
                .step(Event::Cancel, 0)
                .step(Event::Synced, 0)
                .stage(),
            Stage::Cancelled
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn qr_payload_and_square_bounds_refuse_empty_oversize_and_nonstandard_dimensions() {
        assert!(!super::admitted_qr_text(0));
        assert!(super::admitted_qr_text(1));
        assert!(super::admitted_qr_text(super::LONGEST_QR_TEXT));
        assert!(!super::admitted_qr_text(super::LONGEST_QR_TEXT + 1));
        assert!(!super::admitted_qr_text(usize::MAX));
        for side in 0..=200 {
            assert_eq!(
                super::admitted_qr_side(side),
                (21..=177).contains(&side) && side % 4 == 1
            );
        }
        assert!(!super::admitted_qr_side(u32::MAX));
    }

    #[test]
    fn page_numbers_and_dates_stop_at_their_exact_boundaries() {
        use super::{LATEST_DISPLAY_TIME, MOST_RUN, displayed_time, next_run};
        assert_eq!(next_run(0), Some(1));
        assert_eq!(next_run(MOST_RUN - 1), Some(MOST_RUN));
        assert_eq!(next_run(MOST_RUN), None);
        assert_eq!(next_run(u64::MAX), None);
        assert_eq!(displayed_time(0), None);
        assert_eq!(displayed_time(1), Some(1));
        assert_eq!(
            displayed_time(LATEST_DISPLAY_TIME),
            Some(LATEST_DISPLAY_TIME)
        );
        assert_eq!(displayed_time(LATEST_DISPLAY_TIME + 1), None);
        assert_eq!(displayed_time(u64::MAX), None);
    }

    #[test]
    fn sealed_names_admit_unicode_and_clear_but_refuse_padding_controls_and_oversize() {
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
