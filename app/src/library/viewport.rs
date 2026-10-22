use super::{Library, entry_of};
use crate::intent::item;
use crate::view::{AnchoredRow, View, ViewportAnchor};
use anyhow::Result;
use youarel_core::library_rules::Layout;
use youarel_store::Session;

struct Position {
    column: Option<usize>,
    id: String,
    place: usize,
}

impl Library {
    fn anchor_positions(
        &mut self,
        session: &Session,
        anchors: &[ViewportAnchor],
    ) -> Result<Vec<Position>> {
        let columns = if self.layout == Layout::Columns {
            self.walk(session)?.0
        } else {
            Vec::new()
        };
        let main = if self.layout == Layout::Columns {
            None
        } else {
            Some(self.main_laid(session)?)
        };
        let mut positions = Vec::new();
        for anchor in anchors {
            let laid = anchor
                .column
                .map_or(main.as_ref(), |depth| columns.get(depth));
            let Some(laid) = laid.filter(|laid| laid.identity() == anchor.identity) else {
                continue;
            };
            let id = item(&anchor.id)?;
            if let Some(place) = laid.place(session, entry_of(session.folders(), id))? {
                if let Some(depth) = anchor.column {
                    self.keep_column_sight(depth, laid, place);
                } else {
                    self.viewport = place;
                }
                positions.push(Position {
                    column: anchor.column,
                    id: anchor.id.clone(),
                    place,
                });
            }
        }
        Ok(positions)
    }

    /// Refreshes the view around the same visible entries after a store update.
    ///
    /// # Errors
    ///
    /// Returns an error when an identifier is invalid or the store cannot be read.
    pub fn view_anchored(&mut self, anchors: &[ViewportAnchor]) -> Result<View> {
        self.reading(|library, session| {
            library.settle(session)?;
            let positions = library.anchor_positions(session, anchors)?;
            let mut view = library.build(session)?;
            for position in positions {
                if let Some(depth) = position.column {
                    if let Some(column) = view.columns.get_mut(depth) {
                        column.anchor = Some(AnchoredRow {
                            id: position.id,
                            place: position.place,
                        });
                    }
                } else {
                    view.listing.anchor = Some(AnchoredRow {
                        id: position.id,
                        place: position.place,
                    });
                }
            }
            Ok(view)
        })
    }
}
