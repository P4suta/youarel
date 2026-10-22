//! A history of steps that a person takes back and takes again: the edits that undo and redo walk, and the places that back and forward walk.
//!
//! A history keeps its steps in the order they came and a present among them, with the steps behind the present before it and the steps ahead after it.
//! A new step clears the steps ahead, back moves the present one step toward the oldest, and forward moves it one step toward the newest.
//! A history keeps at most its bound of steps, so the oldest goes once a new step passes the bound.
//! A step that would hold more edits than [`MOST_EDITS`] clears the history instead, since an undo then would skip what the person did last.
//! [`pushed`], [`back_from`], and [`forward_from`] decide each act from where a history stands, and [`History`] carries their decisions out on its steps.

use alloc::vec::Vec;

/// The most steps a history keeps.
pub const MOST_STEPS: usize = 100;
/// The most edits one step of the undo history holds, the most one batch of a job carries out.
pub const MOST_EDITS: usize = 10_000;

/// Where a history stands: how many steps it keeps, and how many of them lie behind the present.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Walk {
    pub kept: usize,
    /// How many of the steps lie behind the present, never more than the history keeps.
    pub present: usize,
}

/// What a new step does to a history: how many of the oldest steps stay before it joins, whether the oldest goes once it joined, and where the history stands then.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Pushed {
    /// The steps that stay before the new one joins, which are those behind the present.
    pub stay: usize,
    /// Whether the oldest step goes once the new one joined, since the steps behind the present filled the bound.
    pub drops: bool,
    pub after: Walk,
}

/// What a new step does to a history that stands at `walk` and keeps at most `most` steps: the steps ahead go, the new step joins the steps behind the present, and the oldest goes once those filled the bound.
#[must_use]
pub const fn pushed(walk: Walk, most: usize) -> Pushed {
    let stay = walk.present;
    let drops = stay >= most;
    let kept = if drops { stay } else { stay.saturating_add(1) };
    Pushed {
        stay,
        drops,
        after: Walk {
            kept,
            present: kept,
        },
    }
}

/// Where back takes a history that stands at `walk`: the place of the step it passes and where the history stands then, or none when no step lies behind the present.
#[must_use]
pub fn back_from(walk: Walk) -> Option<(usize, Walk)> {
    walk.present.checked_sub(1).map(|at| {
        (
            at,
            Walk {
                kept: walk.kept,
                present: at,
            },
        )
    })
}

/// Where forward takes a history that stands at `walk`: the place of the step it passes and where the history stands then, or none when no step lies ahead of the present.
#[must_use]
pub fn forward_from(walk: Walk) -> Option<(usize, Walk)> {
    (walk.present < walk.kept).then(|| {
        (
            walk.present,
            Walk {
                kept: walk.kept,
                present: walk.present.saturating_add(1),
            },
        )
    })
}

