// The state machine of the launcher's interface, `crates/youarel-core/src/ui_rules.rs`, over three entries and the row lists a shell shows.
// `crates/youarel-core/tests/ui_replay.rs` runs the traces of this model through `ui_rules::step` and compares every state and effect.
// The shell chooses the rows before each step, so the model draws them from a set; it keeps typed text to two characters so every state is reachable.
module ui_model {
  // Whether a step keeps the selection on a row, which the probe switches off.
  const CLAMP: bool

  // The entries the rows name: a folder, a shortcut, and another shortcut.
  type Item = Folder | Shortcut | Other
  type Row = ParentRow | EntryRow(Item) | AddRow
  type Field = Title | Alias | Name
  type Mode = Browse | Typing({ target: Option, part: Field, text: List[str] }) | Confirm(Item)
  type Option = None | Some(Item)
  type Key = Char(str) | Control(str) | Backspace | Enter | Escape | Up | Down | Left | Right | Tab | ShiftUp | ShiftDown
  type Event = Press(Key) | Paste(str) | Refresh
  type Effect =
    | Nothing
    | Open(Item)
    | Add
    | Rename({ target: Item, text: List[str] })
    | Realias({ id: Item, alias: List[str] })
    | CreateFolder({ name: List[str], inside: bool })
    | Remove(Item)
    | Move({ entry: Item, inside: bool })
    | Reorder({ entry: Item, neighbor: Item, before: bool })
    | Quit

  // The interface: the typed search, whether the person browses inside the folder, the selection, the mode, and the carried entry.
  type Ui = { query: List[str], inside: bool, selected: int, mode: Mode, carried: Option }

  // The longest search or text the model types.
  pure val LONGEST = 1

  pure val START: Ui = { query: [], inside: false, selected: 0, mode: Browse, carried: None }

  // The row lists a shell may show: none, a folder's listing with its way up, a search with an offer to add, and a listing of three entries.
  pure val ROWS = Set(
    [],
    [ParentRow, EntryRow(Shortcut)],
    [AddRow, EntryRow(Folder), EntryRow(Shortcut)],
    [EntryRow(Shortcut), EntryRow(Folder), EntryRow(Other)],
  )

  // Every key the machine tells apart; control-n and control-p repeat Down and Up, and Tab repeats Right, which the Kani proof covers.
  pure val KEYS = Set(
    Char("q"), Char("y"), Control("c"), Control("e"), Control("l"), Control("d"), Control("o"),
    Control("x"), Control("v"), Control("z"),
    Backspace, Enter, Escape, Up, Down, Left, Right, ShiftUp, ShiftDown,
  )

  pure def lastIndex(rows: List[Row]): int = if (rows.length() > 0) rows.length() - 1 else 0

  pure def rowAt(rows: List[Row], index: int): Option =
    if (index >= 0 and index < rows.length()) match rows[index] { EntryRow(thing) => Some(thing) | _ => None } else None

  pure def isParentAt(rows: List[Row], index: int): bool =
    index >= 0 and index < rows.length() and rows[index] == ParentRow

  pure def isAddAt(rows: List[Row], index: int): bool =
    index >= 0 and index < rows.length() and rows[index] == AddRow

  pure def hasParent(rows: List[Row]): bool = rows.indices().exists(index => rows[index] == ParentRow)

  pure def pop(text: List[str]): List[str] = if (text.length() > 0) text.slice(0, text.length() - 1) else text

  // The interface after moving into the folder, or up to the top, with a fresh search.
  pure def enter(ui: Ui, inside: bool): Ui = { query: [], inside: inside, selected: 0, mode: Browse, carried: ui.carried }

  pure def down(ui: Ui, rows: List[Row]): int = if (ui.selected + 1 < lastIndex(rows)) ui.selected + 1 else lastIndex(rows)
  pure def up(ui: Ui): int = if (ui.selected > 0) ui.selected - 1 else 0

  pure def reorder(ui: Ui, rows: List[Row], upward: bool): (Ui, Effect) =
    if (ui.query.length() > 0) (ui, Nothing)
    else match rowAt(rows, ui.selected) {
      | Some(entry) => {
          val place = if (upward) ui.selected - 1 else ui.selected + 1
          match rowAt(rows, place) {
            | Some(neighbor) => (ui.with("selected", place), Reorder({ entry: entry, neighbor: neighbor, before: upward }))
            | None => (ui, Nothing)
          }
        }
      | None => (ui, Nothing)
    }

