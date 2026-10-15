//! The launcher's interface as a state machine over the keys a shell reads.
//!
//! [`step`] takes the interface, the rows the shell shows for it, and one event, and hands back the next interface and at most one effect.
//! The shell owns the terminal, the store, and the browser; it carries out the effect, recomputes the rows, and sends [`Event::Refresh`] so the selection stays on a row.

use crate::state::{Entry, ItemId};
use alloc::string::String;

/// A key the shell reads, with the modifiers the machine tells apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Key {
    Char(char),
    /// A letter typed with the control key.
    Control(char),
    Backspace,
    Enter,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Tab,
    /// The up arrow with shift, which moves the selected entry up.
    ShiftUp,
    /// The down arrow with shift, which moves the selected entry down.
    ShiftDown,
}

/// What the shell feeds the machine.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Event {
    Key(Key),
    /// Text pasted into the terminal at once.
    Paste(String),
    /// The rows changed under the interface, after an effect or another process's edit.
    Refresh,
}

/// One row the shell shows.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Row {
    /// The way up out of the folder, to its parent or the top.
    Parent(Option<ItemId>),
    Entry(Entry),
    /// The pasted or typed link, normalized, which the store does not hold yet.
    Add(String),
}

/// The field a person types into.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Field {
    Title,
    Alias,
    /// A folder's name, or a new folder's when the typing has no target.
    Name,
}

/// What the keys do now.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Mode {
    /// Typing searches, the arrows select, and Enter opens.
    Browse,
    /// Typing fills a field of the target, or names a new folder when there is no target.
    Typing {
        target: Option<Entry>,
        field: Field,
        text: String,
    },
    /// The next key confirms the removal of the target, or cancels it.
    Confirm { target: Entry },
}

/// The whole interface.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Ui {
    pub query: String,
    /// The folder the person browses, or `None` for the top.
    pub folder: Option<ItemId>,
    /// The index of the selected row.
    pub selected: usize,
    pub mode: Mode,
    /// The entry the person carries to another folder.
    pub carried: Option<Entry>,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            query: String::new(),
            folder: None,
            selected: 0,
            mode: Mode::Browse,
            carried: None,
        }
    }
}

/// What the shell carries out after a step.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Effect {
    None,
    /// Open the shortcut in the default browser, and leave.
    Open(ItemId),
    /// Add the link as a shortcut in the browsed folder.
    Add(String),
    /// Give a shortcut a new title, or a folder a new name.
    Rename {
        target: Entry,
        text: String,
    },
    /// Give a shortcut a new alias, or none when the text is empty.
    Realias {
        id: ItemId,
        alias: String,
    },
    CreateFolder {
        name: String,
        parent: Option<ItemId>,
    },
    Remove(Entry),
    /// Move the entry to the end of the folder.
    Move {
        entry: Entry,
        folder: Option<ItemId>,
    },
    /// Move the entry right before or right after its neighbor.
    Reorder {
        entry: Entry,
        neighbor: Entry,
        before: bool,
    },
    Quit,
}

/// The index of the last row, or zero when there is none.
const fn last(rows: &[Row]) -> usize {
    rows.len().saturating_sub(1)
}

fn selected<'a>(ui: &Ui, rows: &'a [Row]) -> Option<&'a Row> {
    rows.get(ui.selected)
}

fn selected_entry(ui: &Ui, rows: &[Row]) -> Option<Entry> {
    match selected(ui, rows) {
        Some(Row::Entry(entry)) => Some(*entry),
        Some(Row::Parent(_) | Row::Add(_)) | None => None,
    }
}

/// The interface after moving into `folder` with a fresh search.
const fn enter(ui: &Ui, folder: Option<ItemId>) -> Ui {
    Ui {
        query: String::new(),
        folder,
        selected: 0,
        mode: Mode::Browse,
        carried: ui.carried,
    }
}

/// The way up a folder's rows offer: to the parent folder, or to the top.
#[derive(Clone, Copy, Debug)]
struct Up {
    to: Option<ItemId>,
}

