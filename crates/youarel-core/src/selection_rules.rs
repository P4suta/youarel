//! What a person selects in a listing: rows by their identifiers, the rows between two places, or every row.
//!
//! A selection adds rows or leaves them out by identifier, so a selection of a million rows names none of them.
//! A listing names its rows by places from zero and each row by an identifier.
//! A selection of places holds to the listing it was made in, takes no row of another listing by its places, and takes no place past the end of the listing it reads.

use alloc::vec::Vec;

/// The listing a selection of places was made in, as a hash of the facts that order its rows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Listing(pub u64);

/// The rows and store generation that an identifier snapshot was read from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Version {
    pub listing: Listing,
    pub generation: u64,
}

/// A listing as it shows now: which one, and how many rows it holds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Shown {
    pub listing: Listing,
    pub total: usize,
}

/// The rows a selection takes by their places.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Span {
    /// No row by its place.
    #[default]
    None,
    /// The rows of a listing from one place to another, both taken.
    Places {
        listing: Listing,
        from: usize,
        to: usize,
    },
    /// Every row of a listing.
    All(Listing),
}

impl Span {
    /// Whether the span takes the row at a place of the listing shown, which it never does past the listing's end or in another listing.
    #[must_use]
    pub fn covers(self, shown: Shown, place: usize) -> bool {
        place < shown.total
            && match self {
                Self::None => false,
                Self::Places { listing, from, to } => {
                    listing == shown.listing && from <= place && place <= to
                }
                Self::All(listing) => listing == shown.listing,
            }
    }

    /// How many rows of the listing shown the span takes.
    #[must_use]
    pub fn covered(self, shown: Shown) -> usize {
        match self {
            Self::Places { listing, from, to } if listing == shown.listing => {
                to.saturating_add(1).min(shown.total).saturating_sub(from)
            }
            Self::All(listing) if listing == shown.listing => shown.total,
            Self::None | Self::Places { .. } | Self::All(_) => 0,
        }
    }

    /// Where a place of the listing shown lies against the span.
    #[must_use]
    pub fn reach(self, shown: Shown, place: usize) -> Reach {
        match (place < shown.total, self.covers(shown, place)) {
            (false, _) => Reach::Past,
            (true, true) => Reach::Inside,
            (true, false) => Reach::Outside,
        }
    }
}

/// Where a place lies against a listing and a span.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Reach {
    /// Past the listing's end.
    #[default]
    Past,
    /// A row of the listing outside the span.
    Outside,
    /// A row of the listing inside the span.
    Inside,
}

/// What a toggle does to the lists of a selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Turn {
    /// Leaves out a row inside the span.
    LeaveOut,
    /// Takes back a row inside the span that it left out.
    TakeBack,
    /// Drops a row it added outside the span.
    Drop,
    /// Adds a row outside the span.
    Add,
}

/// Where a row stands in a selection: where it lies against the span, and whether the selection left it out or added it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Standing {
    pub reach: Reach,
    pub left_out: bool,
    pub added: bool,
}

impl Standing {
    /// Whether the selection takes the row: a row inside its span that it did not leave out, or outside its span that it added, and never a place past the listing's end.
    #[must_use]
    pub const fn taken(self) -> bool {
        match self.reach {
            Reach::Past => false,
            Reach::Outside => self.added,
            Reach::Inside => !self.left_out,
        }
    }

    /// What a toggle does to the row: inside the span, a row taken is left out and one left out is taken back; elsewhere, a row added is dropped and any other is added.
    #[must_use]
    pub const fn turn(self) -> Turn {
        match (self.reach, self.taken()) {
            (Reach::Inside, true) => Turn::LeaveOut,
            (Reach::Inside, false) => Turn::TakeBack,
            (Reach::Outside | Reach::Past, true) => Turn::Drop,
            (Reach::Outside | Reach::Past, false) => Turn::Add,
        }
    }
}

/// What a person selected: rows by their places, with rows added and rows left out by their identifiers.
///
/// The rows it adds lie outside its span and the rows it leaves out lie inside it, which every change keeps, so a count reads no row.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Selection<Id> {
    span: Span,
    /// The rows outside the span that the selection takes, ascending.
    added: Vec<Id>,
    /// The rows inside the span that the selection leaves out, ascending.
    left: Vec<Id>,
}

