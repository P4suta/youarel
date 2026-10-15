//! The members of an account, resolved from a log of operations that each name the operations their author saw.
//!
//! Devices exchange their logs and merge them by union, so two devices that hold the same operations must draw the same members, whatever order the operations arrived in.
//! The resolution replays the operations in one order fixed by the log alone: an operation waits for its parents, and among the ready ones the next is the one whose author joined first, then the one with the smallest digest.
//! An operation counts only when its author is a member as it is replayed, and a removal never leaves the account without a member.
//! The adapter checks every signature and the account before it hands the operations over, so this module decides from their content alone.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

/// The digest that names an operation.
pub type Digest = [u8; 32];

/// The identifier of a device: the digest of its public keys.
pub type Device = [u8; 32];

/// What an operation does.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Action {
    /// Founds the account with its author as the only member.
    Genesis,
    /// Makes a device a member.
    Add(Device),
    /// Ends a device's membership, unless it is the last member.
    Remove(Device),
    /// Moves the account to a new epoch, when its number is greater than the current one.
    Rotate(u32),
}

/// One signed operation of the log, as the adapter verified it.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Operation {
    pub digest: Digest,
    /// The operations its author saw, by digest.
    pub parents: Vec<Digest>,
    pub author: Device,
    pub action: Action,
}

/// What the log comes to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Resolution {
    /// The members, in ascending order.
    pub members: Vec<Device>,
    /// The current epoch, and the operation that rotated to it; epoch zero belongs to the genesis.
    pub epoch: u32,
    pub rotation: Option<Digest>,
    /// Every rotation a member wrote, in the order of the replay, including those that lost to a greater or an earlier epoch.
    ///
    /// Two members may rotate to one epoch at once, and a member that sealed under the losing key still holds it, so a reader keeps the key of every rotation here.
    pub rotations: Vec<Digest>,
    /// Every operation in the order of the replay, applied or skipped.
    pub order: Vec<Digest>,
    /// The operations that took effect, in the order of the replay.
    pub applied: Vec<Digest>,
}

/// The account as the replay builds it: each member with the step it joined at, and the current epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Members {
    joined: Vec<(Device, usize)>,
    epoch: u32,
}

/// One operation as the replay meets it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Step<'a> {
    /// Whether the operation is the account's genesis: its digest names the account and it has no parents.
    pub founds: bool,
    pub author: &'a Device,
    pub action: Action,
    /// The replay step the operation takes, which a device it adds joins at.
    pub index: usize,
}

/// What an operation did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// It changed nothing: its author is no member, its device already is or is not one, it would leave no member, or it rotates to no greater epoch.
    Skipped,
    Applied,
    /// It rotated to a greater epoch.
    Rotated,
}

/// What the account looks like to one operation as the replay meets it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Facts {
    /// Whether the operation is the account's genesis: its digest names the account and it has no parents.
    pub founds: bool,
    pub members: usize,
    /// Whether the author is a member.
    pub author: bool,
    /// Whether the device that an addition or a removal names is a member.
    pub target: bool,
    pub epoch: u32,
}

/// What the replay does with one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Skip,
    /// The author becomes the first member.
    Found,
    Join(Device),
    Leave(Device),
    Rotate(u32),
}

/// The decision of one replay step, from the operation's action and the facts it meets.
///
/// The genesis founds only an empty account; any other action counts only when its author is a member, an addition only for a stranger, a removal only for a member that is not the last, and a rotation only to a greater epoch.
#[must_use]
pub const fn decide(action: Action, facts: Facts) -> Decision {
    match action {
        Action::Genesis if facts.founds && facts.members == 0 => Decision::Found,
        Action::Add(device) if facts.author && !facts.target => Decision::Join(device),
        Action::Remove(device) if facts.author && facts.target && facts.members > 1 => {
            Decision::Leave(device)
        }
        Action::Rotate(epoch) if facts.author && epoch > facts.epoch => Decision::Rotate(epoch),
        Action::Genesis | Action::Add(_) | Action::Remove(_) | Action::Rotate(_) => Decision::Skip,
    }
}

impl Members {
    /// Whether the device is a member.
    #[must_use]
    pub fn contains(&self, device: &Device) -> bool {
        self.joined.iter().any(|(member, _)| member == device)
    }