/// The way up the rows offer, when the person browses a folder.
fn parent(rows: &[Row]) -> Option<Up> {
    rows.iter().find_map(|row| match row {
        Row::Parent(parent) => Some(Up { to: *parent }),
        Row::Entry(_) | Row::Add(_) => None,
    })
}

/// The typing in progress, as [`Mode::Typing`] holds it.
#[derive(Clone, Copy, Debug)]
struct Draft<'a> {
    target: Option<Entry>,
    field: Field,
    text: &'a str,
}

/// Text as the person meant to type it, without the line breaks and tabs a paste carries.
fn typed(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter(|character| !character.is_control())
}

fn browse(ui: &Ui, rows: &[Row], key: Key) -> (Ui, Effect) {
    let mut next = ui.clone();
    let effect = match key {
        Key::Char(character) => {
            next.query.push(character);
            next.selected = 0;
            Effect::None
        }
        Key::Backspace => {
            if next.query.pop().is_some() {
                next.selected = 0;
            } else if let (Some(_), Some(up)) = (ui.folder, parent(rows)) {
                next = enter(ui, up.to);
            }
            Effect::None
        }
        Key::Up => {
            next.selected = ui.selected.saturating_sub(1);
            Effect::None
        }
        Key::Down => {
            next.selected = ui.selected.saturating_add(1).min(last(rows));
            Effect::None
        }
        Key::Left => {
            if let (Some(_), Some(up)) = (ui.folder, parent(rows)) {
                next = enter(ui, up.to);
            }
            Effect::None
        }
        Key::Right | Key::Tab => {
            if let Some(Entry::Folder(folder)) = selected_entry(ui, rows) {
                next = enter(ui, Some(folder));
            }
            Effect::None
        }
        Key::Enter => match selected(ui, rows) {
            Some(Row::Parent(up)) => {
                next = enter(ui, *up);
                Effect::None
            }
            Some(Row::Entry(Entry::Folder(folder))) => {
                next = enter(ui, Some(*folder));
                Effect::None
            }
            Some(Row::Entry(Entry::Shortcut(id))) => Effect::Open(*id),
            Some(Row::Add(link)) => {
                next.query.clear();
                next.selected = 0;
                Effect::Add(link.clone())
            }
            None => Effect::None,
        },
        Key::Escape => {
            if ui.carried.is_some() {
                next.carried = None;
                Effect::None
            } else if ui.query.is_empty() {
                Effect::Quit
            } else {
                next.query.clear();
                next.selected = 0;
                Effect::None
            }
        }
        Key::ShiftUp | Key::ShiftDown => reorder(ui, rows, key, &mut next),
        Key::Control(letter) => command(ui, rows, letter, &mut next),
    };
    (next, effect)
}

/// Moves the selected entry past its neighbor in the folder's listing, which shows only without a search.
fn reorder(ui: &Ui, rows: &[Row], key: Key, next: &mut Ui) -> Effect {
    if !ui.query.is_empty() {
        return Effect::None;
    }
    let Some(entry) = selected_entry(ui, rows) else {
        return Effect::None;
    };
    let up = key == Key::ShiftUp;
    let place = if up {
        ui.selected.checked_sub(1)
    } else {
        ui.selected.checked_add(1)
    };
    match place.and_then(|place| rows.get(place).map(|row| (place, row))) {
        Some((place, Row::Entry(neighbor))) => {
            next.selected = place;
            Effect::Reorder {
                entry,
                neighbor: *neighbor,
                before: up,
            }
        }
        Some((_, Row::Parent(_) | Row::Add(_))) | None => Effect::None,
    }
}