/// Steps in the order they came, a present among them, and at most a bound of them.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct History<T> {
    /// Every step the history keeps, the oldest first.
    steps: Vec<T>,
    /// How many of the steps lie behind the present.
    present: usize,
    /// The most steps the history keeps.
    most: usize,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> History<T> {
    /// An empty history of at most [`MOST_STEPS`] steps.
    #[must_use]
    pub const fn new() -> Self {
        Self::bounded(MOST_STEPS)
    }

    /// An empty history of at most `most` steps, which keeps none when `most` is zero.
    #[must_use]
    pub const fn bounded(most: usize) -> Self {
        Self {
            steps: Vec::new(),
            present: 0,
            most,
        }
    }

    /// Where the history stands.
    #[must_use]
    pub const fn walk(&self) -> Walk {
        Walk {
            kept: self.steps.len(),
            present: self.present,
        }
    }

    /// Takes a new step as [`pushed`] decides: the steps ahead go, the new step joins the steps behind the present, and the oldest goes once those filled the bound.
    pub fn push(&mut self, step: T) {
        let pushed = pushed(self.walk(), self.most);
        self.steps.truncate(pushed.stay);
        self.steps.push(step);
        if pushed.drops {
            self.steps.remove(0);
        }
        self.present = pushed.after.present;
    }

    /// Takes a step of `edits` edits: one within [`MOST_EDITS`] joins the history as [`History::push`] takes it, and a larger one clears the history.
    pub fn record(&mut self, step: T, edits: usize) {
        if edits > MOST_EDITS {
            self.clear();
        } else {
            self.push(step);
        }
    }

    /// Forgets every step.
    pub fn clear(&mut self) {
        self.steps.clear();
        self.present = 0;
    }

    /// Moves the present back over the most recent step behind it, as [`back_from`] decides, and answers that step, or none when no step lies behind.
    pub fn back(&mut self) -> Option<&T> {
        let (at, after) = back_from(self.walk())?;
        self.present = after.present;
        self.steps.get(at)
    }

    /// Moves the present forward over the next step ahead of it, as [`forward_from`] decides, and answers that step, or none when no step lies ahead.
    pub fn forward(&mut self) -> Option<&T> {
        let (at, after) = forward_from(self.walk())?;
        self.present = after.present;
        self.steps.get(at)
    }

    /// The step that back takes, if any.
    #[must_use]
    pub fn last(&self) -> Option<&T> {
        back_from(self.walk()).and_then(|(at, _)| self.steps.get(at))
    }

    /// The step that forward takes, if any.
    #[must_use]
    pub fn next(&self) -> Option<&T> {
        forward_from(self.walk()).and_then(|(at, _)| self.steps.get(at))
    }
}

