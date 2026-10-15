//! The terminal launcher: the shell around the core's interface machine.
//!
//! The shell reads keys, asks [`step`] what they mean, carries out the effect through the store and the browser, and draws the rows again.
//! It decides nothing the machine decides.

use crate::{clock, web};
use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::collections::BTreeMap;
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, Entry, ItemId, Slot, State};
use youarel_core::ui_rules::{Effect, Event, Field, Key, Mode, Row, Ui, step};
use youarel_core::view;
use youarel_link::Link;
use youarel_store::Store;

/// The key hints the footer shows while browsing.
const HINTS: &str = "Enter open · → ← folders · ^E rename · ^L alias · ^D remove · ^O new folder · ^X ^V move · Shift ↑↓ reorder · Esc quit";

/// What the screen shows: the state as read, and the rows for the interface.
#[derive(Debug)]
struct Screen {
    state: State,
    rows: Vec<Row>,
}

/// What the loop does after an effect.
#[derive(Debug)]
enum Outcome {
    Continue(Option<String>),
    Quit,
}

/// Runs the launcher until the person leaves it.
///
/// # Errors
///
/// Returns an error when the terminal cannot be driven or the store cannot be read.
pub fn run(store: &Store) -> Result<()> {
    let mut terminal = ratatui::init();
    let pasting = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = pasting
        .context("cannot enable pasting into the terminal")
        .and_then(|()| drive(store, &mut terminal));
    // The terminal returns to its own mode either way, and the primary error is the drive's.
    drop(execute!(std::io::stdout(), DisableBracketedPaste));
    ratatui::restore();
    result
}

/// The rows for the interface: the core's rows, after an offer to add a typed link the store does not hold.
fn rows(state: &State, ui: &Ui, visits: &BTreeMap<ItemId, Visits>, now: u64) -> Vec<Row> {
    let mut rows = view::rows(state, ui, visits, now);
    if let Ok(link) = Link::parse(ui.query.trim()) {
        let held = state
            .shortcuts
            .values()
            .any(|shortcut| !shortcut.removed.value && shortcut.link.value == link.as_str());
        if !held {
            rows.insert(0, Row::Add(link.as_str().to_owned()));
        }
    }
    rows
}

fn load(store: &Store, ui: &Ui) -> Result<Screen> {
    let now = clock::now()?;
    let (state, visits) = store
        .session(now, |session| {
            Ok((session.state().clone(), session.visits().clone()))
        })
        .map_err(anyhow::Error::from)?;
    let rows = rows(&state, ui, &visits, now);
    Ok(Screen { state, rows })
}

fn drive(store: &Store, terminal: &mut DefaultTerminal) -> Result<()> {
    let mut ui = Ui::default();
    let mut notice: Option<String> = None;
    loop {
        let screen = load(store, &ui)?;
        ui = step(&ui, &screen.rows, &Event::Refresh).0;
        terminal.draw(|frame| draw(frame, &ui, &screen, notice.as_deref()))?;
        let Some(event) = next_event()? else {
            continue;
        };
        let (next, effect) = step(&ui, &screen.rows, &event);
        ui = next;
        if effect == Effect::None {
            if event != Event::Refresh {
                notice = None;
            }
            continue;
        }
        if let Effect::Add(_) = &effect {
            terminal.draw(|frame| draw(frame, &ui, &screen, Some("reading the page's title…")))?;
        }
        match carry_out(store, &ui, &screen, effect) {
            Outcome::Continue(message) => notice = message,
            Outcome::Quit => return Ok(()),
        }
    }
}

/// The next event the machine reads, or `None` for one it ignores.
fn next_event() -> Result<Option<Event>> {
    Ok(match event::read().context("cannot read the terminal")? {
        event::Event::Key(pressed) => key(pressed).map(Event::Key),
        event::Event::Paste(text) => Some(Event::Paste(text)),
        event::Event::Resize(..) => Some(Event::Refresh),
        event::Event::FocusGained | event::Event::FocusLost | event::Event::Mouse(_) => None,
    })
}

