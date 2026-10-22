//! The moves, the moves to the trash, the put backs, and the deletions the window asks for, which run in batches of [`BATCH`] edits, so that a job of a million rows leaves the window free between its batches.
//!
//! A job reads what it acts on once, as the window showed it, and passes over what went since or stands no longer where the job finds it.

use super::history::{Step, WorkHistory};
use super::listing::{Laid, Shown};
use super::{Library, entry_id, entry_of, kinds, selection_shown};
use crate::intent::TargetName;
use crate::view::{Notice, PATH_SEPARATOR, Progress, View};
use anyhow::Result;
use std::collections::BTreeSet;
use youarel_core::library_rules::Layout;
use youarel_core::selection_rules::{Selection, Span};
use youarel_core::state::{Edit, EditError, Entry, ItemId, Presence, Slot};
use youarel_store::folders::Folders;
use youarel_store::{BATCH, Session, StoreError};

/// What a job does to each of its entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Work {
    /// Moves each entry to the end of a collection, or to the top.
    File(Option<ItemId>),
    Paste(Option<ItemId>),
    /// Moves each entry to the trash, where a folder hides everything inside it.
    Trash,
    /// Puts each entry in the trash back where it was.
    PutBack,
    /// Deletes each entry for good, a folder with everything inside it.
    Remove,
    /// Deletes for good each entry that still waits in the trash, so one put back meanwhile stays.
    Empty,
}

/// What the window says once the trash is empty.
const EMPTIED: &str = "Emptied the Trash";
/// How many of a job's entries the window looks for in the listing it shows, for the row that takes the selection after them.
const PLACED: usize = 8;

impl Work {
    fn admit(self, folders: &Folders, entries: &[Entry]) -> Result<(), StoreError> {
        match self {
            Self::File(Some(folder)) | Self::Paste(Some(folder)) => {
                admitted(folders, folder, entries)
            }
            Self::File(None)
            | Self::Paste(None)
            | Self::Trash
            | Self::PutBack
            | Self::Remove
            | Self::Empty => Ok(()),
        }
    }
    const fn history(self) -> WorkHistory {
        match self {
            Self::File(_) | Self::Paste(_) => WorkHistory::Remember(Step::new("Move", Vec::new())),
            Self::Trash => WorkHistory::Remember(Step::new("Move to Trash", Vec::new())),
            Self::PutBack => WorkHistory::Remember(Step::new("Put Back", Vec::new())),
            Self::Remove | Self::Empty => WorkHistory::Forget,
        }
    }
    /// The edit of one entry.
    const fn edit(self, entry: Entry) -> Edit {
        match self {
            Self::File(folder) | Self::Paste(folder) => Edit::Move {
                entry,
                folder,
                slot: Slot::End,
            },
            Self::Trash => Edit::Trash { entry },
            Self::PutBack => Edit::PutBack { entry },
            Self::Remove | Self::Empty => Edit::Remove { entry },
        }
    }

    /// Whether the work takes an entry of a presence: a live one for a move or the trash, one in the trash for a put back or an emptying, and any not gone for good for a deletion.
    const fn takes(self, presence: Presence) -> bool {
        match self {
            Self::File(_) | Self::Paste(_) | Self::Trash => matches!(presence, Presence::Live),
            Self::PutBack | Self::Empty => matches!(presence, Presence::Trashed),
            Self::Remove => matches!(presence, Presence::Live | Presence::Trashed),
        }
    }

    /// Whether the work takes its entries out of the listing the window shows, so the row after them takes the selection, as an explorer selects the next item after a deletion.
    const fn clears(self) -> bool {
        matches!(self, Self::Trash | Self::PutBack | Self::Remove)
    }

    /// What the window says while the job works.
    const fn doing(self) -> &'static str {
        match self {
            Self::File(_) | Self::Paste(_) => "Moving",
            Self::Trash => "Moving to the Trash",
            Self::PutBack => "Putting back",
            Self::Remove | Self::Empty => "Deleting",
        }
    }
}

/// How many folders and shortcuts a job carried out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Done {
    folders: usize,
    shortcuts: usize,
}