/// The properties Kani proves over every place a history of any size and any bound can stand at: a new step clears the steps ahead and keeps the bound, and back and forward each pass exactly one step and undo each other.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Walk, back_from, forward_from, pushed};

    /// Whether every act decides as a history promises from `walk` at a bound of `most`, for a walk whose present lies among its steps and whose steps keep the bound.
    ///
    /// A new step keeps the steps behind the present, joins them, drops the oldest exactly when they filled the bound, and leaves no step ahead.
    /// Back passes the most recent step behind the present and forward the next step ahead, each exactly when one lies there, and each undoes the other.
    pub(super) fn walks(walk: Walk, most: usize) -> bool {
        let Walk { kept, present } = walk;
        let new = pushed(walk, most);
        let joined = present.checked_add(1).map_or(most, |held| held.min(most));
        let pushes = new.stay == present
            && new.drops == (present == most)
            && new.after.kept == joined
            && new.after.present == new.after.kept;
        let backs = back_from(walk).map_or(present == 0, |(at, after)| {
            at.checked_add(1) == Some(present)
                && at < kept
                && after == Walk { kept, present: at }
                && forward_from(after) == Some((at, walk))
        });
        let forwards = forward_from(walk).map_or(present == kept, |(at, after)| {
            at == present
                && at < kept
                && Some(after.present) == present.checked_add(1)
                && after.kept == kept
                && back_from(after) == Some((at, walk))
        });
        pushes && backs && forwards
    }

    /// Every place a history of any size can stand at, at every bound it keeps.
    #[cfg(kani)]
    #[kani::proof]
    fn a_history_walks_one_step_at_a_time_and_keeps_its_bound() {
        let most: usize = kani::any();
        let walk = Walk {
            kept: kani::any(),
            present: kani::any(),
        };
        kani::assume(walk.present <= walk.kept && walk.kept <= most);
        kani::assert(
            walks(walk, most),
            "a new step clears the steps ahead and keeps the bound, and back and forward pass one step",
        );
        kani::cover!(
            walk.present == most && most > 0,
            "the bound drops the oldest step"
        );
        kani::cover!(
            walk.present > 0 && walk.present < walk.kept,
            "steps behind and ahead"
        );
        kani::cover!(most == 0, "a bound that keeps no step");
    }

    /// A new step must clear the steps ahead, or a redo would replay a step the new one replaced; this probe claims a step ahead survives a new step and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_step_ahead_kept_past_a_new_step() {
        let new = pushed(
            Walk {
                kept: 2,
                present: 1,
            },
            2,
        );
        kani::assert(
            forward_from(new.after).is_some(),
            "NEGATIVE_HISTORY_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::walks;
    use super::{History, MOST_EDITS, MOST_STEPS, Walk};
    use alloc::vec::Vec;

    #[test]
    fn back_and_forward_walk_one_step_each() {
        let mut history: History<&str> = History::default();
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
        history.push("move");
        history.push("rename");
        assert_eq!(history.last(), Some(&"rename"));
        assert_eq!(history.back(), Some(&"rename"));
        assert_eq!(history.last(), Some(&"move"));
        assert_eq!(history.next(), Some(&"rename"));
        assert_eq!(history.back(), Some(&"move"));
        assert_eq!(history.back(), None, "nothing lies behind the first step");
        assert_eq!(
            history.walk(),
            Walk {
                kept: 2,
                present: 0
            }
        );
        assert_eq!(history.next(), Some(&"move"));
        assert_eq!(history.forward(), Some(&"move"));
        assert_eq!(history.forward(), Some(&"rename"));
        assert_eq!(
            history.forward(),
            None,
            "nothing lies ahead of the last step"
        );
        assert_eq!(history.last(), Some(&"rename"));
    }

    #[test]
    fn a_new_step_clears_what_lies_ahead() {
        let mut history: History<&str> = History::default();
        history.push("move");
        history.push("rename");
        assert_eq!(history.back(), Some(&"rename"));
        history.push("trash");
        assert_eq!(history.next(), None, "a new step clears the steps ahead");
        assert_eq!(history.forward(), None);
        assert_eq!(history.back(), Some(&"trash"));
        assert_eq!(history.back(), Some(&"move"));
        history.clear();
        assert_eq!(history.last(), None);
        assert_eq!(history.next(), None);
    }

    #[test]
    fn a_history_keeps_its_bound_and_a_step_of_too_many_edits_clears_it() {
        let mut history = History::new();
        for step in 0..=MOST_STEPS {
            history.record(step, MOST_EDITS);
        }
        assert_eq!(
            history.walk(),
            Walk {
                kept: MOST_STEPS,
                present: MOST_STEPS
            }
        );
        let mut walked = Vec::new();
        while let Some(step) = history.back() {
            walked.push(*step);
        }
        assert_eq!(walked.len(), MOST_STEPS);
        assert_eq!(walked.last(), Some(&1), "the oldest went");
        history.record(MOST_STEPS + 1, MOST_EDITS + 1);
        assert_eq!(history.last(), None);
        assert_eq!(history.next(), None);
        let mut none = History::bounded(0);
        none.push(1);
        assert_eq!(none.last(), None, "a bound of zero keeps no step");
    }

    #[test]
    fn every_place_of_a_drawn_domain_walks() {
        let sizes = [0, 1, 2, 3, MOST_STEPS, usize::MAX - 1, usize::MAX];
        for most in sizes {
            for kept in sizes.into_iter().filter(|kept| *kept <= most) {
                for present in sizes.into_iter().filter(|present| *present <= kept) {
                    assert!(
                        walks(Walk { kept, present }, most),
                        "{kept} {present} {most}"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover"
    )]
    fn every_short_run_keeps_the_steps_a_list_with_a_present_keeps() {
        // Each act is a step of the next number, a back, or a forward, against a list of steps and a present.
        for most in 1..=3 {
            for run in 0..3_usize.pow(6) {
                let mut history = History::bounded(most);
                let mut steps: Vec<usize> = Vec::new();
                let mut present = 0;
                let mut choice = run;
                for step in 0..6 {
                    match choice % 3 {
                        0 => {
                            history.push(step);
                            steps.truncate(present);
                            steps.push(step);
                            if steps.len() > most {
                                steps.remove(0);
                            }
                            present = steps.len();
                        }
                        1 => {
                            let expected = present.checked_sub(1).map(|at| {
                                present = at;
                                steps[at]
                            });
                            assert_eq!(history.back().copied(), expected);
                        }
                        _ => {
                            let expected = steps.get(present).copied();
                            if expected.is_some() {
                                present += 1;
                            }
                            assert_eq!(history.forward().copied(), expected);
                        }
                    }
                    choice /= 3;
                    assert_eq!(history.steps, steps, "run {run} at a bound of {most}");
                    assert_eq!(history.present, present, "run {run} at a bound of {most}");
                }
            }
        }
    }
}
