use super::{Answer, Library, Place};
use crate::intent::{TargetName, item, text};
use crate::view::DragMaterial;
use anyhow::{Context as _, Result, bail};
use youarel_cli::clock::Stopwatch;
use youarel_core::library_rules::{self, Scope};
use youarel_core::state::{EditError, Entry, ItemId};
use youarel_link::Link;
use youarel_store::{Session, StoreError};

#[derive(Debug)]
pub(super) struct Drag {
    token: ItemId,
    entries: Vec<Entry>,
    hover: Option<Hover>,
}

#[derive(Clone, Copy, Debug)]
struct Hover {
    folder: ItemId,
    began: Stopwatch,
}

impl Library {
    pub(super) fn target_entries(
        &mut self,
        session: &Session,
        target: &TargetName,
    ) -> Result<Vec<Entry>> {
        match target {
            TargetName::Selection => Ok(self.selected(session)?),
            TargetName::Ids { ids } => {
                Ok(super::kinds(session.folders(), &crate::intent::items(ids)?))
            }
            TargetName::Drag { token } => {
                let token = item(token)?;
                Ok(self
                    .drag
                    .as_ref()
                    .filter(|drag| drag.token == token)
                    .context("the drag is no longer active")?
                    .entries
                    .clone())
            }
        }
    }

    /// Freezes a drag's entries and admitted links before the browser's synchronous dragstart event.
    ///
    /// # Errors
    ///
    /// Returns an error when the target, store, or one of the links is invalid.
    pub fn prepare_drag(&mut self, target: &TargetName) -> Result<DragMaterial> {
        self.reading(|library, session| {
            let entries = library.target_entries(session, target)?;
            if entries.is_empty() {
                bail!("select an item to drag");
            }
            let mut links = Vec::new();
            for entry in &entries {
                if let Entry::Shortcut(id) = entry
                    && let Some(shortcut) = session.item(*id)?
                {
                    links.push(Link::parse(&shortcut.link.value)?.as_str().to_owned());
                }
            }
            let token = session.fresh_id()?;
            let material = DragMaterial {
                token: text(token),
                links: links.join("\r\n"),
                count: entries.len(),
            };
            library.drag = Some(Drag {
                token,
                entries,
                hover: None,
            });
            Ok(material)
        })
    }

    /// Starts or cancels a hover of the active drag, answering the delay for a valid destination.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot validate the destination.
    pub fn hover(&mut self, token: ItemId, folder: Option<ItemId>) -> Result<Option<u64>> {
        self.reading(|library, session| {
            let Some(drag) = library.drag.as_mut().filter(|drag| drag.token == token) else {
                return Ok(None);
            };
            drag.hover = None;
            let Some(folder) = folder else {
                return Ok(None);
            };
            match super::jobs::admitted(session.folders(), folder, &drag.entries) {
                Ok(()) => {}
                Err(StoreError::Edit(EditError::Unknown(_) | EditError::IntoItself(_))) => {
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            }
            drag.hover = Some(Hover {
                folder,
                began: Stopwatch::start(),
            });
            Ok(Some(library_rules::SPRING_DELAY))
        })
    }

    pub(super) fn end_drag(&mut self, token: ItemId) -> Option<String> {
        if self.drag.as_ref().is_some_and(|drag| drag.token == token) {
            self.drag = None;
        }
        None
    }

    pub(super) fn spring(&mut self, token: ItemId, folder: ItemId) -> Result<Answer> {
        let elapsed = self
            .drag
            .as_ref()
            .and_then(|drag| drag.hover)
            .map_or(Ok(0), |hover| hover.began.elapsed())?;
        self.spring_after(token, folder, elapsed).map(Answer::Said)
    }

    pub(super) fn spring_after(
        &mut self,
        token: ItemId,
        folder: ItemId,
        elapsed: u64,
    ) -> Result<Option<String>> {
        let active = self.drag.as_ref().and_then(|drag| {
            drag.hover.map(|hover| library_rules::Hover {
                token: drag.token,
                folder: hover.folder,
            })
        });
        if !library_rules::spring_due(active, library_rules::Hover { token, folder }, elapsed) {
            return Ok(None);
        }
        self.drag.as_mut().context("the active drag ended")?.hover = None;
        self.reading(|library, session| {
            super::jobs::admitted(
                session.folders(),
                folder,
                &library.drag.as_ref().context("the drag ended")?.entries,
            )?;
            library.choose_place(Place::Scope(Scope::Collection(folder)));
            Ok(None)
        })
    }
}