/// The commands typed with the control key.
fn command(ui: &Ui, rows: &[Row], letter: char, next: &mut Ui) -> Effect {
    let entry = selected_entry(ui, rows);
    match (letter, entry) {
        ('c', _) => Effect::Quit,
        ('e', Some(target)) => {
            let field = match target {
                Entry::Folder(_) => Field::Name,
                Entry::Shortcut(_) => Field::Title,
            };
            next.mode = Mode::Typing {
                target: Some(target),
                field,
                text: String::new(),
            };
            Effect::None
        }
        ('l', Some(target @ Entry::Shortcut(_))) => {
            next.mode = Mode::Typing {
                target: Some(target),
                field: Field::Alias,
                text: String::new(),
            };
            Effect::None
        }
        ('d', Some(target)) => {
            next.mode = Mode::Confirm { target };
            Effect::None
        }
        ('o', _) => {
            next.mode = Mode::Typing {
                target: None,
                field: Field::Name,
                text: String::new(),
            };
            Effect::None
        }
        ('x', Some(target)) => {
            next.carried = Some(target);
            Effect::None
        }
        ('v', _) => match ui.carried {
            Some(carried) => {
                next.carried = None;
                Effect::Move {
                    entry: carried,
                    folder: ui.folder,
                }
            }
            None => Effect::None,
        },
        ('n', _) => {
            next.selected = ui.selected.saturating_add(1).min(last(rows));
            Effect::None
        }
        ('p', _) => {
            next.selected = ui.selected.saturating_sub(1);
            Effect::None
        }
        _ => Effect::None,
    }
}

fn finish_typing(ui: &Ui, draft: Draft<'_>) -> Effect {
    let text = draft.text;
    match (draft.target, draft.field) {
        (Some(target), Field::Title | Field::Name) if !text.is_empty() => Effect::Rename {
            target,
            text: text.into(),
        },
        (Some(Entry::Shortcut(id)), Field::Alias) => Effect::Realias {
            id,
            alias: text.into(),
        },
        (None, Field::Name) if !text.is_empty() => Effect::CreateFolder {
            name: text.into(),
            parent: ui.folder,
        },
        _ => Effect::None,
    }
}

fn typing(ui: &Ui, draft: Draft<'_>, event: &Event) -> (Ui, Effect) {
    let mut next = ui.clone();
    let Draft { target, field, .. } = draft;
    let mut text = String::from(draft.text);
    let effect = match event {
        Event::Key(Key::Char(character)) => {
            text.push(*character);
            Effect::None
        }
        Event::Paste(pasted) => {
            text.extend(typed(pasted));
            Effect::None
        }
        Event::Key(Key::Backspace) => {
            text.pop();
            Effect::None
        }
        Event::Key(Key::Enter) => {
            next.mode = Mode::Browse;
            let finished = Draft {
                target,
                field,
                text: &text,
            };
            return (next, finish_typing(ui, finished));
        }
        Event::Key(Key::Escape) => {
            next.mode = Mode::Browse;
            return (next, Effect::None);
        }
        Event::Key(Key::Control('c')) => return (next, Effect::Quit),
        Event::Key(_) | Event::Refresh => Effect::None,
    };
    next.mode = Mode::Typing {
        target,
        field,
        text,
    };
    (next, effect)
}

fn confirm(ui: &Ui, target: Entry, event: &Event) -> (Ui, Effect) {
    let mut next = ui.clone();
    let effect = match event {
        Event::Key(Key::Char('y' | 'Y') | Key::Enter) => Effect::Remove(target),
        Event::Key(Key::Control('c')) => Effect::Quit,
        Event::Refresh => return (next, Effect::None),
        Event::Key(_) | Event::Paste(_) => Effect::None,
    };
    next.mode = Mode::Browse;
    (next, effect)
}

/// The interface after one event, and the effect the shell carries out.
///
/// The selection stays on one of `rows`, or at zero when there is none.
#[must_use]
pub fn step(ui: &Ui, rows: &[Row], event: &Event) -> (Ui, Effect) {
    let (mut next, effect) = match (&ui.mode, event) {
        (Mode::Browse, Event::Key(key)) => browse(ui, rows, *key),
        (Mode::Browse, Event::Paste(text)) => {
            let mut next = ui.clone();
            next.query.extend(typed(text));
            next.selected = 0;
            (next, Effect::None)
        }
        (Mode::Browse, Event::Refresh) => (ui.clone(), Effect::None),
        (
            Mode::Typing {
                target,
                field,
                text,
            },
            _,
        ) => {
            let draft = Draft {
                target: *target,
                field: *field,
                text,
            };
            typing(ui, draft, event)
        }
        (Mode::Confirm { target }, _) => confirm(ui, *target, event),
    };
    next.selected = next.selected.min(last(rows));
    (next, effect)
}

