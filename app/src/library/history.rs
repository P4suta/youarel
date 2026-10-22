use super::Library;
use anyhow::{Context, Result};
use youarel_core::history_rules::MOST_EDITS;
use youarel_core::state::Edit;
use youarel_core::undo_rules::Reversal;
use youarel_store::{Session, StoreError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Direction {
    Undo,
    Redo,
}

#[derive(Clone, Debug)]
pub(super) struct Step {
    pub(super) action: &'static str,
    changes: Vec<Reversal>,
}

impl Step {
    pub(super) const fn new(action: &'static str, changes: Vec<Reversal>) -> Self {
        Self { action, changes }
    }

    fn apply(&self, session: &mut Session, direction: Direction) -> Result<(), StoreError> {
        match direction {
            Direction::Undo => self
                .changes
                .iter()
                .rev()
                .try_for_each(|change| session.edit(change.undo.clone())),
            Direction::Redo => self
                .changes
                .iter()
                .try_for_each(|change| session.edit(change.redo.clone())),
        }
    }
}

#[derive(Debug)]
pub(super) enum WorkHistory {
    Remember(Step),
    Forget,
    Finished,
}

impl WorkHistory {
    pub(super) fn append(&mut self, mut changes: Vec<Reversal>) {
        if let Self::Remember(step) = self {
            if step.changes.len().saturating_add(changes.len()) > MOST_EDITS {
                *self = Self::Forget;
            } else {
                step.changes.append(&mut changes);
            }
        }
    }
}

impl Library {
    pub(super) fn remember(&mut self, action: &'static str, changes: Vec<Reversal>) {
        if !changes.is_empty() {
            let count = changes.len();
            self.history.record(Step::new(action, changes), count);
        }
    }

    pub(super) fn edit_remembered(&mut self, action: &'static str, edit: Edit) -> Result<()> {
        let change = self.session(|session| session.edit_remembered(edit))?;
        self.remember(action, vec![change]);
        Ok(())
    }

    pub(super) fn walk_history(&mut self, direction: Direction) -> Result<Option<String>> {
        let step = match direction {
            Direction::Undo => self.history.last(),
            Direction::Redo => self.history.next(),
        }
        .cloned();
        let Some(step) = step else { return Ok(None) };
        let now = self.session(|session| {
            step.apply(session, direction)?;
            Ok(session.now())
        })?;
        match direction {
            Direction::Undo => self.history.back(),
            Direction::Redo => self.history.forward(),
        }
        .context("the applied history step disappeared")?;
        self.syncing.edited(now);
        let verb = match direction {
            Direction::Undo => "Undid",
            Direction::Redo => "Redid",
        };
        Ok(Some(format!("{verb} {}", step.action)))
    }

    pub(super) fn finish_history(&mut self, history: WorkHistory) {
        match history {
            WorkHistory::Remember(step) => self.remember(step.action, step.changes),
            WorkHistory::Forget => self.history.clear(),
            WorkHistory::Finished => {}
        }
    }
}