impl Done {
    const fn count(&mut self, entry: Entry) {
        match entry {
            Entry::Folder(_) => self.folders = self.folders.saturating_add(1),
            Entry::Shortcut(_) => self.shortcuts = self.shortcuts.saturating_add(1),
        }
    }

    const fn any(self) -> bool {
        self.folders > 0 || self.shortcuts > 0
    }

    const fn and(self, other: Self) -> Self {
        Self {
            folders: self.folders.saturating_add(other.folders),
            shortcuts: self.shortcuts.saturating_add(other.shortcuts),
        }
    }
}

/// How many shortcuts, in words.
pub(super) fn shortcuts(count: usize) -> String {
    if count == 1 {
        "1 shortcut".to_owned()
    } else {
        format!("{count} shortcuts")
    }
}

/// How many folders and shortcuts, in words.
fn described(done: Done) -> String {
    let folders = if done.folders == 1 {
        "1 folder".to_owned()
    } else {
        format!("{} folders", done.folders)
    };
    match (done.folders, done.shortcuts) {
        (0, count) => shortcuts(count),
        (_, 0) => folders,
        (_, count) => format!("{folders} and {}", shortcuts(count)),
    }
}

/// A job that runs in batches: the entries it acts on in the order the window showed them, how far it came, and what stopped it.
#[derive(Debug)]
pub struct Job {
    work: Work,
    history: WorkHistory,
    entries: Vec<Entry>,
    /// How many entries one batch carries out.
    batch: usize,
    /// How many entries the batches so far went through.
    passed: usize,
    done: Done,
    /// The path of the collection a move files into, which its notice names.
    destination: String,
    /// Whether the job acts on a selection of places, which lets go once the job ends, since its places hold other rows then.
    releases: bool,
    /// The first place of the listing shown that the job's entries held, whose row takes the selection once they left the listing.
    after: Option<usize>,
    failed: Option<anyhow::Error>,
}

impl Job {
    /// The job in batches of another size, which a test sets to see a job of a few entries run in several.
    #[cfg(test)]
    pub(super) const fn in_batches_of(mut self, batch: usize) -> Self {
        self.batch = batch;
        self
    }

    /// What the notice says once the job ended: what it did, or what stopped it and how far it came.
    fn notice(&self) -> Notice {
        let total = self.entries.len();
        match (&self.failed, self.work) {
            (Some(error), _) if self.passed > 0 => Notice {
                text: format!("Stopped after {} of {total}: {error:#}", self.passed),
                error: true,
            },
            (Some(error), _) => Notice {
                text: format!("{error:#}"),
                error: true,
            },
            (None, Work::Trash) => Notice {
                text: format!("Moved {} to the Trash", described(self.done)),
                error: false,
            },
            (None, Work::PutBack) => Notice {
                text: format!("Put back {}", described(self.done)),
                error: false,
            },
            (None, Work::Remove) => Notice {
                text: format!("Deleted {}", described(self.done)),
                error: false,
            },
            (None, Work::Empty) => Notice {
                text: EMPTIED.to_owned(),
                error: false,
            },
            (None, Work::File(_) | Work::Paste(_)) => {
                // A folder at the top stands in no collection as a shortcut there does, yet reads better as at the top.
                let place = match (self.destination.is_empty(), self.done.folders > 0) {
                    (false, _) => self.destination.clone(),
                    (true, true) => "the top".to_owned(),
                    (true, false) => "no collection".to_owned(),
                };
                Notice {
                    text: format!("Moved {} to {place}", described(self.done)),
                    error: false,
                }
            }
        }
    }
}

/// What an intent began: a view it answered with at once, or a job that runs in batches.
#[derive(Debug)]
pub enum Begun {
    Answered(Box<View>),
    Working(Box<Job>),
}

/// Refuses a move into a folder that is gone, or of a folder into itself or into a folder it holds, before any batch runs.
pub(super) fn admitted(
    folders: &Folders,
    destination: ItemId,
    entries: &[Entry],
) -> Result<(), StoreError> {
    if !folders.shows(destination) {
        return Err(StoreError::Edit(EditError::Unknown(Entry::Folder(
            destination,
        ))));
    }
    let lineage = folders.tree().lineage(Some(destination));
    for entry in entries {
        if let Entry::Folder(id) = entry
            && lineage.contains(id)
        {
            return Err(StoreError::Edit(EditError::IntoItself(*id)));
        }
    }
    Ok(())
}