/// The identifiers a selection took in one version, retained when new rows or another order replace its places.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Pinned<Id> {
    version: Version,
    selected: Selection<Id>,
}

impl<Id: Ord + Copy> Pinned<Id> {
    /// Pins the identifiers read under this version.
    #[must_use]
    pub fn new(version: Version, ids: impl IntoIterator<Item = Id>) -> Self {
        Self {
            version,
            selected: Selection::of(ids),
        }
    }

    /// The version whose places the snapshot still describes.
    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Whether the places still belong to the same listing and store generation.
    #[must_use]
    pub const fn keeps(&self, current: Version) -> bool {
        self.version.generation == current.generation && self.version.listing.0 == current.listing.0
    }

    /// Selects exactly the pinned identifiers, independent of where later listings put them.
    #[must_use]
    pub fn into_selection(self) -> Selection<Id> {
        self.selected
    }
}

impl<Id> Default for Selection<Id> {
    fn default() -> Self {
        Self {
            span: Span::None,
            added: Vec::new(),
            left: Vec::new(),
        }
    }
}

/// Puts an identifier into an ascending list once.
fn insert<Id: Ord>(ids: &mut Vec<Id>, id: Id) {
    if let Err(at) = ids.binary_search(&id) {
        ids.insert(at, id);
    }
}

/// Takes an identifier out of an ascending list.
fn remove<Id: Ord>(ids: &mut Vec<Id>, id: &Id) {
    if let Ok(at) = ids.binary_search(id) {
        ids.remove(at);
    }
}

impl<Id: Ord + Copy> Selection<Id> {
    /// Nothing selected.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            span: Span::None,
            added: Vec::new(),
            left: Vec::new(),
        }
    }

    /// The rows of the identifiers given.
    #[must_use]
    pub fn of(ids: impl IntoIterator<Item = Id>) -> Self {
        let mut added: Vec<Id> = ids.into_iter().collect();
        added.sort_unstable();
        added.dedup();
        Self {
            span: Span::None,
            added,
            left: Vec::new(),
        }
    }

    /// Every row of a listing.
    #[must_use]
    pub const fn all(listing: Listing) -> Self {
        Self {
            span: Span::All(listing),
            added: Vec::new(),
            left: Vec::new(),
        }
    }

    /// The rows of a listing between two places, both taken, whichever of them comes first.
    #[must_use]
    pub fn between(listing: Listing, one: usize, other: usize) -> Self {
        Self {
            span: Span::Places {
                listing,
                from: one.min(other),
                to: one.max(other),
            },
            added: Vec::new(),
            left: Vec::new(),
        }
    }

    /// The rows the selection takes by their places.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// The rows outside its span that the selection takes, ascending.
    #[must_use]
    pub fn added(&self) -> &[Id] {
        &self.added
    }

    /// The rows inside its span that the selection leaves out, ascending.
    #[must_use]
    pub fn left(&self) -> &[Id] {
        &self.left
    }

    /// Where the row of an identifier at a place of the listing shown stands in the selection.
    fn standing(&self, shown: Shown, place: usize, id: Id) -> Standing {
        Standing {
            reach: self.span.reach(shown, place),
            left_out: self.left.binary_search(&id).is_ok(),
            added: self.added.binary_search(&id).is_ok(),
        }
    }

    /// Whether the selection takes the row of an identifier at a place of the listing shown, which it never does past the listing's end.
    #[must_use]
    pub fn contains(&self, shown: Shown, place: usize, id: Id) -> bool {
        self.standing(shown, place, id).taken()
    }

    /// Takes the row of an identifier at a place of the listing shown into the selection, or out of it; a place past the listing's end changes nothing.
    pub fn toggle(&mut self, shown: Shown, place: usize, id: Id) {
        if place >= shown.total {
            return;
        }
        match self.standing(shown, place, id).turn() {
            Turn::LeaveOut => insert(&mut self.left, id),
            Turn::TakeBack => remove(&mut self.left, &id),
            Turn::Drop => remove(&mut self.added, &id),
            Turn::Add => insert(&mut self.added, id),
        }
    }

    /// Forgets an identifier that names no row any longer, so the selection neither adds it nor leaves it out.
    pub fn forget(&mut self, id: &Id) {
        remove(&mut self.added, id);
        remove(&mut self.left, id);
    }

    /// How many rows of the listing shown the selection takes, when each identifier it names names one row of that listing.
    #[must_use]
    pub fn count(&self, shown: Shown) -> usize {
        self.span
            .covered(shown)
            .saturating_sub(self.left.len())
            .saturating_add(self.added.len())
    }

    /// The selection over the same rows in another order: every row stays every row, and the rows between two places, which another order moves apart, give way to the rows it added.
    #[must_use]
    pub fn reordered(self, listing: Listing) -> Self {
        match self.span {
            Span::All(_) => Self {
                span: Span::All(listing),
                ..self
            },
            Span::None | Span::Places { .. } => Self::of(self.added),
        }
    }
}

