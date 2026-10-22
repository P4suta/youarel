use super::{BUDGET_CHECK_RECORDS, Campaign, Measurement, NOW, data::Generator};
use anyhow::{Result, ensure};
use youarel_core::bench_rules::Namespace;
use youarel_core::record_rules::Record;
use youarel_core::state::{Edit, Entry, ItemId, Slot};

const ADDED_LINK: &str = "https://added.example/benchmark";

impl Campaign {
    pub(super) fn own_edits(&self, folders: &[ItemId]) -> Result<()> {
        let source = self.store("source");
        let id = Namespace::Added.id(1, NOW);
        self.measure(Measurement::operation("add", 1), |metric| {
            source.session(NOW, |session| {
                session.edit(Edit::AddShortcut {
                    id,
                    link: ADDED_LINK.to_owned(),
                    title: "Added benchmark shortcut".to_owned(),
                    folder: None,
                })
            })?;
            metric.rows = 1;
            Ok(())
        })?;
        self.measure(Measurement::operation("move", 1), |metric| {
            source.session(NOW, |session| {
                session.edit(Edit::Move {
                    entry: Entry::Shortcut(id),
                    folder: folders.first().copied(),
                    slot: Slot::End,
                })
            })?;
            metric.rows = 1;
            Ok(())
        })
    }

    pub(super) fn removal(&self) -> Result<()> {
        let store = self.store("remove");
        let mut generator = Generator::new(self.options.workload.seed());
        let folder = generator.folder(Namespace::RemovalFolder, 1, None)?;
        let id = match &folder {
            Record::Folder(id, _folder) => *id,
            Record::Shortcut(..) | Record::Device(..) => {
                anyhow::bail!("the removal fixture has no folder")
            }
        };
        let mut records = vec![folder];
        for number in 1..=self.options.workload.removed() {
            if usize::try_from(number)?.is_multiple_of(BUDGET_CHECK_RECORDS) {
                self.guard.check()?;
            }
            records.push(generator.shortcut(Namespace::RemovalShortcut, number, Some(id))?);
        }
        store.session(NOW, |session| {
            for record in records {
                session.merge_record(record)?;
            }
            Ok(())
        })?;
        self.measure(
            Measurement::operation("remove_folder", self.options.workload.removed()),
            |metric| {
                store.session(NOW, |session| {
                    session.edit(Edit::Remove {
                        entry: Entry::Folder(id),
                    })
                })?;
                metric.rows = usize::try_from(self.options.workload.removed())?;
                Ok(())
            },
        )?;
        ensure!(
            store.session(NOW, |session| session.home_count(Some(id)))? == 0,
            "folder removal left visible fixture shortcuts"
        );
        Ok(())
    }

    pub(super) fn import(&self) -> Result<()> {
        let store = self.store("import");
        let mut generator = Generator::new(self.options.workload.seed());
        let mut records = Vec::new();
        for number in 1..=self.options.workload.imported() {
            if usize::try_from(number)?.is_multiple_of(BUDGET_CHECK_RECORDS) {
                self.guard.check()?;
            }
            records.push(generator.shortcut(Namespace::Imported, number, None)?);
        }
        self.measure(
            Measurement::operation("record_import", self.options.workload.imported()),
            |metric| {
                store.session(NOW, |session| {
                    for (index, record) in records.into_iter().enumerate() {
                        if index.is_multiple_of(BUDGET_CHECK_RECORDS) {
                            self.guard.publication()?;
                        }
                        session.merge_record(record)?;
                    }
                    Ok(())
                })?;
                metric.rows = usize::try_from(self.options.workload.imported())?;
                Ok(())
            },
        )?;
        ensure!(
            u64::try_from(store.session(NOW, |session| session.home_count(None))?)?
                == self.options.workload.imported(),
            "the record import did not keep every shortcut"
        );
        Ok(())
    }
}
