//! The terminal launcher: the terminal's side of the shell around the core's interface machine.
//!
//! It reads keys, asks [`step`] what they mean, has [`shell::carry_out`] carry out the effect, and draws the rows again.
//! It decides nothing the machine decides.

use crate::shell::{self, Kind, Outcome, Screen};
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
use youarel_core::state::State;
use youarel_core::ui_rules::{Effect, Event, Key, Row, Ui, step};
use youarel_store::Store;

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

fn drive(store: &Store, terminal: &mut DefaultTerminal) -> Result<()> {
    let mut ui = Ui::default();
    let mut notice: Option<String> = None;
    loop {
        let screen = shell::load(store, &ui)?;
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
            terminal.draw(|frame| draw(frame, &ui, &screen, Some(shell::READING_TITLE)))?;
        }
        match shell::carry_out(store, &ui, &screen, effect) {
            Outcome::Continue(message) => notice = message,
            Outcome::Leave => return Ok(()),
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

/// The line of one row.
fn row_line(state: &State, row: &Row) -> Line<'static> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let text = shell::row_text(state, row);
    match text.kind {
        Kind::Parent => Line::from(Span::styled(text.name, dim)),
        Kind::Add => Line::from(vec![Span::raw("+ add "), Span::raw(text.detail)]),
        Kind::Folder => Line::from(vec![Span::raw(text.name), Span::styled("/", dim)]),
        Kind::Shortcut => {
            let mut spans = vec![
                Span::raw(text.name),
                Span::raw("  "),
                Span::styled(text.detail, dim),
            ];
            if !text.alias.is_empty() {
                spans.push(Span::styled(format!("  [{}]", text.alias), dim));
            }
            Line::from(spans)
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
    frame.render_widget(
        Paragraph::new(shell::header(ui, &screen.state))
            .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );
    let line = Line::from(shell::prompt(ui, &screen.state));
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
        Paragraph::new(notice.unwrap_or(shell::HINTS))
            .style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
}
