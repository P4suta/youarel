use super::{Library, web};
use crate::view::{Notice, View};
use anyhow::{Context as _, Result};
use std::path::Path;
use youarel_cli::{clock, commands};
use youarel_core::state::{Entry, ItemId};
use youarel_store::fs;

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

    /// Exports the live library as a new Netscape bookmark file in the chosen directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the clock, store, directory, or new file cannot be read or written.
    pub fn export_to(&mut self, directory: &Path) -> Result<View> {
        let now = clock::now()?;
        let html = commands::export(&self.store, now, commands::Format::Html)?;
        let path = directory.join(format!("youarel-bookmarks-{now}.html"));
        fs::write_new(&path, html.as_bytes())?;
        self.notice = Some(Notice {
            text: format!("Exported {}", path.display()),
            error: false,
        });
        self.view()
    }
}
