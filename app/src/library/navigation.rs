use super::{Laid, Library, Place, checked_place, entry_of, set};
use anyhow::{Context as _, Result};
use youarel_core::library_rules::Scope;
use youarel_core::state::{Entry, ItemId};
use youarel_store::{Session, StoreError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Visit {
    pub(super) before: Place,
    pub(super) after: Place,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Disclosure {
    Row,
    Subtree,
}

impl From<bool> for Disclosure {
    fn from(deep: bool) -> Self {
        if deep { Self::Subtree } else { Self::Row }
    }
}

impl Library {
    pub(super) fn back(&mut self) -> Option<String> {
        if let Some(visit) = self.places.back() {
            self.place = visit.before;
            self.forget_selection();
        }
        None
    }

    pub(super) fn forward(&mut self) -> Option<String> {
        if let Some(visit) = self.places.forward() {
            self.place = visit.after;
            self.forget_selection();
        }
        None
    }

    pub(super) fn range_anchor(
        &self,
        session: &Session,
        laid: &Laid,
    ) -> Result<Option<usize>, StoreError> {
        self.anchor.or(self.focus).map_or_else(
            || Ok(None),
            |id| checked_place(session, laid, entry_of(session.folders(), id), None),
        )
    }

    pub(super) fn show_in_folder(&mut self, id: Option<ItemId>) -> Result<Option<String>> {
        let id = id
            .or(self.focus)
            .context("select an item to show its folder")?;
        self.reading(|library, session| {
            let folders = session.folders();
            let home = match entry_of(folders, id) {
                Entry::Folder(id) if folders.shows(id) => folders.tree().parent(id),
                Entry::Folder(_) => anyhow::bail!("the folder is gone"),
                Entry::Shortcut(id) => {
                    let shortcut = session.item(id)?.context("the shortcut is gone")?;
                    folders.home(&shortcut)
                }
            };
            library.choose_place(Place::Scope(home.map_or(Scope::All, Scope::Collection)));
            library.search.clear();
            library.focus_on(id);
            library.anchor = Some(id);
            library.open_to(session, id)?;
            Ok(None)
        })
    }

    pub(super) fn disclose(
        &mut self,
        id: ItemId,
        open: bool,
        depth: Disclosure,
    ) -> Result<Option<String>> {
        if depth == Disclosure::Row {
            set(&mut self.outlined, id, open);
            return Ok(None);
        }
        self.reading(|library, session| {
            let tree = session.folders().tree();
            for (folder, _) in tree.folders() {
                if tree.within(folder, id) {
                    set(&mut library.outlined, folder, open);
                }
            }
            Ok(None)
        })
    }
}