    /// The place of an author in the replay: the step it joined at, or after every member when it is none.
    #[must_use]
    pub fn rank(&self, author: &Device) -> usize {
        self.joined
            .iter()
            .find(|(member, _)| member == author)
            .map_or(usize::MAX, |(_, step)| *step)
    }

    #[must_use]
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// How many members the account has.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.joined.len()
    }

    /// Whether the account has no member: no genesis founded it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.joined.is_empty()
    }

    /// The members in ascending order.
    #[must_use]
    pub fn devices(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self.joined.iter().map(|(member, _)| *member).collect();
        devices.sort_unstable();
        devices
    }

    /// The facts the operation of `step` meets in this account.
    #[must_use]
    pub fn facts(&self, step: &Step<'_>) -> Facts {
        let target = match step.action {
            Action::Add(device) | Action::Remove(device) => self.contains(&device),
            Action::Genesis | Action::Rotate(_) => false,
        };
        Facts {
            founds: step.founds,
            members: self.joined.len(),
            author: self.contains(step.author),
            target,
            epoch: self.epoch,
        }
    }

    /// Applies one operation of the replay, as [`decide`] decides it.
    pub fn apply(&mut self, step: &Step<'_>) -> Effect {
        match decide(step.action, self.facts(step)) {
            Decision::Skip => Effect::Skipped,
            Decision::Found => {
                self.joined.push((*step.author, step.index));
                Effect::Applied
            }
            Decision::Join(device) => {
                self.joined.push((device, step.index));
                Effect::Applied
            }
            Decision::Leave(device) => {
                self.joined.retain(|(member, _)| *member != device);
                Effect::Applied
            }
            Decision::Rotate(epoch) => {
                self.epoch = epoch;
                Effect::Rotated
            }
        }
    }
}

/// The members, the epoch, and the replay order of the operations, for the account whose genesis is `genesis`.
///
/// Operations whose parents never all appear, and repeated digests after the first, take no part.
#[must_use]
pub fn resolve(genesis: Digest, operations: &[Operation]) -> Resolution {
    let mut pending: BTreeMap<Digest, &Operation> = BTreeMap::new();
    for operation in operations {
        pending.entry(operation.digest).or_insert(operation);
    }
    let mut placed: BTreeSet<Digest> = BTreeSet::new();
    let mut members = Members::default();
    let mut resolution = Resolution::default();
    for index in 0..pending.len() {
        let next = pending
            .values()
            .filter(|operation| {
                operation
                    .parents
                    .iter()
                    .all(|parent| placed.contains(parent))
            })
            .min_by_key(|operation| (members.rank(&operation.author), operation.digest))
            .copied();
        let Some(operation) = next else {
            break;
        };
        pending.remove(&operation.digest);
        placed.insert(operation.digest);
        resolution.order.push(operation.digest);
        if matches!(operation.action, Action::Rotate(_)) && members.contains(&operation.author) {
            resolution.rotations.push(operation.digest);
        }
        let step = Step {
            founds: operation.digest == genesis && operation.parents.is_empty(),
            author: &operation.author,
            action: operation.action,
            index,
        };
        match members.apply(&step) {
            Effect::Skipped => {}
            Effect::Applied => resolution.applied.push(operation.digest),
            Effect::Rotated => {
                resolution.applied.push(operation.digest);
                resolution.rotation = Some(operation.digest);
            }
        }
    }
    resolution.epoch = members.epoch();
    resolution.members = members.devices();
    resolution
}

