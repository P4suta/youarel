use super::{Library, web};
use anyhow::{Context as _, Result};
use youarel_core::state::{Entry, ItemId};

impl Library {
    pub(super) fn open(&mut self, id: Option<ItemId>) -> Result<Option<String>> {
        self.open_with(id, &web::open)
    }

    pub(super) fn open_with(
        &mut self,
        chosen: Option<ItemId>,
        opener: &impl Fn(&str) -> Result<()>,
    ) -> Result<Option<String>> {
        if chosen.is_some() {
            return self.open_one(chosen, opener);
        }
        let entries = self.reading(|library, session| Ok(library.selected(session)?))?;
        if entries.len() <= 1 {
            return self.open_one(chosen, opener);
        }
        let ids: Vec<ItemId> = entries
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Shortcut(id) => Some(id),
                Entry::Folder(_) => None,
            })
            .collect();
        for (opened, id) in ids.iter().enumerate() {
            self.open_one(Some(*id), opener)
                .with_context(|| format!("completed {opened} of {} opens", ids.len()))?;
        }
        Ok(Some(format!(
            "Opened {}",
            super::jobs::shortcuts(ids.len())
        )))
    }
}
