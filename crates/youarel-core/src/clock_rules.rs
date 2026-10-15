//! Hybrid logical clocks, which order every write across devices without trusting their wall clocks to agree.
//!
//! A stamp orders by its wall time, then its counter, then its device, so two writes never compare equal unless they are one write.

/// The device a write came from, which breaks ties between writes at the same moment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DeviceId(pub [u8; 16]);

/// One write's place in the total order of writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Stamp {
    /// Milliseconds since the Unix epoch, as the writer's clock or a later stamp it saw.
    pub wall: u64,
    /// The writes within one wall time.
    pub counter: u32,
    /// The writer.
    pub device: DeviceId,
}

impl Stamp {
    /// The stamp before any write of `device`.
    #[must_use]
    pub const fn origin(device: DeviceId) -> Self {
        Self {
            wall: 0,
            counter: 0,
            device,
        }
    }
}

/// The stamp after `wall` and `counter`, on the same wall time while the counter lasts.
const fn after(wall: u64, counter: u32, device: DeviceId) -> Option<Stamp> {
    if let Some(counter) = counter.checked_add(1) {
        return Some(Stamp {
            wall,
            counter,
            device,
        });
    }
    match wall.checked_add(1) {
        Some(wall) => Some(Stamp {
            wall,
            counter: 0,
            device,
        }),
        None => None,
    }
}

/// The stamp of the next local write after `last`, when the device's clock reads `now`.
///
/// The stamp orders after `last` and carries its device, and its wall time never falls behind `now`.
/// Returns `None` only once the clock has spent every stamp, at the last wall time and counter.
#[must_use]
pub const fn tick(last: Stamp, now: u64) -> Option<Stamp> {
    if now > last.wall {
        return Some(Stamp {
            wall: now,
            counter: 0,
            device: last.device,
        });
    }
    after(last.wall, last.counter, last.device)
}

/// The local clock after it saw `remote`, so the next local write orders after both.
#[must_use]
pub fn observe(last: Stamp, remote: Stamp) -> Stamp {
    let (wall, counter) = (last.wall, last.counter).max((remote.wall, remote.counter));
    Stamp {
        wall,
        counter,
        device: last.device,
    }
}