/// The properties Kani proves over every interface of a short query, every row list of up to [`proofs::ROW_BOUND`] rows, and every event.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Effect, Event, Mode, Row, Ui, step};
    use crate::state::Entry;

    /// The most rows the harness draws.
    pub(super) const ROW_BOUND: usize = 3;

    /// What a step from `ui` to the interface and effect of `outcome` promises: a selection on a row, an opening only of the shortcut selected in browsing by Enter, and a removal only after a confirmation.
    pub(super) fn promises(ui: &Ui, rows: &[Row], event: &Event, outcome: &(Ui, Effect)) -> bool {
        let (next, effect) = outcome;
        let on_a_row = next.selected < rows.len().max(1);
        let opens_selected = if let Effect::Open(id) = effect {
            ui.mode == Mode::Browse
                && *event == Event::Key(super::Key::Enter)
                && rows.get(ui.selected) == Some(&Row::Entry(Entry::Shortcut(*id)))
        } else {
            true
        };
        let removes_confirmed = if let Effect::Remove(target) = effect {
            ui.mode == Mode::Confirm { target: *target }
        } else {
            true
        };
        on_a_row && opens_selected && removes_confirmed
    }

    /// Whether control-c leaves from `ui`.
    pub(super) fn quits(ui: &Ui, rows: &[Row]) -> bool {
        step(ui, rows, &Event::Key(super::Key::Control('c'))).1 == Effect::Quit
    }

    /// Every promise of a step, and the way out from its state.
    #[cfg(test)]
    pub(super) fn spec(ui: &Ui, rows: &[Row], event: &Event) -> bool {
        promises(ui, rows, event, &step(ui, rows, event)) && quits(ui, rows)
    }

    /// Up to [`ROW_BOUND`] rows, as the harnesses draw them from `all`.
    #[cfg(kani)]
    fn any_rows(all: &[Row; ROW_BOUND]) -> &[Row] {
        let count: usize = kani::any();
        kani::assume(count <= ROW_BOUND);
        all.get(..count).unwrap_or_default()
    }

    /// An interface in `mode` with every query, folder, selection on the rows, and carried entry the harnesses draw.
    #[cfg(kani)]
    fn any_ui(mode: Mode, rows: &[Row], carried: Entry) -> Ui {
        let ui = Ui {
            query: if kani::any() {
                alloc::string::String::from("q")
            } else {
                alloc::string::String::new()
            },
            folder: if kani::any() {
                Some(crate::state::ItemId([1; 16]))
            } else {
                None
            },
            selected: kani::any(),
            mode,
            carried: if kani::any() { Some(carried) } else { None },
        };
        kani::assume(ui.selected < rows.len().max(1));
        ui
    }

    #[cfg(kani)]
    fn any_event() -> Event {
        match kani::any::<u8>() {
            0 => Event::Refresh,
            1 => Event::Paste(alloc::string::String::from("p")),
            choice => Event::Key(any_key(choice)),
        }
    }

    #[cfg(kani)]
    fn any_target() -> Entry {
        Entry::Shortcut(crate::state::ItemId([kani::any(); 16]))
    }

    #[cfg(kani)]
    fn any_row(choice: u8) -> Row {
        let id = crate::state::ItemId([choice; 16]);
        match choice % 4 {
            0 => Row::Parent(None),
            1 => Row::Entry(Entry::Folder(id)),
            2 => Row::Entry(Entry::Shortcut(id)),
            _ => Row::Add(alloc::string::String::from("l")),
        }
    }

    #[cfg(kani)]
    fn any_key(choice: u8) -> super::Key {
        use super::Key;
        match choice {
            0 => Key::Char('y'),
            1 => Key::Control('c'),
            2 => Key::Control('e'),
            3 => Key::Control('l'),
            4 => Key::Control('d'),
            5 => Key::Control('o'),
            6 => Key::Control('x'),
            7 => Key::Control('v'),
            8 => Key::Control('n'),
            9 => Key::Control('p'),
            10 => Key::Backspace,
            11 => Key::Enter,
            12 => Key::Escape,
            13 => Key::Up,
            14 => Key::Down,
            15 => Key::Left,
            16 => Key::Right,
            17 => Key::Tab,
            18 => Key::ShiftUp,
            _ => Key::ShiftDown,
        }
    }

    /// Typing each field, with and without a target.
    #[cfg(kani)]
    fn any_typing(target: Entry) -> Mode {
        use super::Field;
        match kani::any::<u8>() {
            0 => Mode::Typing {
                target: Some(target),
                field: Field::Title,
                text: alloc::string::String::new(),
            },
            1 => Mode::Typing {
                target: None,
                field: Field::Name,
                text: alloc::string::String::from("n"),
            },
            _ => Mode::Typing {
                target: Some(target),
                field: Field::Alias,
                text: alloc::string::String::from("a"),
            },
        }
    }

    /// A mode other than browsing: typing, or confirming a removal.
    #[cfg(kani)]
    fn any_other_mode(target: Entry) -> Mode {
        if kani::any() {
            any_typing(target)
        } else {
            Mode::Confirm { target }
        }
    }

    /// Every helper walks at most the rows, a one-letter query, and a one-letter paste, and comparing two identifiers compares their sixteen bytes.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn browsing_keeps_the_selection_on_a_row_and_opens_only_the_selected_shortcut() {
        let all: [u8; ROW_BOUND] = kani::any();
        let all = all.map(any_row);
        let rows = any_rows(&all);
        let ui = any_ui(Mode::Browse, rows, any_target());
        let event = any_event();
        let outcome = step(&ui, rows, &event);
        kani::assert(
            promises(&ui, rows, &event, &outcome),
            "every browsing step keeps its promises",
        );
        kani::cover!(matches!(outcome.1, Effect::Open(_)), "a shortcut opens");
    }

    /// Every helper walks at most the rows, a one-letter text, and a one-letter paste.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn typing_keeps_the_selection_on_a_row_and_opens_and_removes_nothing() {
        let all: [u8; ROW_BOUND] = kani::any();
        let all = all.map(any_row);
        let rows = any_rows(&all);
        let target = any_target();
        let ui = any_ui(any_typing(target), rows, target);
        let event = any_event();
        let outcome = step(&ui, rows, &event);
        kani::assert(
            promises(&ui, rows, &event, &outcome),
            "every typing step keeps its promises",
        );
        kani::cover!(
            matches!(
                outcome.1,
                Effect::Realias { .. } | Effect::CreateFolder { .. }
            ),
            "a finished typing"
        );
    }

    /// Every helper walks at most the rows and a one-letter paste, and comparing two identifiers compares their sixteen bytes.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn confirming_removes_only_the_entry_waiting_for_confirmation() {
        let all: [u8; ROW_BOUND] = kani::any();
        let all = all.map(any_row);
        let rows = any_rows(&all);
        let target = any_target();
        let ui = any_ui(Mode::Confirm { target }, rows, target);
        let event = any_event();
        let outcome = step(&ui, rows, &event);
        kani::assert(
            promises(&ui, rows, &event, &outcome),
            "every confirming step keeps its promises",
        );
        kani::cover!(
            matches!(outcome.1, Effect::Remove(_)),
            "a confirmed removal"
        );
    }

    /// Every helper walks at most the rows and a one-letter text.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn control_c_leaves_from_every_state() {
        let all: [u8; ROW_BOUND] = kani::any();
        let all = all.map(any_row);
        let rows = any_rows(&all);
        let target = any_target();
        let mode = if kani::any() {
            Mode::Browse
        } else {
            any_other_mode(target)
        };
        let ui = any_ui(mode, rows, target);
        kani::assert(quits(&ui, rows), "control-c leaves");
        kani::cover!(
            matches!(ui.mode, Mode::Confirm { .. }),
            "leaving from a confirmation"
        );
    }

    /// Enter on a folder must not open anything; this probe claims it opens a shortcut and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn rejects_an_opening_from_a_folder_row() {
        let id = crate::state::ItemId([2; 16]);
        let rows = [Row::Entry(Entry::Folder(id))];
        let (_, effect) = step(&Ui::default(), &rows, &Event::Key(super::Key::Enter));
        kani::assert(effect == Effect::Open(id), "NEGATIVE_UI_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{ROW_BOUND, spec};
    use super::{Effect, Event, Field, Key, Mode, Row, Ui, step};
    use crate::state::{Entry, ItemId};
    use alloc::string::String;
    use alloc::vec::Vec;

    const FOLDER: ItemId = ItemId([1; 16]);
    const SHORTCUT: ItemId = ItemId([2; 16]);
    const OTHER: ItemId = ItemId([3; 16]);

    fn press(ui: &Ui, rows: &[Row], key: Key) -> (Ui, Effect) {
        step(ui, rows, &Event::Key(key))
    }

    fn keys(rows: &[Row], pressed: &[Key]) -> (Ui, Vec<Effect>) {
        let mut ui = Ui::default();
        let mut effects = Vec::new();
        for key in pressed {
            let (next, effect) = press(&ui, rows, *key);
            ui = next;
            effects.push(effect);
        }
        (ui, effects)
    }

    fn after(rows: &[Row], pressed: &[Key]) -> Ui {
        keys(rows, pressed).0
    }

    fn last_effect(rows: &[Row], pressed: &[Key]) -> Effect {
        keys(rows, pressed).1.pop().unwrap()
    }

    #[test]
    fn typing_searches_and_enter_opens_the_selected_shortcut() {
        let rows = [
            Row::Entry(Entry::Folder(FOLDER)),
            Row::Entry(Entry::Shortcut(SHORTCUT)),
        ];
        let opening = [Key::Char('g'), Key::Down, Key::Down, Key::Enter];
        let opened = after(&rows, &opening);
        assert_eq!(opened.query, "g");
        assert_eq!(opened.selected, 1);
        assert_eq!(last_effect(&rows, &opening), Effect::Open(SHORTCUT));
        assert_eq!(after(&rows, &[Key::Char('g'), Key::Backspace]).query, "");
        let moved = after(&rows, &[Key::Down, Key::Control('p'), Key::Control('n')]);
        assert_eq!(moved.selected, 1);
        assert_eq!(after(&rows, &[Key::Down, Key::Control('p')]).selected, 0);
    }

    #[test]
    fn a_rename_without_text_changes_nothing() {
        let rows = [Row::Entry(Entry::Shortcut(SHORTCUT))];
        let (ui, effects) = keys(&rows, &[Key::Control('e'), Key::Enter]);
        assert_eq!(effects, [Effect::None, Effect::None]);
        assert_eq!(ui.mode, Mode::Browse);
        let (_, unnamed) = keys(&rows, &[Key::Control('o'), Key::Enter]);
        assert_eq!(unnamed, [Effect::None, Effect::None]);
    }

    #[test]
    fn folders_open_with_enter_and_close_with_backspace_or_left() {
        let rows = [Row::Entry(Entry::Folder(FOLDER))];
        let (inside, _) = keys(&rows, &[Key::Enter]);
        assert_eq!(inside.folder, Some(FOLDER));
        let nested = [Row::Parent(None), Row::Entry(Entry::Shortcut(SHORTCUT))];
        assert_eq!(press(&inside, &nested, Key::Backspace).0.folder, None);
        assert_eq!(press(&inside, &nested, Key::Left).0.folder, None);
        let (via_tab, _) = keys(&rows, &[Key::Tab]);
        assert_eq!(via_tab.folder, Some(FOLDER));
        let (via_parent, _) = press(&inside, &nested, Key::Enter);
        assert_eq!(via_parent.folder, None);
        assert_eq!(press(&Ui::default(), &rows, Key::Left).0, Ui::default());
    }

    #[test]
    fn a_pasted_link_is_added_and_the_search_clears() {
        let rows = [Row::Add(String::from("https://example.com/"))];
        let (pasted, _) = step(
            &Ui::default(),
            &rows,
            &Event::Paste(String::from("https://example.com/\n")),
        );
        assert_eq!(pasted.query, "https://example.com/");
        let (added, effect) = press(&pasted, &rows, Key::Enter);
        assert_eq!(effect, Effect::Add(String::from("https://example.com/")));
        assert_eq!(added.query, "");
    }

    #[test]
    fn escape_clears_then_quits_and_control_c_always_quits() {
        let rows: [Row; 0] = [];
        let (_, effects) = keys(&rows, &[Key::Char('a'), Key::Escape, Key::Escape]);
        assert_eq!(effects, [Effect::None, Effect::None, Effect::Quit]);
        for mode in [
            Mode::Browse,
            Mode::Confirm {
                target: Entry::Folder(FOLDER),
            },
            Mode::Typing {
                target: None,
                field: Field::Name,
                text: String::new(),
            },
        ] {
            let ui = Ui {
                mode,
                ..Ui::default()
            };
            assert_eq!(press(&ui, &rows, Key::Control('c')).1, Effect::Quit);
        }
    }

    #[test]
    fn renaming_aliasing_and_new_folders_go_through_typing() {
        let rows = [
            Row::Entry(Entry::Shortcut(SHORTCUT)),
            Row::Entry(Entry::Folder(FOLDER)),
        ];
        assert_eq!(
            last_effect(
                &rows,
                &[
                    Key::Control('e'),
                    Key::Char('x'),
                    Key::Char('y'),
                    Key::Backspace,
                    Key::Enter
                ],
            ),
            Effect::Rename {
                target: Entry::Shortcut(SHORTCUT),
                text: String::from("x")
            }
        );
        assert_eq!(
            last_effect(&rows, &[Key::Control('l'), Key::Char('g'), Key::Enter]),
            Effect::Realias {
                id: SHORTCUT,
                alias: String::from("g")
            }
        );
        assert_eq!(
            last_effect(&rows, &[Key::Control('l'), Key::Enter]),
            Effect::Realias {
                id: SHORTCUT,
                alias: String::new()
            }
        );
        assert_eq!(
            last_effect(&rows, &[Key::Control('e'), Key::Enter]),
            Effect::None
        );
        let creating = [Key::Control('o'), Key::Char('n'), Key::Enter];
        assert_eq!(
            last_effect(&rows, &creating),
            Effect::CreateFolder {
                name: String::from("n"),
                parent: None
            }
        );
        assert_eq!(after(&rows, &creating).mode, Mode::Browse);
        let cancelled = after(&rows, &[Key::Control('o'), Key::Char('n'), Key::Escape]);
        assert_eq!(cancelled.mode, Mode::Browse);
        let typing = Ui {
            mode: Mode::Typing {
                target: None,
                field: Field::Name,
                text: String::new(),
            },
            ..Ui::default()
        };
        let (pasted, _) = step(&typing, &rows, &Event::Paste(String::from("a\tb")));
        assert_eq!(
            pasted.mode,
            Mode::Typing {
                target: None,
                field: Field::Name,
                text: String::from("ab")
            }
        );
    }

    #[test]
    fn removal_waits_for_confirmation() {
        let rows = [Row::Entry(Entry::Shortcut(SHORTCUT))];
        assert_eq!(
            keys(&rows, &[Key::Control('d'), Key::Char('y')]).1,
            [Effect::None, Effect::Remove(Entry::Shortcut(SHORTCUT))]
        );
        let declined = [Key::Control('d'), Key::Char('n')];
        assert_eq!(keys(&rows, &declined).1, [Effect::None, Effect::None]);
        assert_eq!(after(&rows, &declined).mode, Mode::Browse);
        let asking = after(&rows, &[Key::Control('d')]);
        assert_eq!(step(&asking, &rows, &Event::Refresh).0.mode, asking.mode);
    }

    #[test]
    fn entries_are_carried_into_a_folder_and_reordered_in_place() {
        let rows = [
            Row::Entry(Entry::Shortcut(SHORTCUT)),
            Row::Entry(Entry::Folder(FOLDER)),
            Row::Entry(Entry::Shortcut(OTHER)),
        ];
        let (ui, _) = keys(&rows, &[Key::Control('x'), Key::Down, Key::Enter]);
        assert_eq!(ui.carried, Some(Entry::Shortcut(SHORTCUT)));
        let inside = [Row::Parent(None)];
        let (dropped, drop_effect) = press(&ui, &inside, Key::Control('v'));
        assert_eq!(
            drop_effect,
            Effect::Move {
                entry: Entry::Shortcut(SHORTCUT),
                folder: Some(FOLDER)
            }
        );
        assert_eq!(dropped.carried, None);
        assert_eq!(press(&dropped, &inside, Key::Control('v')).1, Effect::None);
        let (put_down, _) = press(&ui, &inside, Key::Escape);
        assert_eq!(put_down.carried, None);
        let moving = [Key::Down, Key::ShiftDown];
        assert_eq!(
            last_effect(&rows, &moving),
            Effect::Reorder {
                entry: Entry::Folder(FOLDER),
                neighbor: Entry::Shortcut(OTHER),
                before: false
            }
        );
        assert_eq!(after(&rows, &moving).selected, 2);
        assert_eq!(last_effect(&rows, &[Key::ShiftUp]), Effect::None);
        assert_eq!(
            last_effect(&rows, &[Key::Char('q'), Key::ShiftDown]),
            Effect::None
        );
    }

    #[test]
    fn the_specification_agrees_over_a_drawn_domain() {
        let row_choices = [
            Row::Parent(None),
            Row::Entry(Entry::Folder(FOLDER)),
            Row::Entry(Entry::Shortcut(SHORTCUT)),
            Row::Add(String::from("l")),
        ];
        let mut lists: Vec<Vec<Row>> = alloc::vec![Vec::new()];
        for _ in 0..ROW_BOUND {
            let longer: Vec<Vec<Row>> = lists
                .iter()
                .flat_map(|list| {
                    row_choices.iter().map(move |row| {
                        let mut next = list.clone();
                        next.push(row.clone());
                        next
                    })
                })
                .collect();
            lists.extend(longer);
        }
        let modes = [
            Mode::Browse,
            Mode::Typing {
                target: Some(Entry::Shortcut(SHORTCUT)),
                field: Field::Alias,
                text: String::from("a"),
            },
            Mode::Confirm {
                target: Entry::Shortcut(SHORTCUT),
            },
        ];
        let keys = [
            Key::Char('y'),
            Key::Control('c'),
            Key::Control('d'),
            Key::Control('x'),
            Key::Control('v'),
            Key::Backspace,
            Key::Enter,
            Key::Escape,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::ShiftUp,
            Key::ShiftDown,
        ];
        for rows in &lists {
            for mode in &modes {
                for selected in 0..rows.len().max(1) {
                    for query in ["", "q"] {
                        let ui = Ui {
                            query: String::from(query),
                            folder: Some(FOLDER),
                            selected,
                            mode: mode.clone(),
                            carried: Some(Entry::Shortcut(OTHER)),
                        };
                        assert!(spec(&ui, rows, &Event::Refresh));
                        assert!(spec(&ui, rows, &Event::Paste(String::from("p"))));
                        for key in keys {
                            assert!(spec(&ui, rows, &Event::Key(key)), "{ui:?} {rows:?} {key:?}");
                        }
                    }
                }
            }
        }
    }
}