  pure def command(ui: Ui, rows: List[Row], letter: str): (Ui, Effect) = {
    val entry = rowAt(rows, ui.selected)
    if (letter == "c") (ui, Quit)
    else if (letter == "e") match entry {
      | Some(thing) => (ui.with("mode", Typing({ target: Some(thing), part: if (thing == Folder) Name else Title, text: [] })), Nothing)
      | None => (ui, Nothing)
    }
    else if (letter == "l") match entry {
      | Some(thing) => if (thing != Folder) (ui.with("mode", Typing({ target: Some(thing), part: Alias, text: [] })), Nothing) else (ui, Nothing)
      | None => (ui, Nothing)
    }
    else if (letter == "d") match entry {
      | Some(thing) => (ui.with("mode", Confirm(thing)), Nothing)
      | None => (ui, Nothing)
    }
    else if (letter == "o") (ui.with("mode", Typing({ target: None, part: Name, text: [] })), Nothing)
    else if (letter == "x") match entry {
      | Some(thing) => (ui.with("carried", Some(thing)), Nothing)
      | None => (ui, Nothing)
    }
    else if (letter == "v") match ui.carried {
      | Some(thing) => (ui.with("carried", None), Move({ entry: thing, inside: ui.inside }))
      | None => (ui, Nothing)
    }
    else if (letter == "n") (ui.with("selected", down(ui, rows)), Nothing)
    else if (letter == "p") (ui.with("selected", up(ui)), Nothing)
    else (ui, Nothing)
  }

  pure def browse(ui: Ui, rows: List[Row], key: Key): (Ui, Effect) = match key {
    | Char(c) => (ui.with("query", ui.query.append(c)).with("selected", 0), Nothing)
    | Backspace =>
        if (ui.query.length() > 0) (ui.with("query", pop(ui.query)).with("selected", 0), Nothing)
        else if (ui.inside and hasParent(rows)) (enter(ui, false), Nothing)
        else (ui, Nothing)
    | Up => (ui.with("selected", up(ui)), Nothing)
    | Down => (ui.with("selected", down(ui, rows)), Nothing)
    | Left => (if (ui.inside and hasParent(rows)) enter(ui, false) else ui, Nothing)
    | Right => (if (rowAt(rows, ui.selected) == Some(Folder)) enter(ui, true) else ui, Nothing)
    | Tab => (if (rowAt(rows, ui.selected) == Some(Folder)) enter(ui, true) else ui, Nothing)
    | Enter =>
        if (isParentAt(rows, ui.selected)) (enter(ui, false), Nothing)
        else if (isAddAt(rows, ui.selected)) (ui.with("query", []).with("selected", 0), Add)
        else match rowAt(rows, ui.selected) {
          | Some(thing) => if (thing == Folder) (enter(ui, true), Nothing) else (ui, Open(thing))
          | None => (ui, Nothing)
        }
    | Escape =>
        if (ui.carried != None) (ui.with("carried", None), Nothing)
        else if (ui.query.length() == 0) (ui, Quit)
        else (ui.with("query", []).with("selected", 0), Nothing)
    | ShiftUp => reorder(ui, rows, true)
    | ShiftDown => reorder(ui, rows, false)
    | Control(letter) => command(ui, rows, letter)
  }

  pure def finish(ui: Ui, target: Option, part: Field, text: List[str]): Effect = match target {
    | Some(thing) =>
        if (part == Alias) (if (thing != Folder) Realias({ id: thing, alias: text }) else Nothing)
        else if (text.length() > 0) Rename({ target: thing, text: text })
        else Nothing
    | None => if (part == Name and text.length() > 0) CreateFolder({ name: text, inside: ui.inside }) else Nothing
  }

  // The interface typing `text` into the part of the target.
  pure def drafting(ui: Ui, target: Option, part: Field, text: List[str]): Ui =
    ui.with("mode", Typing({ target: target, part: part, text: text }))

  pure def typing(ui: Ui, target: Option, part: Field, text: List[str], e: Event): (Ui, Effect) = match e {
    | Press(key) => match key {
        | Char(c) => (drafting(ui, target, part, text.append(c)), Nothing)
        | Backspace => (drafting(ui, target, part, pop(text)), Nothing)
        | Enter => (ui.with("mode", Browse), finish(ui, target, part, text))
        | Escape => (ui.with("mode", Browse), Nothing)
        | Control(letter) => if (letter == "c") (ui, Quit) else (drafting(ui, target, part, text), Nothing)
        | _ => (drafting(ui, target, part, text), Nothing)
      }
    | Paste(c) => (drafting(ui, target, part, text.append(c)), Nothing)
    | Refresh => (drafting(ui, target, part, text), Nothing)
  }

  pure def confirm(ui: Ui, target: Item, e: Event): (Ui, Effect) = match e {
    | Refresh => (ui, Nothing)
    | Press(key) => match key {
        | Char(c) => (ui.with("mode", Browse), if (c == "y") Remove(target) else Nothing)
        | Enter => (ui.with("mode", Browse), Remove(target))
        | Control(letter) => (ui.with("mode", Browse), if (letter == "c") Quit else Nothing)
        | _ => (ui.with("mode", Browse), Nothing)
      }
    | Paste(_) => (ui.with("mode", Browse), Nothing)
  }

