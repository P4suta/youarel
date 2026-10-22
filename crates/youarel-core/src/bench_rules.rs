//! Admitted benchmark sizes, identities, and cooperative resource limits.

use crate::state::ItemId;
use core::num::NonZeroU64;

const GIBIBYTE: u64 = 1 << 30;
const FOLDER_NAMESPACE: u8 = 1;
const SHORTCUT_NAMESPACE: u8 = 2;
const IMPORT_NAMESPACE: u8 = 3;
const REMOVAL_FOLDER_NAMESPACE: u8 = 4;
const REMOVAL_SHORTCUT_NAMESPACE: u8 = 5;
const ADDED_NAMESPACE: u8 = 6;
pub const MOST_SHORTCUTS: u64 = 10_000_000;
pub const MOST_FOLDERS: u64 = 100_000;
pub const MOST_BYTES: u64 = 40 * GIBIBYTE;
pub const LEAST_FREE: u64 = 30 * GIBIBYTE;
pub const WRITE_RESERVE: u64 = 9 * GIBIBYTE;
pub const MOST_MILLIS: u64 = 12 * 60 * 60 * 1000;
pub const MOST_IMPORTED: u64 = 100_000;
pub const MOST_REMOVED: u64 = 1000;
pub const PURGED_PERIOD: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Workload {
    shortcuts: NonZeroU64,
    folders: u64,
    seed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Invalid {
    Shortcuts,
    Folders,
}

impl Workload {
    /// # Errors
    ///
    /// Refuses zero shortcuts or counts beyond the supported campaign sizes.
    pub fn new(shortcuts: u64, folders: u64, seed: u64) -> Result<Self, Invalid> {
        let shortcuts = NonZeroU64::new(shortcuts)
            .filter(|count| count.get() <= MOST_SHORTCUTS)
            .ok_or(Invalid::Shortcuts)?;
        if folders > MOST_FOLDERS {
            return Err(Invalid::Folders);
        }
        Ok(Self {
            shortcuts,
            folders,
            seed,
        })
    }
    #[must_use]
    pub const fn shortcuts(self) -> u64 {
        self.shortcuts.get()
    }
    #[must_use]
    pub const fn folders(self) -> u64 {
        self.folders
    }
    #[must_use]
    pub const fn seed(self) -> u64 {
        self.seed
    }
    #[must_use]
    pub const fn imported(self) -> u64 {
        if self.shortcuts.get() > MOST_IMPORTED {
            MOST_IMPORTED
        } else {
            self.shortcuts.get()
        }
    }
    #[must_use]
    pub const fn removed(self) -> u64 {
        if self.shortcuts.get() > MOST_REMOVED {
            MOST_REMOVED
        } else {
            self.shortcuts.get()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Namespace {
    Folder,
    Shortcut,
    Imported,
    RemovalFolder,
    RemovalShortcut,
    Added,
}

impl Namespace {
    const fn tag(self) -> u8 {
        match self {
            Self::Folder => FOLDER_NAMESPACE,
            Self::Shortcut => SHORTCUT_NAMESPACE,
            Self::Imported => IMPORT_NAMESPACE,
            Self::RemovalFolder => REMOVAL_FOLDER_NAMESPACE,
            Self::RemovalShortcut => REMOVAL_SHORTCUT_NAMESPACE,
            Self::Added => ADDED_NAMESPACE,
        }
    }
    #[must_use]
    pub const fn id(self, number: u64, created: u64) -> ItemId {
        let [
            counter_7,
            counter_6,
            counter_5,
            counter_4,
            counter_3,
            counter_2,
            counter_1,
            counter_0,
        ] = number.to_be_bytes();
        ItemId::v7(
            created,
            [
                self.tag(),
                counter_7,
                0,
                counter_6,
                counter_5,
                counter_4,
                counter_3,
                counter_2,
                counter_1,
                counter_0,
            ],
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resources {
    pub synthetic_bytes: u64,
    pub available_bytes: u64,
    pub elapsed_millis: u64,
    pub cancelled: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stop {
    Cancelled,
    Deadline,
    Storage,
    FreeSpace,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Continue,
    Stop(Stop),
}

#[must_use]
pub const fn permission(resources: Resources) -> Permission {
    if resources.cancelled {
        Permission::Stop(Stop::Cancelled)
    } else if resources.elapsed_millis >= MOST_MILLIS {
        Permission::Stop(Stop::Deadline)
    } else if resources.synthetic_bytes > MOST_BYTES.saturating_sub(WRITE_RESERVE) {
        Permission::Stop(Stop::Storage)
    } else if resources.available_bytes < LEAST_FREE.saturating_add(WRITE_RESERVE) {
        Permission::Stop(Stop::FreeSpace)
    } else {
        Permission::Continue
    }
}

#[cfg(kani)]
mod proofs {
    use super::{Namespace, Permission, Resources, Stop, Workload};
    #[kani::proof]
    fn admitted_workloads_keep_every_subworkload_within_its_requested_size() {
        let shortcuts: u64 = kani::any();
        let folders: u64 = kani::any();
        let result = Workload::new(shortcuts, folders, kani::any());
        kani::assert(
            result.is_ok()
                == (shortcuts > 0
                    && shortcuts <= super::MOST_SHORTCUTS
                    && folders <= super::MOST_FOLDERS),
            "only bounded nonempty workloads pass",
        );
        if let Ok(workload) = result {
            kani::assert(
                workload.shortcuts() == shortcuts && workload.folders() == folders,
                "counts preserve their meaning",
            );
            kani::assert(
                workload.imported() <= shortcuts
                    && workload.imported() <= super::MOST_IMPORTED
                    && workload.removed() <= shortcuts
                    && workload.removed() <= super::MOST_REMOVED,
                "smoke runs never expand into a full import or removal",
            );
        }
        kani::cover!(result.is_ok(), "an admitted workload");
        kani::cover!(result.is_err(), "an invalid workload");
    }
    fn namespace(tag: u8) -> Namespace {
        match tag {
            0 => Namespace::Folder,
            1 => Namespace::Shortcut,
            2 => Namespace::Imported,
            3 => Namespace::RemovalFolder,
            4 => Namespace::RemovalShortcut,
            _ => Namespace::Added,
        }
    }
    #[kani::proof]
    #[kani::unwind(17)]
    fn synthetic_identifiers_never_alias_across_indices_or_namespaces() {
        let first = namespace(kani::any());
        let second = namespace(kani::any());
        let left: u64 = kani::any();
        let right: u64 = kani::any();
        let equal = first.id(left, kani::any()) == second.id(right, kani::any());
        kani::assert(
            !equal || (first == second && left == right),
            "indices and namespaces remain distinct regardless of timestamps",
        );
        kani::cover!(first != second && !equal, "different namespaces");
        kani::cover!(left != right && !equal, "different indices");
    }
    #[kani::proof]
    fn resource_limits_stop_before_the_next_reserved_write() {
        let facts = Resources {
            synthetic_bytes: kani::any(),
            available_bytes: kani::any(),
            elapsed_millis: kani::any(),
            cancelled: kani::any(),
        };
        let result = super::permission(facts);
        let fits = !facts.cancelled
            && facts.elapsed_millis < super::MOST_MILLIS
            && facts.synthetic_bytes as u128 + super::WRITE_RESERVE as u128
                <= super::MOST_BYTES as u128
            && facts.available_bytes as u128
                >= super::LEAST_FREE as u128 + super::WRITE_RESERVE as u128;
        kani::assert(
            (result == Permission::Continue) == fits,
            "continuing requires every bound including the next write reserve",
        );
        kani::cover!(result == Permission::Continue, "a bounded run continues");
        kani::cover!(
            result == Permission::Stop(Stop::Cancelled),
            "cancellation stops"
        );
        kani::cover!(
            result == Permission::Stop(Stop::Deadline),
            "the deadline stops"
        );
        kani::cover!(result == Permission::Stop(Stop::Storage), "storage stops");
        kani::cover!(
            result == Permission::Stop(Stop::FreeSpace),
            "low free space stops"
        );
    }
    #[kani::proof]
    fn rejects_a_campaign_after_its_deadline() {
        let result = super::permission(Resources {
            synthetic_bytes: 0,
            available_bytes: u64::MAX,
            elapsed_millis: super::MOST_MILLIS,
            cancelled: false,
        });
        kani::assert(result == Permission::Continue, "NEGATIVE_BENCH_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::{Namespace, Permission, Resources, Workload};
    #[test]
    fn a_smoke_workload_scales_its_import_and_removal_and_keeps_its_seed() {
        let workload = Workload::new(128, 4, 17).unwrap();
        assert_eq!(
            (workload.imported(), workload.removed(), workload.seed()),
            (128, 128, 17)
        );
        assert_eq!(Workload::new(0, 0, 1), Err(super::Invalid::Shortcuts));
        assert_eq!(
            Workload::new(super::MOST_SHORTCUTS + 1, 0, 1),
            Err(super::Invalid::Shortcuts)
        );
        assert_eq!(
            Workload::new(1, super::MOST_FOLDERS + 1, 1),
            Err(super::Invalid::Folders)
        );
    }
    #[test]
    fn resource_boundaries_reserve_the_next_batch_and_stop_at_the_deadline() {
        let edge = Resources {
            synthetic_bytes: super::MOST_BYTES - super::WRITE_RESERVE,
            available_bytes: super::LEAST_FREE + super::WRITE_RESERVE,
            elapsed_millis: super::MOST_MILLIS - 1,
            cancelled: false,
        };
        assert_eq!(super::permission(edge), Permission::Continue);
        assert_eq!(
            super::permission(Resources {
                synthetic_bytes: edge.synthetic_bytes + 1,
                ..edge
            }),
            Permission::Stop(super::Stop::Storage)
        );
        assert_eq!(
            super::permission(Resources {
                available_bytes: edge.available_bytes - 1,
                ..edge
            }),
            Permission::Stop(super::Stop::FreeSpace)
        );
        assert_eq!(
            super::permission(Resources {
                elapsed_millis: super::MOST_MILLIS,
                ..edge
            }),
            Permission::Stop(super::Stop::Deadline)
        );
        assert_eq!(
            super::permission(Resources {
                cancelled: true,
                ..edge
            }),
            Permission::Stop(super::Stop::Cancelled)
        );
        assert_ne!(
            Namespace::Shortcut.id(u64::MAX, 0),
            Namespace::Imported.id(u64::MAX, 0)
        );
    }
}