/// Whether an entry still stands where a work takes it, which a job passes over once it went or moved in or out of the trash since.
fn standing(session: &Session, entry: Entry, work: Work) -> Result<bool, StoreError> {
    Ok(session
        .presence_of(entry)?
        .is_some_and(|presence| work.takes(presence)))
}

/// The entries of a listing in a window of places that a selection takes, in their order.
fn taken(
    session: &Session,
    laid: &Laid,
    selection: &Selection<ItemId>,
) -> Result<(Vec<Entry>, BTreeSet<ItemId>), StoreError> {
    let shown = selection_shown(laid);
    let window = match selection.span() {
        Span::Places { from, to, .. } => from..to.saturating_add(1),
        Span::None | Span::All(_) => 0..laid.total(),
    };
    let mut found = Vec::new();
    let mut met = BTreeSet::new();
    laid.each(session, window, |place, entry| {
        let id = entry_id(entry);
        if selection.contains(shown, place, id) {
            found.push(entry);
            if selection.added().binary_search(&id).is_ok() {
                met.insert(id);
            }
        }
    })?;
    Ok((found, met))
}

pub(super) fn selected_from(
    session: &Session,
    laid: &Laid,
    selection: &Selection<ItemId>,
) -> Result<Vec<Entry>, StoreError> {
    let (mut found, met) = taken(session, laid, selection)?;
    found.extend(
        selection
            .added()
            .iter()
            .filter(|id| !met.contains(id))
            .map(|id| entry_of(session.folders(), *id)),
    );
    Ok(found)
}

impl Library {
    /// The entries the selection takes, in the order the window shows them, then the ones it adds from outside the listing.
    pub(super) fn selected(&mut self, session: &Session) -> Result<Vec<Entry>, StoreError> {
        let folders = session.folders();
        if self.selection.span() == Span::None {
            return Ok(kinds(folders, self.selection.added()));
        }
        let laid = self.selection_laid(session)?;
        selected_from(session, &laid, &self.selection)
    }

    /// The first place of the listing the cards or the list show that the entries a job acts on hold, which a selection of places names at once and the first few entries name otherwise.
    fn first_place(
        &mut self,
        session: &Session,
        entries: &[Entry],
    ) -> Result<Option<usize>, StoreError> {
        if !matches!(self.layout, Layout::Cards | Layout::List) {
            return Ok(None);
        }
        let laid = self.main_laid(session)?;
        let shown = selection_shown(&laid).listing;
        match self.selection.span() {
            Span::Places { listing, from, .. } if listing == shown => return Ok(Some(from)),
            Span::All(listing) if listing == shown => return Ok(Some(0)),
            Span::None | Span::Places { .. } | Span::All(_) => {}
        }
        if entries.len() > PLACED {
            return laid.first_of(session, entries);
        }
        let mut first: Option<usize> = None;
        for entry in entries.iter().take(PLACED) {
            if let Some(place) = laid.place(session, *entry)? {
                first = Some(first.map_or(place, |held| held.min(place)));
            }
        }
        Ok(first)
    }

    /// Begins a job: reads what it acts on now, and refuses a move of a folder into itself before any batch runs.
    pub(super) fn job(&mut self, target: &TargetName, work: Work) -> Result<Job> {
        self.reading(|library, session| {
            let entries = library.target_entries(session, target)?;
            let releases =
                matches!(target, TargetName::Selection) && library.selection.span() != Span::None;
            let after = if work.clears() {
                library.first_place(session, &entries)?
            } else {
                None
            };
            let folders = session.folders();
            let destination = match work {
                Work::File(Some(folder)) | Work::Paste(Some(folder)) => {
                    admitted(folders, folder, &entries)?;
                    folders.path(Some(folder)).join(PATH_SEPARATOR)
                }
                Work::File(None)
                | Work::Paste(None)
                | Work::Trash
                | Work::PutBack
                | Work::Remove
                | Work::Empty => String::new(),
            };
            Ok(Job {
                work,
                history: work.history(),
                entries,
                batch: BATCH,
                passed: 0,
                done: Done::default(),
                destination,
                releases,
                after,
                failed: None,
            })
        })
    }