/// The properties Kani proves over every listing of a few rows and every span and marks that keep a selection's promise there: no place past the listing's end is taken, the count is the number of rows taken, and a toggle turns one row alone and keeps the promise.
///
/// The harness reads the lists of identifiers as marks on places, since a selection reads its lists only for whether they hold an identifier and how long they are; the tests run the lists themselves through every short sequence of changes.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn pinned_identifiers_survive_a_change_of_generation_or_order() {
        let before = super::Version {
            listing: Listing(kani::any()),
            generation: kani::any(),
        };
        let current = super::Version {
            listing: Listing(kani::any()),
            generation: kani::any(),
        };
        let ids: [u8; 3] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= ids.len());
        let pinned = super::Pinned::new(before, ids.iter().copied().take(length));
        kani::assert(
            pinned.keeps(current) == (before == current),
            "a snapshot keeps places only in its original version",
        );
        let selected = pinned.into_selection();
        let id: u8 = kani::any();
        let listed = Shown {
            listing: current.listing,
            total: 1,
        };
        let expected = ids.iter().take(length).any(|saved| *saved == id);
        kani::assert(
            selected.contains(listed, 0, id) == expected,
            "later rows select exactly the saved identifiers",
        );
        kani::assert(
            selected.span() == Span::None,
            "the snapshot no longer selects by old places",
        );
        kani::cover!(
            before.generation != current.generation,
            "another store generation"
        );
        kani::cover!(before.listing != current.listing, "another row order");
        kani::cover!(
            length == 0 && before == current,
            "an empty selection in the same version"
        );
    }

    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn rejects_a_new_row_joining_a_pinned_selection() {
        let pinned = super::Pinned::new(
            super::Version {
                listing: Listing(1),
                generation: 1,
            },
            [1_u8],
        );
        kani::assert(
            pinned.into_selection().contains(
                Shown {
                    listing: Listing(2),
                    total: 1,
                },
                0,
                2,
            ),
            "NEGATIVE_SELECTION_CONTROL",
        );
    }
    use super::{Listing, Selection, Shown, Span, Standing, Turn};

    /// The most rows of a listing the harness draws.
    pub(super) const ROWS: usize = 3;
    /// The places the marks of the harness hold, the last past every listing it draws.
    pub(super) const PLACES: usize = 4;
    /// The mark of each place.
    const MARKS: [u8; PLACES] = [1, 2, 4, 8];

    /// The rows a selection's lists name, as the marks of their places.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(super) struct Marks {
        pub(super) left: u8,
        pub(super) added: u8,
    }

    /// The mark of a place, or none past the places the marks hold.
    fn mark(place: usize) -> u8 {
        MARKS.get(place).copied().unwrap_or_default()
    }

    /// Whether marks hold a place.
    fn marked(marks: u8, place: usize) -> bool {
        (marks & mark(place)) != 0
    }

    /// Whether marks keep a selection's promise: a row left out lies inside the span and a row added outside it, each a row of the listing.
    pub(super) fn keeps_marks(span: Span, shown: Shown, marks: Marks) -> bool {
        (0..PLACES).all(|place| {
            let listed = place < shown.total;
            let covered = span.covers(shown, place);
            (!marked(marks.left, place) || (listed && covered))
                && (!marked(marks.added, place) || (listed && !covered))
        })
    }

    /// Where the row at a place stands under a span and marks.
    fn standing(span: Span, shown: Shown, marks: Marks, place: usize) -> Standing {
        Standing {
            reach: span.reach(shown, place),
            left_out: marked(marks.left, place),
            added: marked(marks.added, place),
        }
    }

    /// Whether the row at a place is taken under a span and marks.
    pub(super) fn taken_at(span: Span, shown: Shown, marks: Marks, place: usize) -> bool {
        standing(span, shown, marks, place).taken()
    }

    /// What a selection's count promises over marks that keep the promise: no place past the listing's end is taken, and the span's rows less those left out and with those added are the rows taken.
    pub(super) fn marks_spec(span: Span, shown: Shown, marks: Marks) -> bool {
        let taken = (0..PLACES)
            .filter(|place| taken_at(span, shown, marks, *place))
            .count();
        let past = (shown.total..PLACES).all(|place| !taken_at(span, shown, marks, place));
        let named = |held: u8| (0..PLACES).filter(|place| marked(held, *place)).count();
        let counted = span
            .covered(shown)
            .saturating_sub(named(marks.left))
            .saturating_add(named(marks.added));
        past && taken == counted
    }

    /// The marks after a toggle's turn at a place.
    fn turned(marks: Marks, change: Turn, place: usize) -> Marks {
        let bit = mark(place);
        match change {
            Turn::LeaveOut => Marks {
                left: marks.left | bit,
                added: marks.added,
            },
            Turn::TakeBack => Marks {
                left: marks.left & !bit,
                added: marks.added,
            },
            Turn::Drop => Marks {
                left: marks.left,
                added: marks.added & !bit,
            },
            Turn::Add => Marks {
                left: marks.left,
                added: marks.added | bit,
            },
        }
    }

    /// What a toggle promises over marks that keep the promise: at a row of the listing, the row turns, every other row stays, and the marks keep the promise.
    pub(super) fn turn_spec(span: Span, shown: Shown, marks: Marks, place: usize) -> bool {
        let before = |at| taken_at(span, shown, marks, at);
        let next = turned(marks, standing(span, shown, marks, place).turn(), place);
        keeps_marks(span, shown, next)
            && (0..PLACES)
                .all(|at| taken_at(span, shown, next, at) == (before(at) != (at == place)))
    }

    /// The listing the harness shows, and another one.
    pub(super) const SHOWN: Listing = Listing(0);
    pub(super) const OTHER: Listing = Listing(1);

    /// One change a person makes to a selection, which the tests run in every short sequence.
    #[cfg(test)]
    #[derive(Clone, Copy, Debug)]
    pub(super) enum Change {
        /// The row at a place alone.
        One(usize),
        /// The row at a place in or out.
        Toggle(usize),
        /// The rows of a listing between two places.
        Between(Listing, usize, usize),
        /// Every row of a listing.
        All(Listing),
    }

    /// The identifier of the row at place zero, apart from every place the harness draws, so a mix of places and identifiers shows.
    pub(super) const FIRST_ID: usize = 10;

    /// The identifier of the row at a place, which the harness keeps apart for each place.
    pub(super) const fn id(place: usize) -> usize {
        place.saturating_add(FIRST_ID)
    }

    /// A selection after a change, over the listing shown.
    #[cfg(test)]
    pub(super) fn changed(
        selection: Selection<usize>,
        shown: Shown,
        change: Change,
    ) -> Selection<usize> {
        match change {
            Change::One(place) => Selection::of([id(place)]),
            Change::Toggle(place) => {
                let mut toggled = selection;
                toggled.toggle(shown, place, id(place));
                toggled
            }
            Change::Between(listing, one, other) => Selection::between(listing, one, other),
            Change::All(listing) => Selection::all(listing),
        }
    }

    /// What a selection promises over the listing shown, when each identifier it names names one of its rows: no place past the end, and a count of exactly the rows it takes.
    #[cfg(test)]
    pub(super) fn spec(selection: &Selection<usize>, shown: Shown) -> bool {
        let past = (shown.total..=ROWS).all(|place| !selection.contains(shown, place, id(place)));
        let taken = (0..shown.total)
            .filter(|place| selection.contains(shown, *place, id(*place)))
            .count();
        past && taken == selection.count(shown)
    }

    /// Whether a selection keeps its promise over the listing shown: the rows it adds lie outside its span and the rows it leaves out inside it, each a row of the listing named once, in ascending order.
    #[cfg(test)]
    pub(super) fn keeps(selection: &Selection<usize>, shown: Shown) -> bool {
        let rows = |ids: &[usize], inside: bool| {
            ids.windows(2).all(|pair| pair.first() < pair.last())
                && ids.iter().all(|held| {
                    held.checked_sub(FIRST_ID).is_some_and(|place| {
                        place < shown.total && selection.span.covers(shown, place) == inside
                    })
                })
        };
        rows(&selection.added, false) && rows(&selection.left, true)
    }

    /// What a toggle promises: the row at its place turns when the listing holds it, and every other row stays as it was.
    #[cfg(test)]
    pub(super) fn toggle_spec(selection: &Selection<usize>, shown: Shown, place: usize) -> bool {
        let mut toggled = selection.clone();
        toggled.toggle(shown, place, id(place));
        (0..=ROWS).all(|at| {
            let before = selection.contains(shown, at, id(at));
            let after = toggled.contains(shown, at, id(at));
            if at == place && place < shown.total {
                before != after
            } else {
                before == after
            }
        })
    }

    /// No span, or a span of either listing between any two places in order, or over every row.
    #[cfg(kani)]
    fn any_span() -> Span {
        let listing = if kani::any() { SHOWN } else { OTHER };
        let (from, to): (usize, usize) = (kani::any(), kani::any());
        kani::assume(from <= to);
        match kani::any::<u8>() {
            0 => Span::None,
            1 => Span::Places { listing, from, to },
            _ => Span::All(listing),
        }
    }

    /// Every listing of up to three rows, every span, every marks that keep the promise there, and every toggle at a row of the listing: no place past the end is taken, the count is the number of rows taken, and a toggle turns its row alone and keeps the promise.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn a_selection_takes_no_place_past_its_listing_and_counts_what_it_takes() {
        let total: usize = kani::any();
        kani::assume(total <= ROWS);
        let shown = Shown {
            listing: SHOWN,
            total,
        };
        let span = any_span();
        let marks = Marks {
            left: kani::any(),
            added: kani::any(),
        };
        kani::assume(keeps_marks(span, shown, marks));
        kani::assert(
            marks_spec(span, shown, marks),
            "a selection takes no place past its listing and counts what it takes",
        );
        let place: usize = kani::any();
        kani::assume(place < total);
        kani::assert(
            turn_spec(span, shown, marks, place),
            "a toggle turns its row alone and keeps the promise",
        );
        kani::cover!(
            span.covered(shown) == total && total == ROWS && marks == Marks::default(),
            "every row of the listing"
        );
        kani::cover!(
            marks.left != 0 && marks.added != 0,
            "rows added and left out"
        );
        kani::cover!(
            matches!(span, Span::Places { to, .. } if to == usize::MAX) && span.covered(shown) > 0,
            "a span past the listing's end"
        );
    }

    /// No selection takes a place past its listing's end; this probe claims selecting every row takes one and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_selection_of_a_place_outside_the_listing() {
        let shown = Shown {
            listing: SHOWN,
            total: 2,
        };
        kani::assert(
            Selection::<usize>::all(SHOWN).contains(shown, 2, id(2)),
            "NEGATIVE_SELECTION_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pinned_identifiers_keep_only_the_original_selection_after_new_rows_arrive() {
        let version = super::Version {
            listing: Listing(1),
            generation: 7,
        };
        let pinned = super::Pinned::new(version, [3_u8, 1, 3]);
        assert!(pinned.keeps(version));
        assert!(!pinned.keeps(super::Version {
            generation: 8,
            ..version
        }));
        assert!(!pinned.keeps(super::Version {
            listing: Listing(2),
            ..version
        }));
        assert_eq!(pinned.into_selection().added(), [1, 3]);
    }
    use super::proofs::{
        Change, FIRST_ID, Marks, OTHER, PLACES, ROWS, SHOWN, changed, id, keeps, keeps_marks,
        marks_spec, spec, toggle_spec, turn_spec,
    };
    use super::{Listing, Reach, Selection, Shown, Span, Standing, Turn};
    use alloc::vec::Vec;

    /// Every mark a harness's list may hold over its places.
    const EVERY_MARK: u8 = 16;

    #[test]
    fn the_marks_keep_their_promise_over_every_span() {
        let mut spans = alloc::vec![Span::None, Span::All(SHOWN), Span::All(OTHER)];
        for listing in [SHOWN, OTHER] {
            for from in 0..PLACES {
                for to in (from..PLACES).chain([usize::MAX]) {
                    spans.push(Span::Places { listing, from, to });
                }
            }
        }
        let mut kept = 0_usize;
        for total in 0..=ROWS {
            let shown = Shown {
                listing: SHOWN,
                total,
            };
            for span in &spans {
                for left in 0..EVERY_MARK {
                    for added in 0..EVERY_MARK {
                        let marks = Marks { left, added };
                        if !keeps_marks(*span, shown, marks) {
                            continue;
                        }
                        kept = kept.saturating_add(1);
                        assert!(marks_spec(*span, shown, marks), "{span:?} {marks:?}");
                        for place in 0..total {
                            assert!(turn_spec(*span, shown, marks, place), "{span:?} {marks:?}");
                        }
                    }
                }
            }
        }
        assert!(kept > spans.len());
    }

    #[test]
    fn a_turn_and_a_take_follow_the_span() {
        let stand = |reach, left_out, added| Standing {
            reach,
            left_out,
            added,
        };
        assert_eq!(stand(Reach::Inside, false, false).turn(), Turn::LeaveOut);
        assert_eq!(stand(Reach::Inside, true, false).turn(), Turn::TakeBack);
        assert_eq!(stand(Reach::Outside, false, true).turn(), Turn::Drop);
        assert_eq!(stand(Reach::Outside, false, false).turn(), Turn::Add);
        assert_eq!(stand(Reach::Past, false, false).turn(), Turn::Add);
        assert!(stand(Reach::Inside, false, false).taken());
        assert!(!stand(Reach::Inside, true, true).taken());
        assert!(stand(Reach::Outside, true, true).taken());
        assert!(!stand(Reach::Outside, false, false).taken());
        assert!(
            !stand(Reach::Past, false, true).taken(),
            "never past the listing's end"
        );
        let shown = Shown {
            listing: SHOWN,
            total: 3,
        };
        let span = Span::Places {
            listing: SHOWN,
            from: 1,
            to: 1,
        };
        assert_eq!(span.reach(shown, 0), Reach::Outside);
        assert_eq!(span.reach(shown, 1), Reach::Inside);
        assert_eq!(span.reach(shown, 3), Reach::Past);
        assert_eq!(Standing::default().reach, Reach::Past);
    }

    /// Every change of the harness's kinds at places up to the bound, over both listings.
    fn changes(total: usize) -> Vec<Change> {
        let mut all = Vec::new();
        for place in 0..=ROWS {
            if place < total {
                all.push(Change::One(place));
            }
            all.push(Change::Toggle(place));
            for other in 0..=ROWS {
                all.push(Change::Between(SHOWN, place, other));
                all.push(Change::Between(OTHER, place, other));
            }
        }
        all.push(Change::All(SHOWN));
        all.push(Change::All(OTHER));
        all
    }

    #[test]
    #[cfg_attr(miri, ignore = "an exhaustive sweep that the native run covers")]
    fn the_specifications_agree_over_every_run_of_three_changes() {
        for total in 0..=ROWS {
            let shown = Shown {
                listing: SHOWN,
                total,
            };
            let kinds = changes(total);
            let mut seen = 0_usize;
            for first in &kinds {
                for second in &kinds {
                    for third in &kinds {
                        let selection = [*first, *second, *third]
                            .into_iter()
                            .fold(Selection::none(), |held, change| {
                                changed(held, shown, change)
                            });
                        assert!(keeps(&selection, shown), "{selection:?} {total}");
                        assert!(spec(&selection, shown), "{selection:?} {total}");
                        for place in 0..=ROWS {
                            assert!(toggle_spec(&selection, shown, place), "{selection:?}");
                        }
                        seen = seen.saturating_add(1);
                    }
                }
            }
            assert!(seen > 0);
        }
    }

    #[test]
    fn a_span_holds_to_its_listing_and_no_place_past_the_end() {
        let shown = Shown {
            listing: Listing(7),
            total: 5,
        };
        let between = Selection::between(Listing(7), 3, 1);
        assert_eq!(
            between.span(),
            Span::Places {
                listing: Listing(7),
                from: 1,
                to: 3
            }
        );
        assert!(!between.contains(shown, 0, 0));
        assert!(between.contains(shown, 1, 1) && between.contains(shown, 3, 3));
        assert!(!between.contains(shown, 4, 4));
        assert_eq!(between.count(shown), 3);
        let elsewhere = Shown {
            listing: Listing(8),
            total: 5,
        };
        assert!(!between.contains(elsewhere, 2, 2));
        assert_eq!(between.count(elsewhere), 0);
        let all = Selection::<u8>::all(Listing(7));
        assert!(all.contains(shown, 4, 4) && !all.contains(shown, 5, 5));
        assert_eq!(all.count(shown), 5);
        assert_eq!(all.count(elsewhere), 0);
        let wide = Selection::<u8>::between(Listing(7), 2, usize::MAX);
        assert_eq!(wide.count(shown), 3, "a span past the end counts to it");
        assert!(
            !wide.span().covers(shown, 5) && !Span::All(Listing(7)).covers(shown, 5),
            "a span covers no place at the listing's end"
        );
        assert!(Span::All(Listing(7)).covers(shown, 4));
        let beyond = Selection::<u8>::between(Listing(7), 6, 9);
        assert_eq!(beyond.count(shown), 0);
    }

    #[test]
    fn a_toggle_leaves_out_a_row_of_the_span_and_adds_one_outside_it() {
        let shown = Shown {
            listing: Listing(1),
            total: 6,
        };
        let mut selection = Selection::between(Listing(1), 1, 2);
        selection.toggle(shown, 2, 20);
        assert_eq!(selection.left(), [20]);
        assert!(!selection.contains(shown, 2, 20));
        selection.toggle(shown, 4, 40);
        selection.toggle(shown, 5, 50);
        assert_eq!(selection.added(), [40, 50]);
        assert_eq!(selection.count(shown), 3);
        selection.toggle(shown, 2, 20);
        selection.toggle(shown, 4, 40);
        assert_eq!((selection.left(), selection.added()), (&[][..], &[50][..]));
        selection.toggle(shown, 6, 60);
        assert_eq!(
            selection.count(shown),
            3,
            "a place past the end changes nothing"
        );
        assert!(
            !selection.contains(shown, 6, 50),
            "nor does a place past the end show"
        );
    }

    #[test]
    fn a_forgotten_identifier_leaves_both_lists() {
        let shown = Shown {
            listing: Listing(1),
            total: 6,
        };
        let mut selection = Selection::between(Listing(1), 0, 2);
        selection.toggle(shown, 1, 10_u8);
        selection.toggle(shown, 4, 40);
        selection.toggle(shown, 5, 50);
        selection.forget(&10);
        selection.forget(&40);
        assert_eq!((selection.left(), selection.added()), (&[][..], &[50][..]));
        selection.forget(&60);
        assert_eq!(selection.count(shown), 4);
    }

    #[test]
    fn identifiers_select_once_wherever_they_stand() {
        let shown = Shown {
            listing: Listing(3),
            total: 4,
        };
        let picked = Selection::of([3_u8, 1, 3, 2]);
        assert_eq!(picked.added(), [1, 2, 3]);
        assert_eq!(picked.span(), Span::None);
        assert!(picked.contains(shown, 0, 2));
        assert_eq!(picked.count(shown), 3);
        assert_eq!(Selection::<u8>::none(), Selection::default());
        assert_eq!(Selection::<u8>::none().count(shown), 0);
        assert_eq!(id(ROWS), ROWS + FIRST_ID);
        assert_eq!(id(usize::MAX), usize::MAX);
    }

    #[test]
    fn another_order_keeps_every_row_and_lets_a_range_go() {
        let mut all = Selection::all(Listing(1));
        all.toggle(
            Shown {
                listing: Listing(1),
                total: 3,
            },
            1,
            7_u8,
        );
        let reordered = all.reordered(Listing(2));
        assert_eq!(reordered.span(), Span::All(Listing(2)));
        assert_eq!(reordered.left(), [7], "what it left out stays out");
        let mut range = Selection::between(Listing(1), 0, 2);
        range.toggle(
            Shown {
                listing: Listing(1),
                total: 9,
            },
            5,
            9_u8,
        );
        let narrowed = range.reordered(Listing(2));
        assert_eq!(narrowed, Selection::of([9]));
        assert_eq!(
            Selection::of([4_u8]).reordered(Listing(2)),
            Selection::of([4])
        );
    }
}