/// The machine's key for a terminal key press.
fn key(pressed: KeyEvent) -> Option<Key> {
    if pressed.kind == KeyEventKind::Release {
        return None;
    }
    let control = pressed.modifiers.contains(KeyModifiers::CONTROL);
    let shift = pressed.modifiers.contains(KeyModifiers::SHIFT);
    Some(match pressed.code {
        KeyCode::Char(character) if control => Key::Control(character.to_ascii_lowercase()),
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Escape,
        KeyCode::Up if shift => Key::ShiftUp,
        KeyCode::Up => Key::Up,
        KeyCode::Down if shift => Key::ShiftDown,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::BackTab
        | KeyCode::Delete
        | KeyCode::Insert
        | KeyCode::F(_)
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => return None,
    })
}

/// Runs one edit in a session now, and the notice it leaves.
fn edit(store: &Store, edit: Edit, done: &str) -> Outcome {
    let result = clock::now().and_then(|now| {
        store
            .session(now, |session| session.edit(edit))
            .map_err(anyhow::Error::from)
    });
    Outcome::Continue(Some(match result {
        Ok(()) => done.to_owned(),
        Err(error) => format!("{error:#}"),
    }))
}

fn carry_out(store: &Store, ui: &Ui, screen: &Screen, effect: Effect) -> Outcome {
    match effect {
        Effect::None => Outcome::Continue(None),
        Effect::Quit => Outcome::Quit,
        Effect::Open(id) => open(store, id),
        Effect::Add(link) => add(store, ui.folder, &link),
        Effect::Rename { target, text } => edit(
            store,
            match target {
                Entry::Folder(id) => Edit::RenameFolder { id, name: text },
                Entry::Shortcut(id) => Edit::Retitle { id, title: text },
            },
            "renamed",
        ),
        Effect::Realias { id, alias } => edit(store, Edit::Realias { id, alias }, "alias set"),
        Effect::CreateFolder { name, parent } => match fresh_id(store) {
            Ok(id) => edit(
                store,
                Edit::AddFolder { id, name, parent },
                "folder created",
            ),
            Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
        },
        Effect::Remove(entry) => edit(store, Edit::Remove { entry }, "removed"),
        Effect::Move { entry, folder } => edit(
            store,
            Edit::Move {
                entry,
                folder,
                slot: Slot::End,
            },
            "moved",
        ),
        Effect::Reorder {
            entry,
            neighbor,
            before,
        } => {
            let tree = screen.state.tree();
            let folder = match entry {
                Entry::Folder(id) => tree.parent(id),
                Entry::Shortcut(id) => screen.state.folder_of(&tree, id),
            };
            let slot = if before {
                Slot::Before(neighbor)
            } else {
                Slot::After(neighbor)
            };
            edit(
                store,
                Edit::Move {
                    entry,
                    folder,
                    slot,
                },
                "moved",
            )
        }
    }
}

fn fresh_id(store: &Store) -> Result<ItemId> {
    let now = clock::now()?;
    store
        .session(now, |session| session.fresh_id())
        .map_err(anyhow::Error::from)
}

/// Counts the visit and opens the shortcut, leaving the launcher once the browser has it.
fn open(store: &Store, id: ItemId) -> Outcome {
    let link = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                session.visit(id);
                Ok(session
                    .state()
                    .shortcut(id)
                    .map(|shortcut| shortcut.link.value.clone()))
            })
            .map_err(anyhow::Error::from)
    });
    match link {
        Ok(Some(link)) => match web::open(&link) {
            Ok(()) => Outcome::Quit,
            Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
        },
        Ok(None) => Outcome::Continue(Some("the shortcut is gone".to_owned())),
        Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
    }
}

