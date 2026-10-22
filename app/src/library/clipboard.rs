use super::{Answer, Library, Place, entry_id};
use crate::intent::TargetName;
use anyhow::{Result, bail};
use youarel_core::state::{Edit, Entry, ItemId};
use youarel_core::undo_rules::Reversal;
use youarel_link::Link;
use youarel_store::{Session, StoreError};

const MOST_PASTED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct Pasted {
    changes: Vec<Reversal>,
    last: Option<ItemId>,
    repeated: usize,
    ignored: usize,
}

impl Pasted {
    fn add(
        &mut self,
        session: &mut Session,
        words: &[Link],
        home: Option<ItemId>,
    ) -> Result<(), StoreError> {
        for link in words {
            if session.holder(home, link.as_str())?.is_some() {
                self.repeated = self.repeated.saturating_add(1);
                continue;
            }
            let id = session.fresh_id()?;
            self.changes
                .push(session.edit_remembered(Edit::AddShortcut {
                    id,
                    link: link.as_str().to_owned(),
                    title: link.as_str().to_owned(),
                    folder: home,
                })?);
            self.last = Some(id);
        }
        Ok(())
    }

    fn notice(&self) -> String {
        let mut parts = vec![format!(
            "Added {}",
            super::jobs::shortcuts(self.changes.len())
        )];
        if self.repeated > 0 {
            parts.push(format!("{} already here", self.repeated));
        }
        if self.ignored > 0 {
            parts.push(format!("{} ignored", self.ignored));
        }
        parts.join("; ")
    }
}

impl Library {
    pub(super) fn cut_selection(&mut self) -> Result<Option<String>> {
        self.reading(|library, session| {
            if library.place == Place::Trash {
                bail!("put items back before cutting them");
            }
            library.cut = library.selected(session)?.into_iter().collect();
            Ok(None)
        })
    }

    pub(super) fn paste(&mut self, text: &str) -> Result<Answer> {
        if self.place == Place::Trash {
            bail!("choose a folder before pasting");
        }
        if !self.cut.is_empty() {
            let target = TargetName::Ids {
                ids: self
                    .cut
                    .iter()
                    .map(|entry| crate::intent::text(entry_id(*entry)))
                    .collect(),
            };
            return self.begin_job(&target, super::jobs::Work::Paste(self.here()));
        }
        self.paste_text(text).map(Answer::Said)
    }

    fn paste_text(&mut self, text: &str) -> Result<Option<String>> {
        if text.len() > MOST_PASTED_BYTES {
            bail!("the pasted text exceeds 16 MiB");
        }
        let mut pasted = Pasted::default();
        let words: Vec<Link> = text
            .split_whitespace()
            .filter_map(|word| {
                Link::parse(word).map_or_else(
                    |_not_a_link| {
                        pasted.ignored = pasted.ignored.saturating_add(1);
                        None
                    },
                    Some,
                )
            })
            .collect();
        if words.is_empty() {
            bail!("the clipboard holds no http or https links");
        }
        let home = self.here();
        let now = self.session(|session| {
            pasted.add(session, &words, home)?;
            Ok(session.now())
        })?;
        let notice = pasted.notice();
        if !pasted.changes.is_empty() {
            self.syncing.edited(now);
            self.remember("Paste", pasted.changes);
        }
        if let Some(id) = pasted.last {
            self.focus_on(id);
            self.reading(|library, session| library.open_to(session, id))?;
        }
        Ok(Some(notice))
    }

    /// The selected shortcuts' admitted links, one per line, for the clipboard or a drag.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read or a stored link is invalid.
    pub fn selected_links(&mut self) -> Result<String> {
        self.reading(|library, session| {
            let mut links = Vec::new();
            for entry in library.selected(session)? {
                if let Entry::Shortcut(id) = entry
                    && let Some(shortcut) = session.item(id)?
                {
                    links.push(Link::parse(&shortcut.link.value)?.as_str().to_owned());
                }
            }
            Ok(links.join("\n"))
        })
    }

    /// Whether a paste moves the cut entries instead of reading clipboard text.
    #[must_use]
    pub fn cutting(&self) -> bool {
        !self.cut.is_empty()
    }

    pub(super) fn cancel(&mut self) -> Option<String> {
        self.cut.clear();
        self.drag = None;
        self.forget_selection();
        None
    }
}
