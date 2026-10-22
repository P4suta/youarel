use super::listing::Laid;
use super::{Library, checked_place, entry_id, entry_of, listable, selection_shown};
use crate::intent::Extend;
use anyhow::Result;
use std::sync::Arc;
use youarel_core::library_rules::{Layout, Move, grouped_moved};
use youarel_core::selection_rules::{Pinned, Selection, Span, Version};
use youarel_core::state::{EditError, ItemId};
use youarel_core::typeahead_rules;
use youarel_store::{Session, StoreError};

impl Library {
    pub(super) fn select_ids(&mut self, ids: impl IntoIterator<Item = ItemId>) {
        self.selection = Selection::of(ids);
        self.pins = None;
        self.settled_generation = None;
    }

    pub(super) fn set_selection(
        &mut self,
        session: &Session,
        laid: &Laid,
        selection: Selection<ItemId>,
    ) -> Result<(), StoreError> {
        let pins = match selection.span() {
            Span::None => None,
            Span::Places { .. } | Span::All(_) => {
                let entries = super::jobs::selected_from(session, laid, &selection)?;
                Some(Pinned::new(
                    Version {
                        listing: laid.shown.fingerprint(),
                        generation: session.generation(),
                    },
                    entries.into_iter().map(entry_id),
                ))
            }
        };
        self.selection = selection;
        self.pins = pins;
        self.settled_generation = Some(session.generation());
        Ok(())
    }

    pub(super) fn restore_pins(&mut self, version: Version) {
        if let Some(pins) = self.pins.take_if(|pins| !pins.keeps(version)) {
            self.selection = pins.into_selection();
        }
    }

    pub(super) fn refresh_pins(&mut self, generation: u64) {
        if let Some(version) = self.pins.as_ref().map(Pinned::version) {
            self.restore_pins(Version {
                generation,
                ..version
            });
        }
    }
    pub(super) fn selection_laid(&mut self, session: &Session) -> Result<Arc<Laid>, StoreError> {
        if self.layout != Layout::Columns {
            return self.main_laid(session);
        }
        let (columns, _) = self.walk(session)?;
        let laid = columns
            .get(self.path.len().saturating_sub(1))
            .cloned()
            .ok_or(StoreError::Missing("the selected column"))?;
        self.restore_pins(Version {
            listing: laid.shown.fingerprint(),
            generation: session.generation(),
        });
        Ok(laid)
    }

    pub(super) fn select_in(
        &mut self,
        session: &Session,
        laid: &Laid,
        (id, extend, hint): (ItemId, Extend, Option<usize>),
    ) -> Result<(), StoreError> {
        let place = checked_place(session, laid, entry_of(session.folders(), id), hint)?;
        if place.is_none() && !listable(session, self.place, id)? {
            return Err(StoreError::Edit(EditError::Unknown(entry_of(
                session.folders(),
                id,
            ))));
        }
        let shown = selection_shown(laid);
        let mut selection = self.selection.clone();
        match (extend, place) {
            (Extend::Replace, _) | (Extend::Toggle | Extend::Range, None) => {
                selection = Selection::of([id]);
                self.anchor = Some(id);
            }
            (Extend::Toggle, Some(place)) => {
                selection.toggle(shown, place, id);
                self.anchor = Some(id);
            }
            (Extend::Range, Some(place)) => {
                let anchor = self.range_anchor(session, laid)?.unwrap_or(place);
                selection = Selection::between(shown.listing, anchor, place);
            }
        }
        self.set_selection(session, laid, selection)?;
        self.focus = Some(id);
        self.focus_place = place;
        Ok(())
    }

    pub(super) fn extend_column(&mut self, session: &Session, key: Move) -> Result<(), StoreError> {
        let laid = self.selection_laid(session)?;
        let current = self.focus_place(session, &laid)?;
        if let Some(place) = grouped_moved(current, &laid.sizes(), 1, key)
            && let Some(row) = laid.row(session, place)?
        {
            let depth = self.path.len().saturating_sub(1);
            self.select_in(
                session,
                &laid,
                (entry_id(row.entry), Extend::Range, Some(place)),
            )?;
            self.path.truncate(depth);
            self.path.push((row.entry, place));
            self.revealing = true;
        }
        Ok(())
    }

    pub(super) fn type_ahead(&mut self, key: char) -> Result<Option<String>> {
        let typed = typeahead_rules::typed(self.typed.as_ref(), key, youarel_cli::clock::now()?);
        self.typed = Some(typed.clone());
        self.reading(|library, session| {
            let laid = library.selection_laid(session)?;
            if let Some(entry) = laid.starting(session, &typed.letters)? {
                let id = entry_id(entry);
                library.select_in(session, &laid, (id, Extend::Replace, None))?;
                if library.layout == Layout::Columns {
                    let place = library.focus_place.unwrap_or_default();
                    library.path.truncate(library.path.len().saturating_sub(1));
                    library.path.push((entry, place));
                }
                library.revealing = true;
            }
            Ok(None)
        })
    }
}