/// Adds a link to the browsed folder, titled by its page when the page names one.
fn add(store: &Store, folder: Option<ItemId>, text: &str) -> Outcome {
    let link = match Link::parse(text) {
        Ok(link) => link,
        Err(error) => return Outcome::Continue(Some(format!("cannot add {text}: {error}"))),
    };
    let (title, warning) = match web::fetch_title(&link) {
        Ok(Some(title)) => (title, None),
        Ok(None) => (link.as_str().to_owned(), None),
        Err(error) => (
            link.as_str().to_owned(),
            Some(format!(" (no title: {error:#})")),
        ),
    };
    let added = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: link.as_str().to_owned(),
                    title: title.clone(),
                    folder,
                })
            })
            .map_err(anyhow::Error::from)
    });
    Outcome::Continue(Some(match added {
        Ok(()) => format!("added {title}{}", warning.unwrap_or_default()),
        Err(error) => format!("{error:#}"),
    }))
}

/// The link as the address bar shows it.
fn bare(link: &str) -> &str {
    link.strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link)
}

/// The line of one row.
fn row_line<'a>(state: &'a State, row: &'a Row) -> Line<'a> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    match row {
        Row::Parent(_) => Line::from(Span::styled("..", dim)),
        Row::Add(link) => Line::from(vec![Span::raw("+ add "), Span::raw(link.as_str())]),
        Row::Entry(Entry::Folder(id)) => {
            let name = state
                .folder(*id)
                .map_or("", |folder| folder.name.value.as_str());
            Line::from(vec![Span::raw(name), Span::styled("/", dim)])
        }
        Row::Entry(Entry::Shortcut(id)) => {
            state.shortcut(*id).map_or_else(Line::default, |shortcut| {
                let mut spans = vec![
                    Span::raw(shortcut.title.value.as_str()),
                    Span::raw("  "),
                    Span::styled(bare(&shortcut.link.value), dim),
                ];
                if !shortcut.alias.value.is_empty() {
                    spans.push(Span::styled(format!("  [{}]", shortcut.alias.value), dim));
                }
                Line::from(spans)
            })
        }
    }
}

/// The prompt line, and the text after which the cursor stands.
fn prompt(ui: &Ui, state: &State) -> Line<'static> {
    match &ui.mode {
        Mode::Browse => Line::from(format!("> {}", ui.query)),
        Mode::Typing {
            target,
            field,
            text,
        } => {
            let label = match (field, target) {
                (Field::Title, _) => "new title",
                (Field::Alias, _) => "alias (empty removes it)",
                (Field::Name, Some(_)) => "new name",
                (Field::Name, None) => "new folder",
            };
            Line::from(format!("{label}: {text}"))
        }
        Mode::Confirm { target } => {
            let tree = state.tree();
            let named = crate::resolve::describe(state, &tree, *target);
            Line::from(format!("remove {named}? y / n"))
        }
    }
}

fn draw(frame: &mut Frame<'_>, ui: &Ui, screen: &Screen, notice: Option<&str>) {
    let [header, input, list, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let tree = screen.state.tree();
    let mut title = format!(
        "youarel  /{}",
        screen.state.path(&tree, ui.folder).join("/")
    );
    if let Some(carried) = ui.carried {
        title.push_str("  · carrying ");
        title.push_str(&crate::resolve::describe(&screen.state, &tree, carried));
    }
    frame.render_widget(
        Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );
    let line = prompt(ui, &screen.state);
    // The cursor stands after the typed text, where an input method opens its candidates, and never past the line.
    let offset = u16::try_from(line.width()).map_or(input.width, |width| width.min(input.width));
    frame.render_widget(Paragraph::new(line), input);
    frame.set_cursor_position(Position::new(input.x.saturating_add(offset), input.y));
    let items: Vec<ListItem<'_>> = screen
        .rows
        .iter()
        .map(|row| ListItem::new(row_line(&screen.state, row)))
        .collect();
    let mut selection = ListState::default();
    if !screen.rows.is_empty() {
        selection.select(Some(ui.selected));
    }
    frame.render_stateful_widget(
        List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list,
        &mut selection,
    );
    frame.render_widget(
        Paragraph::new(notice.unwrap_or(HINTS)).style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
}