  // One step of the machine over the rows the shell shows: the interface after `e` and the effect it asks of the shell.
  pure def advance(ui: Ui, rows: List[Row], e: Event): (Ui, Effect) = {
    val stepped = match ui.mode {
      | Browse => match e {
          | Press(key) => browse(ui, rows, key)
          | Paste(c) => (ui.with("query", ui.query.append(c)).with("selected", 0), Nothing)
          | Refresh => (ui, Nothing)
        }
      | Typing(draft) => typing(ui, draft.target, draft.part, draft.text, e)
      | Confirm(target) => confirm(ui, target, e)
    }
    val after = stepped._1
    val clamped = if (CLAMP and after.selected > lastIndex(rows)) after.with("selected", lastIndex(rows)) else after
    (clamped, stepped._2)
  }

  // Whether typing one more character keeps every text within the model's bound.
  pure def room(ui: Ui): bool = match ui.mode {
    | Browse => ui.query.length() < LONGEST
    | Typing(draft) => draft.text.length() < LONGEST
    | Confirm(_) => true
  }

  pure def grows(e: Event): bool = match e {
    | Press(key) => match key { Char(_) => true | _ => false }
    | Paste(_) => true
    | Refresh => false
  }

  // Whether an effect of a step from `ui` over `rows` on `e` opens only the shortcut selected when Enter was pressed while browsing.
  pure def opensSelected(ui: Ui, rows: List[Row], e: Event, x: Effect): bool = match x {
    | Open(thing) => ui.mode == Browse and e == Press(Enter) and rowAt(rows, ui.selected) == Some(thing)
    | _ => true
  }

  // Whether an effect of a step from `ui` removes only an entry that was waiting for its confirmation.
  pure def removesConfirmed(ui: Ui, x: Effect): bool = match x {
    | Remove(thing) => ui.mode == Confirm(thing)
    | _ => true
  }

  var ui: Ui
  // What the last step read, over which rows, what it asked, and whether its effect kept the promises about the interface before it.
  var shown: List[Row]
  var event: Event
  var effect: Effect
  var kept: bool

  action init = all {
    ui' = START,
    shown' = [],
    event' = Refresh,
    effect' = Nothing,
    kept' = true,
  }

  action apply(rows: List[Row], e: Event): bool = {
    val r = advance(ui, rows, e)
    all {
      ui' = r._1,
      shown' = rows,
      event' = e,
      effect' = r._2,
      kept' = (opensSelected(ui, rows, e, r._2) and removesConfirmed(ui, r._2)),
    }
  }

  action step = {
    nondet rows = ROWS.oneOf()
    nondet key = KEYS.oneOf()
    nondet choice = Set(0, 1, 2).oneOf()
    val e = if (choice == 0) Press(key) else if (choice == 1) Paste("p") else Refresh
    all {
      not(grows(e)) or room(ui),
      apply(rows, e),
    }
  }

  // The selection stays on one of the rows the step read, or at zero when there were none.
  val onARow = ui.selected >= 0 and ui.selected <= lastIndex(shown)

  // Leaving is one key away from every state.
  val quitSafe = advance(ui, shown, Press(Control("c")))._2 == Quit

  val safe = onARow and kept and quitSafe

  // The outcomes every simulation must reach.
  val opened = match effect { Open(_) => true | _ => false }
  val added = effect == Add
  val removed = match effect { Remove(_) => true | _ => false }
  val moved = match effect { Move(_) => true | _ => false }
  val reordered = match effect { Reorder(_) => true | _ => false }
  val renamed = match effect { Rename(_) => true | _ => false }
  val folderCreated = match effect { CreateFolder(_) => true | _ => false }
  val wentInside = ui.inside and event == Press(Enter)
}

// The machine as the interface implements it.
module ui {
  import ui_model(CLAMP = true).*

  run openTest = init
    .then(apply([AddRow, EntryRow(Folder), EntryRow(Shortcut)], Press(Down)))
    .then(apply([AddRow, EntryRow(Folder), EntryRow(Shortcut)], Press(Down)))
    .then(apply([AddRow, EntryRow(Folder), EntryRow(Shortcut)], Press(Enter)))
    .then(all {
      assert(effect == Open(Shortcut) and ui.selected == 2),
      init,
    })

  run removeTest = init
    .then(apply([EntryRow(Shortcut), EntryRow(Folder), EntryRow(Other)], Press(Control("d"))))
    .then(apply([EntryRow(Shortcut), EntryRow(Folder), EntryRow(Other)], Press(Char("y"))))
    .then(all {
      assert(effect == Remove(Shortcut) and ui.mode == Browse),
      init,
    })
}

// The machine without the clamp, which must break `safe`.
module ui_probe {
  import ui_model(CLAMP = false).*

  // Without the clamp a selection stays past the rows that shrank under it.
  run strandedTest = init
    .then(apply([EntryRow(Shortcut), EntryRow(Folder), EntryRow(Other)], Press(Down)))
    .then(apply([EntryRow(Shortcut), EntryRow(Folder), EntryRow(Other)], Press(Down)))
    .then(apply([ParentRow, EntryRow(Shortcut)], Refresh))
    .then(all {
      assert(not(onARow)),
      init,
    })
}