/// The properties Kani proves of every decision of the replay, and the specifications of a step and of the whole replay that the tests check over every small account and log.
///
/// The replay keeps its order in B-trees and its members in a vector, which a bounded model checker cannot unroll, so Kani proves the pure decision, and an exhaustive sweep and the Quint model cover the rest.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Action, Device};
    #[cfg(kani)]
    use super::{Decision, Facts, decide};
    #[cfg(test)]
    use super::{Digest, Effect, Members, Operation, Resolution, Step, resolve};
    #[cfg(test)]
    use alloc::vec::Vec;

    /// What `resolve` promises for a log: the same result for the reversed log, a member after the genesis applies, and only effects whose authors were members when they were replayed.
    #[cfg(test)]
    pub(super) fn spec(genesis: Digest, operations: &[Operation]) -> bool {
        let resolution = resolve(genesis, operations);
        let reversed: Vec<Operation> = operations.iter().rev().cloned().collect();
        let independent = resolve(genesis, &reversed) == resolution;
        let founded = resolution.applied.first() == Some(&genesis);
        let kept = !founded || !resolution.members.is_empty();
        let ordered = resolution.order.iter().enumerate().all(|(index, digest)| {
            operations
                .iter()
                .find(|operation| operation.digest == *digest)
                .is_some_and(|operation| {
                    operation.parents.iter().all(|parent| {
                        resolution
                            .order
                            .get(..index)
                            .is_some_and(|before| before.contains(parent))
                    })
                })
        });
        let authorized = authorized(genesis, operations, &resolution);
        independent && kept && ordered && authorized
    }

    /// Whether every applied operation's author was a member just before it, replaying the applied operations alone.
    #[cfg(test)]
    fn authorized(genesis: Digest, operations: &[Operation], resolution: &Resolution) -> bool {
        let mut members: Vec<Device> = Vec::new();
        for digest in &resolution.applied {
            let Some(operation) = operations
                .iter()
                .find(|operation| operation.digest == *digest)
            else {
                return false;
            };
            let founding = operation.action == Action::Genesis;
            if founding != members.is_empty() || (!founding && !members.contains(&operation.author))
            {
                return false;
            }
            match operation.action {
                Action::Genesis => {
                    if *digest != genesis {
                        return false;
                    }
                    members.push(operation.author);
                }
                Action::Add(device) => members.push(device),
                Action::Remove(device) => members.retain(|member| *member != device),
                Action::Rotate(_) => {}
            }
        }
        members.sort_unstable();
        members == resolution.members
    }

    /// The device a test draws by its first byte.
    #[cfg(test)]
    pub(super) const fn device(byte: u8) -> Device {
        [byte; 32]
    }

    /// The digest of the operation a test places at `index`.
    #[cfg(test)]
    pub(super) const fn digest(index: u8) -> Digest {
        let mut digest = [0xd0; 32];
        digest[0] = index;
        digest
    }

    /// The devices a test draws from: up to three members and one stranger more.
    #[cfg(test)]
    pub(super) const DEVICES: u8 = 4;

    /// What one replay step promises.
    ///
    /// A genesis founds only an empty account; nothing else changes the account unless its author is a member; an addition makes a stranger a member; a removal never takes the last member; the epoch only grows, and only through a rotation.
    #[cfg(test)]
    pub(super) fn step_spec(before: &Members, step: &Step<'_>) -> bool {
        let mut after = before.clone();
        let effect = after.apply(step);
        let changed = effect != Effect::Skipped;
        let authorized =
            matches!(step.action, Action::Genesis) || before.contains(step.author) || !changed;
        let founded = !matches!(step.action, Action::Genesis)
            || changed == (step.founds && before.is_empty());
        let added = match step.action {
            Action::Add(device) => {
                changed == (before.contains(step.author) && !before.contains(&device))
                    && (!changed || after.contains(&device))
            }
            Action::Genesis | Action::Remove(_) | Action::Rotate(_) => true,
        };
        let removed = match step.action {
            Action::Remove(device) => {
                let allowed =
                    before.contains(step.author) && before.contains(&device) && before.len() > 1;
                changed == allowed && (!changed || !after.contains(&device))
            }
            Action::Genesis | Action::Add(_) | Action::Rotate(_) => true,
        };
        let kept = before.is_empty() || !after.is_empty();
        let epoch = match step.action {
            Action::Rotate(epoch) => {
                let rotates = before.contains(step.author) && epoch > before.epoch();
                (effect == Effect::Rotated) == rotates
                    && after.epoch() == if rotates { epoch } else { before.epoch() }
            }
            Action::Genesis | Action::Add(_) | Action::Remove(_) => {
                after.epoch() == before.epoch() && effect != Effect::Rotated
            }
        };
        authorized && founded && added && removed && kept && epoch
    }

    /// What one decision promises, spelled out for every pair of an action and a decision.
    #[cfg(kani)]
    fn decision_spec(action: Action, facts: Facts, decision: Decision) -> bool {
        let founds = facts.founds && facts.members == 0;
        match (action, decision) {
            (Action::Genesis, Decision::Found) => founds,
            (Action::Genesis, Decision::Skip) => !founds,
            (Action::Add(device), Decision::Join(joined)) => {
                facts.author && !facts.target && joined == device
            }
            (Action::Add(_), Decision::Skip) => !facts.author || facts.target,
            (Action::Remove(device), Decision::Leave(left)) => {
                facts.author && facts.target && facts.members > 1 && left == device
            }
            (Action::Remove(_), Decision::Skip) => {
                !facts.author || !facts.target || facts.members <= 1
            }
            (Action::Rotate(epoch), Decision::Rotate(to)) => {
                facts.author && epoch > facts.epoch && to == epoch
            }
            (Action::Rotate(epoch), Decision::Skip) => !facts.author || epoch <= facts.epoch,
            (Action::Genesis, Decision::Join(_) | Decision::Leave(_) | Decision::Rotate(_))
            | (Action::Add(_), Decision::Found | Decision::Leave(_) | Decision::Rotate(_))
            | (Action::Remove(_), Decision::Found | Decision::Join(_) | Decision::Rotate(_))
            | (Action::Rotate(_), Decision::Found | Decision::Join(_) | Decision::Leave(_)) => {
                false
            }
        }
    }

    #[cfg(kani)]
    fn any_action() -> Action {
        match kani::any::<u8>() % 4 {
            0 => Action::Genesis,
            1 => Action::Add(kani::any()),
            2 => Action::Remove(kani::any()),
            _ => Action::Rotate(kani::any()),
        }
    }

    /// Comparing the decided device with the named one compares their thirty-two bytes.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(33)]
    fn every_decision_keeps_a_member_and_counts_only_members() {
        let action = any_action();
        let facts = Facts {
            founds: kani::any(),
            members: kani::any(),
            author: kani::any(),
            target: kani::any(),
            epoch: kani::any(),
        };
        let decision = decide(action, facts);
        kani::assert(
            decision_spec(action, facts, decision),
            "a decision keeps its promises",
        );
        kani::assert(
            !matches!(decision, Decision::Leave(_)) || facts.members > 1,
            "a removal never takes the last member",
        );
        kani::cover!(matches!(decision, Decision::Leave(_)), "a member leaves");
        kani::cover!(matches!(decision, Decision::Rotate(_)), "a rotation");
    }

    /// A removal that empties the account would orphan its data; this probe claims one does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_removal_of_the_last_member() {
        let founder: Device = [1; 32];
        let facts = Facts {
            founds: false,
            members: 1,
            author: true,
            target: true,
            epoch: 0,
        };
        kani::assert(
            matches!(decide(Action::Remove(founder), facts), Decision::Leave(_)),
            "NEGATIVE_MEMBERSHIP_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{DEVICES, device, digest, spec, step_spec};
    use super::{Action, Members, Operation, Step, resolve};
    use alloc::vec;
    use alloc::vec::Vec;

    fn operation(index: u8, parents: &[u8], author: u8, action: Action) -> Operation {
        Operation {
            digest: digest(index),
            parents: parents.iter().map(|parent| digest(*parent)).collect(),
            author: device(author),
            action,
        }
    }

    #[test]
    fn a_founder_adds_a_device_and_rotates() {
        let log = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 1, Action::Add(device(2))),
            operation(2, &[1], 2, Action::Rotate(1)),
        ];
        let resolution = resolve(digest(0), &log);
        assert_eq!(resolution.members, [device(1), device(2)]);
        assert_eq!(resolution.epoch, 1);
        assert_eq!(resolution.rotation, Some(digest(2)));
        assert_eq!(resolution.rotations, [digest(2)]);
    }

    #[test]
    fn concurrent_rotations_to_one_epoch_both_keep_their_keys() {
        let log = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 1, Action::Add(device(2))),
            operation(2, &[1], 1, Action::Rotate(1)),
            operation(3, &[1], 2, Action::Rotate(1)),
            operation(4, &[1], 3, Action::Rotate(2)),
        ];
        let resolution = resolve(digest(0), &log);
        assert_eq!(resolution.epoch, 1);
        assert_eq!(resolution.rotation, Some(digest(2)));
        assert_eq!(resolution.rotations, [digest(2), digest(3)]);
        assert!(!resolution.applied.contains(&digest(3)));
        assert_eq!(resolution.applied, [digest(0), digest(1), digest(2)]);
    }

    #[test]
    fn strangers_another_genesis_and_orphans_change_nothing() {
        let log = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 3, Action::Add(device(3))),
            operation(2, &[], 4, Action::Genesis),
            operation(3, &[9], 1, Action::Add(device(5))),
            operation(4, &[0], 1, Action::Rotate(0)),
        ];
        let resolution = resolve(digest(0), &log);
        assert_eq!(resolution.members, [device(1)]);
        assert_eq!(resolution.applied, [digest(0)]);
        assert_eq!(resolution.epoch, 0);
        assert!(!resolution.order.contains(&digest(3)));
        assert_eq!(resolve(digest(7), &log).members.len(), 0);
    }

    #[test]
    fn two_devices_that_remove_each_other_leave_the_earlier_member() {
        let log = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 1, Action::Add(device(2))),
            operation(2, &[1], 1, Action::Remove(device(2))),
            operation(3, &[1], 2, Action::Remove(device(1))),
        ];
        let resolution = resolve(digest(0), &log);
        assert_eq!(resolution.members, [device(1)]);
        assert_eq!(resolution.applied, [digest(0), digest(1), digest(2)]);
        let alone = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 1, Action::Remove(device(1))),
        ];
        assert_eq!(resolve(digest(0), &alone).members, [device(1)]);
    }

    #[test]
    fn a_removed_device_returns_only_through_a_new_addition() {
        let log = [
            operation(0, &[], 1, Action::Genesis),
            operation(1, &[0], 1, Action::Add(device(2))),
            operation(2, &[1], 1, Action::Remove(device(2))),
            operation(3, &[2], 2, Action::Add(device(2))),
            operation(4, &[2], 1, Action::Add(device(2))),
            operation(5, &[4], 1, Action::Add(device(2))),
        ];
        let resolution = resolve(digest(0), &log);
        assert_eq!(resolution.members, [device(1), device(2)]);
        assert_eq!(
            resolution.applied,
            [digest(0), digest(1), digest(2), digest(4)]
        );
    }

    #[test]
    fn the_step_specification_agrees_over_every_small_account() {
        for subset in 0..(1_u8 << (DEVICES - 1)) {
            for epoch in 0..u32::from(DEVICES) {
                let mut members = Members::default();
                for byte in 0..DEVICES - 1 {
                    if subset & (1 << byte) != 0 {
                        members.joined.push((device(byte), usize::from(byte)));
                    }
                }
                members.epoch = epoch;
                for author in 0..DEVICES {
                    for target in 0..DEVICES {
                        for action in [
                            Action::Genesis,
                            Action::Add(device(target)),
                            Action::Remove(device(target)),
                            Action::Rotate(u32::from(target)),
                        ] {
                            for founds in [false, true] {
                                let step = Step {
                                    founds,
                                    author: &device(author),
                                    action,
                                    index: 3,
                                };
                                assert!(step_spec(&members, &step), "{members:?} {step:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "an exhaustive sweep that the native run covers")]
    fn the_specification_agrees_over_every_small_log() {
        let actions = |author: u8| {
            let other = (author + 1) % DEVICES;
            [
                Action::Genesis,
                Action::Add(device(other)),
                Action::Remove(device(other)),
                Action::Remove(device(author)),
                Action::Rotate(1),
            ]
        };
        let mut logs: Vec<Vec<Operation>> = vec![Vec::new()];
        for index in 0..3_u8 {
            let mut longer = Vec::new();
            for log in &logs {
                for author in 0..DEVICES {
                    for action in actions(author) {
                        for parents in 0..(1_u8 << index) {
                            let chosen: Vec<u8> = (0..index)
                                .filter(|earlier| parents & (1 << earlier) != 0)
                                .collect();
                            let mut next = log.clone();
                            next.push(operation(index, &chosen, author, action));
                            longer.push(next);
                        }
                    }
                }
            }
            logs = longer;
        }
        for log in &logs {
            assert!(spec(digest(0), log), "{log:?}");
        }
    }
}