    /// Begins the job that deletes for good what waits in the trash itself: its folders, then its shortcuts, each that still waits there once its batch runs.
    pub(super) fn empty_trash(&mut self) -> Result<Job> {
        self.reading(|library, session| {
            let mut entries: Vec<Entry> = session
                .folders()
                .trashed()
                .into_iter()
                .map(Entry::Folder)
                .collect();
            entries.extend(
                session
                    .trash_page(0, usize::MAX)?
                    .into_iter()
                    .map(Entry::Shortcut),
            );
            Ok(Job {
                work: Work::Empty,
                history: WorkHistory::Forget,
                entries,
                batch: BATCH,
                passed: 0,
                done: Done::default(),
                destination: String::new(),
                releases: matches!(library.main_shown(), Shown::Trash { .. }),
                after: None,
                failed: None,
            })
        })
    }

    /// Selects the row at a place of the listing the cards or the list show, or the last row when the listing ends before it, as an explorer selects the item after the ones that left.
    fn select_at(&mut self, session: &Session, place: usize) -> Result<(), StoreError> {
        let laid = self.main_laid(session)?;
        let row = match laid.total().checked_sub(1) {
            Some(last) => laid.row(session, place.min(last))?,
            None => None,
        };
        let Some(row) = row else {
            self.select_ids([]);
            self.focus = None;
            self.focus_place = None;
            self.anchor = None;
            return Ok(());
        };
        let id = entry_id(row.entry);
        self.focus = Some(id);
        self.anchor = Some(id);
        self.focus_place = Some(place.min(laid.total().saturating_sub(1)));
        self.select_ids([id]);
        self.revealing = true;
        Ok(())
    }

    /// Carries out the next batch of a job in one session, and answers how far the job came, or none once it ended or a batch failed.
    pub fn advance(&mut self, job: &mut Job) -> Option<Progress> {
        if job.failed.is_some() {
            return None;
        }
        let total = job.entries.len();
        let end = job.passed.saturating_add(job.batch).min(total);
        let batch = job
            .entries
            .get(job.passed..end)
            .filter(|batch| !batch.is_empty())?;
        let work = job.work;
        let remembers = matches!(job.history, WorkHistory::Remember(_));
        let outcome = self.session(|session| {
            work.admit(session.folders(), batch)?;
            let mut done = Done::default();
            let mut changes = Vec::new();
            for entry in batch {
                if standing(session, *entry, work)? {
                    if remembers {
                        changes.push(session.edit_remembered(work.edit(*entry))?);
                    } else {
                        session.edit(work.edit(*entry))?;
                    }
                    done.count(*entry);
                }
            }
            Ok((done, session.now(), changes))
        });
        match outcome {
            Ok((done, now, changes)) => {
                job.history.append(changes);
                job.done = job.done.and(done);
                job.passed = end;
                if done.any() {
                    self.syncing.edited(now);
                }
                Some(Progress {
                    done: end,
                    total,
                    text: format!("{} {end} of {total}", work.doing()),
                })
            }
            Err(error) => {
                job.failed = Some(error);
                None
            }
        }
    }

    /// Ends a job: the notice says what it did or what stopped it, a selection of places it acted on lets go, and the row after the entries that left the listing takes the selection.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn finish(&mut self, job: &mut Job) -> Result<View> {
        if matches!(job.work, Work::Paste(_)) && job.failed.is_none() {
            self.cut.clear();
        }
        if job.done.any() {
            self.finish_history(core::mem::replace(&mut job.history, WorkHistory::Finished));
        }
        if job.releases {
            self.select_ids([]);
        }
        if let Some(place) = job.after.filter(|_| job.done.any()) {
            self.reading(|library, session| Ok(library.select_at(session, place)?))?;
        }
        self.notice = Some(job.notice());
        self.view()
    }
}