/// The properties Kani proves over every stamp and every clock reading.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Stamp, observe, tick};

    /// The moment of a stamp without its device.
    const fn moment(stamp: Stamp) -> (u64, u32) {
        (stamp.wall, stamp.counter)
    }

    /// What `tick` promises: a later stamp of the same device that keeps up with the clock, or `None` at the very end.
    pub(super) fn tick_spec(last: Stamp, now: u64) -> bool {
        tick(last, now).map_or_else(
            || last.wall == u64::MAX && last.counter == u32::MAX,
            |next| {
                next > last
                    && moment(next) > moment(last)
                    && next.device == last.device
                    && next.wall >= now
            },
        )
    }

    /// What `observe` promises: the own device, at or after both clocks, so the next tick orders after the remote stamp.
    pub(super) fn observe_spec(last: Stamp, remote: Stamp, now: u64) -> bool {
        let seen = observe(last, remote);
        let caught_up = moment(seen) >= moment(last) && moment(seen) >= moment(remote);
        let next_orders_after = tick(seen, now).is_none_or(|next| next > remote && next > last);
        seen.device == last.device && caught_up && next_orders_after
    }

    #[cfg(kani)]
    fn any_stamp() -> Stamp {
        Stamp {
            wall: kani::any(),
            counter: kani::any(),
            device: super::DeviceId(kani::any()),
        }
    }

    #[cfg(kani)]
    #[kani::proof]
    fn a_tick_orders_after_the_last_stamp_and_keeps_up_with_the_clock() {
        let last = any_stamp();
        let now: u64 = kani::any();
        kani::assert(tick_spec(last, now), "a tick advances");
        kani::cover!(
            tick(last, now).is_some_and(|next| next.counter == 0),
            "the clock moved on"
        );
        kani::cover!(
            tick(last, now).is_some_and(|next| next.wall == last.wall),
            "the counter advanced"
        );
        kani::cover!(tick(last, now).is_none(), "the clock is spent");
    }

    #[cfg(kani)]
    #[kani::proof]
    fn a_write_after_an_observed_stamp_orders_after_it() {
        let last = any_stamp();
        let remote = any_stamp();
        let now: u64 = kani::any();
        kani::assert(
            observe_spec(last, remote, now),
            "an observed stamp precedes the next write",
        );
        kani::cover!(
            observe(last, remote).wall == remote.wall && remote.wall > last.wall,
            "the remote clock was ahead"
        );
    }

    /// A tick that repeats its stamp would let two writes tie; this probe claims one does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_tick_that_repeats_the_stamp() {
        let last = Stamp {
            wall: 5,
            counter: 0,
            device: super::DeviceId([0; 16]),
        };
        kani::assert(tick(last, 5) == Some(last), "NEGATIVE_CLOCK_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{observe_spec, tick_spec};
    use super::{DeviceId, Stamp, observe, tick};

    const DEVICE: DeviceId = DeviceId([1; 16]);
    const OTHER: DeviceId = DeviceId([2; 16]);

    const fn stamp(wall: u64, counter: u32, device: DeviceId) -> Stamp {
        Stamp {
            wall,
            counter,
            device,
        }
    }

    #[test]
    fn a_tick_follows_the_clock_or_counts_within_its_wall_time() {
        assert_eq!(tick(stamp(5, 3, DEVICE), 9), Some(stamp(9, 0, DEVICE)));
        assert_eq!(tick(stamp(5, 3, DEVICE), 5), Some(stamp(5, 4, DEVICE)));
        assert_eq!(tick(stamp(5, 3, DEVICE), 2), Some(stamp(5, 4, DEVICE)));
        assert_eq!(
            tick(stamp(5, u32::MAX, DEVICE), 5),
            Some(stamp(6, 0, DEVICE))
        );
        assert_eq!(tick(stamp(u64::MAX, u32::MAX, DEVICE), 0), None);
        assert_eq!(Stamp::origin(DEVICE), stamp(0, 0, DEVICE));
    }

    #[test]
    fn observing_catches_up_with_the_later_clock_and_keeps_the_device() {
        assert_eq!(
            observe(stamp(5, 3, DEVICE), stamp(7, 1, OTHER)),
            stamp(7, 1, DEVICE)
        );
        assert_eq!(
            observe(stamp(7, 4, DEVICE), stamp(7, 1, OTHER)),
            stamp(7, 4, DEVICE)
        );
        assert_eq!(
            observe(stamp(7, 1, DEVICE), stamp(7, 4, OTHER)),
            stamp(7, 4, DEVICE)
        );
    }

    #[test]
    fn stamps_order_by_wall_then_counter_then_device() {
        assert!(stamp(1, 9, OTHER) < stamp(2, 0, DEVICE));
        assert!(stamp(2, 0, OTHER) < stamp(2, 1, DEVICE));
        assert!(stamp(2, 1, DEVICE) < stamp(2, 1, OTHER));
    }

    #[test]
    fn the_specifications_agree_over_the_edges() {
        let walls = [0, 1, 5, u64::MAX - 1, u64::MAX];
        let counters = [0, 1, u32::MAX - 1, u32::MAX];
        for wall in walls {
            for counter in counters {
                for now in walls {
                    let last = stamp(wall, counter, DEVICE);
                    assert!(tick_spec(last, now), "{last:?} {now}");
                    for remote_wall in walls {
                        let remote = stamp(remote_wall, counter, OTHER);
                        assert!(observe_spec(last, remote, now), "{last:?} {remote:?} {now}");
                    }
                }
            }
        }
    }
}
