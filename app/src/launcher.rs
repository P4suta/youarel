//! The window's side of the launcher's shell: the window reports keys and text, the machine decides, and the window draws the view it gets back.
//!
//! Nothing here touches the webview, so the tests drive it as the window would.

use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use youarel_cli::shell::{self, Kind, Outcome};
use youarel_core::state::IconId;
use youarel_core::ui_rules::{Effect, Event, Key, Ui, step};
use youarel_store::Store;

/// A key press as the window reports it: the key's name as the DOM's `KeyboardEvent.key` spells it, and the modifiers held.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Press {
    pub key: String,
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub shift: bool,
}

/// The machine's key for a press, or `None` for one the window leaves to its input field.
///
/// A character without the control key arrives as typed text instead, so an input method can compose it first.
/// The control key commands as it does in the terminal launcher, and the command key of macOS keeps its text editing.
#[must_use]
pub fn key(press: &Press) -> Option<Key> {
    let mut characters = press.key.chars();
    let single = characters.next().filter(|_| characters.next().is_none());
    let commanding = press.control;
    Some(match (press.key.as_str(), single) {
        ("Enter", _) => Key::Enter,
        ("Escape", _) => Key::Escape,
        ("Backspace", _) => Key::Backspace,
        ("Tab", _) => Key::Tab,
        ("ArrowUp", _) if press.shift => Key::ShiftUp,
        ("ArrowUp", _) => Key::Up,
        ("ArrowDown", _) if press.shift => Key::ShiftDown,
        ("ArrowDown", _) => Key::Down,
        ("ArrowLeft", _) => Key::Left,
        ("ArrowRight", _) => Key::Right,
        (_, Some(letter)) if commanding && letter.is_ascii_alphabetic() => {
            Key::Control(letter.to_ascii_lowercase())
        }
        _ => return None,
    })
}

/// One row as the window draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RowView {
    pub kind: &'static str,
    pub name: String,
    pub detail: String,
    pub alias: String,
    /// The shortcut's icon as a `data:` address of its PNG, or none.
    pub icon: Option<String>,
}

/// The `data:` address of an icon's PNG, which the window's policy admits as an image and nothing else.
fn data_address(image: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64.encode(image))
}

/// What the window draws.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct View {
    pub header: String,
    pub prompt: String,
    pub rows: Vec<RowView>,
    pub selected: usize,
    /// The notice of the last effect, or the key hints.
    pub footer: String,
    /// Whether the person carries an entry, which control and V then moves rather than pastes text.
    pub carrying: bool,
}

/// What the window does after an input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum After {
    /// It draws the view.
    Draw(View),
    /// It hides, since the person opened a shortcut or left.
    Hide,
}

const fn kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Parent => "parent",
        Kind::Add => "add",
        Kind::Folder => "folder",
        Kind::Shortcut => "shortcut",
    }
}

/// The launcher behind one window: the store, the interface, and the last notice.
#[derive(Debug)]
pub struct Launcher {
    store: Store,
    ui: Ui,
    notice: Option<String>,
    /// Each icon the window has shown, drawn again by youarel's own decoder, or `None` for one that does not draw.
    drawn: BTreeMap<IconId, Option<String>>,
}

impl Launcher {
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store,
            ui: Ui::default(),
            notice: None,
            drawn: BTreeMap::new(),
        }
    }

    /// The `data:` address of an icon, drawn again first, so the webview's decoders only ever read an image youarel encoded, whichever member sent it.
    fn picture(&mut self, icon: IconId, image: &[u8]) -> Option<String> {
        self.drawn
            .entry(icon)
            .or_insert_with(|| {
                // An image that does not draw shows as no icon, as a shortcut without one does.
                youarel_icon::normalize(image)
                    .map_or_else(|_undrawable| None, |redrawn| Some(data_address(&redrawn)))
            })
            .clone()
    }

    /// Starts over, as the window does each time it appears.
    pub fn reset(&mut self) {
        self.ui = Ui::default();
        self.notice = None;
    }

    /// The view of the interface over the store as it is now.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn view(&mut self) -> Result<View> {
        let screen = shell::load(&self.store, &self.ui)?;
        self.ui = step(&self.ui, &screen.rows, &Event::Refresh).0;
        let mut rows = Vec::with_capacity(screen.rows.len());
        for row in &screen.rows {
            let text = shell::row_text(&screen.state, row);
            let icon = shell::row_icon(&screen.state, row)
                .and_then(|(icon, image)| self.picture(icon, image));
            rows.push(RowView {
                kind: kind(text.kind),
                name: text.name,
                detail: text.detail,
                alias: text.alias,
                icon,
            });
        }
        Ok(View {
            header: shell::header(&self.ui, &screen.state),
            prompt: shell::prompt(&self.ui, &screen.state),
            rows,
            selected: self.ui.selected,
            footer: self
                .notice
                .clone()
                .unwrap_or_else(|| shell::HINTS.to_owned()),
            carrying: self.ui.carried.is_some(),
        })
    }

    /// Feeds one event to the machine, carries out its effect, and says what the window does next.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn feed(&mut self, event: &Event) -> Result<After> {
        let screen = shell::load(&self.store, &self.ui)?;
        let refreshed = step(&self.ui, &screen.rows, &Event::Refresh).0;
        let (next, effect) = step(&refreshed, &screen.rows, event);
        self.ui = next;
        if effect == Effect::None {
            self.notice = None;
            return self.view().map(After::Draw);
        }
        match shell::carry_out(&self.store, &self.ui, &screen, effect) {
            Outcome::Continue(notice) => {
                self.notice = notice;
                self.view().map(After::Draw)
            }
            Outcome::Leave => {
                self.reset();
                Ok(After::Hide)
            }
        }
    }

    /// Feeds a key press, or draws the view again for a press the machine does not read.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn press(&mut self, press: &Press) -> Result<After> {
        match key(press) {
            Some(read) => self.feed(&Event::Key(read)),
            None => self.view().map(After::Draw),
        }
    }

    /// Feeds text that the person typed or an input method composed, one character at a time.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn typed(&mut self, text: &str) -> Result<After> {
        let mut after = After::Hide;
        for character in text.chars().filter(|character| !character.is_control()) {
            after = self.feed(&Event::Key(Key::Char(character)))?;
            if after == After::Hide {
                return Ok(after);
            }
        }
        match after {
            After::Hide => self.view().map(After::Draw),
            After::Draw(_) => Ok(after),
        }
    }

    /// Feeds pasted text at once, which offers to add a link.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn paste(&mut self, text: &str) -> Result<After> {
        self.feed(&Event::Paste(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{After, Launcher, Press, View, key};
    use youarel_core::state::{Edit, IconId};
    use youarel_core::ui_rules::Key;
    use youarel_store::Store;

    fn press(name: &str) -> Press {
        Press {
            key: name.to_owned(),
            ..Press::default()
        }
    }

    #[test]
    fn a_press_maps_to_the_key_the_machine_reads() {
        assert_eq!(key(&press("Enter")), Some(Key::Enter));
        assert_eq!(key(&press("Escape")), Some(Key::Escape));
        assert_eq!(key(&press("Backspace")), Some(Key::Backspace));
        assert_eq!(key(&press("Tab")), Some(Key::Tab));
        assert_eq!(key(&press("ArrowLeft")), Some(Key::Left));
        assert_eq!(key(&press("ArrowRight")), Some(Key::Right));
        assert_eq!(key(&press("ArrowUp")), Some(Key::Up));
        assert_eq!(key(&press("ArrowDown")), Some(Key::Down));
        let shifted = |name: &str| Press {
            shift: true,
            ..press(name)
        };
        assert_eq!(key(&shifted("ArrowUp")), Some(Key::ShiftUp));
        assert_eq!(key(&shifted("ArrowDown")), Some(Key::ShiftDown));
        let control = Press {
            control: true,
            ..press("E")
        };
        assert_eq!(key(&control), Some(Key::Control('e')));
        assert_eq!(key(&press("a")), None, "typed text arrives as text");
        assert_eq!(
            key(&Press {
                control: true,
                ..press("1")
            }),
            None
        );
        assert_eq!(
            key(&Press {
                control: true,
                ..press("ab")
            }),
            None
        );
        assert_eq!(key(&press("F1")), None);
    }

    /// The view the window draws after an input that must keep it.
    fn drawn(after: After) -> View {
        match after {
            After::Draw(view) => view,
            After::Hide => panic!("the window hid"),
        }
    }

    #[test]
    fn the_window_types_a_folder_s_name_and_leaves_on_escape() {
        let home = tempfile::tempdir().unwrap();
        let mut launcher = Launcher::new(Store::new(home.path().to_path_buf()));
        let empty = launcher.view().unwrap();
        assert_eq!(empty.rows, []);
        assert_eq!(empty.prompt, "> ");
        let naming = Press {
            control: true,
            ..press("o")
        };
        drawn(launcher.press(&naming).unwrap());
        let typed = drawn(launcher.typed("仕事\u{7}").unwrap());
        assert_eq!(typed.prompt, "new folder: 仕事");
        let created = drawn(launcher.press(&press("Enter")).unwrap());
        assert_eq!(created.footer, "folder created");
        assert_eq!(created.rows.len(), 1);
        assert_eq!(created.rows.first().map(|row| row.kind), Some("folder"));
        let ignored = drawn(launcher.press(&press("F1")).unwrap());
        assert_eq!(
            ignored.footer, "folder created",
            "an ignored key keeps the notice"
        );
        let offered = drawn(launcher.paste("https://example.com/").unwrap());
        assert_eq!(offered.rows.first().map(|row| row.kind), Some("add"));
        assert_eq!(drawn(launcher.typed("").unwrap()), launcher.view().unwrap());
        let cleared = drawn(launcher.press(&press("Escape")).unwrap());
        assert_eq!(cleared.prompt, "> ", "the first escape clears the query");
        assert_eq!(launcher.press(&press("Escape")).unwrap(), After::Hide);
        assert_eq!(launcher.view().unwrap().prompt, "> ");
    }

    #[test]
    fn the_window_shows_each_icon_drawn_again_and_none_for_an_image_that_does_not_draw() {
        const PNG: &[u8] = include_bytes!("../icons/32x32.png");
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        store
            .session(1, |session| {
                for (link, image) in [
                    ("https://drawn.example/", PNG),
                    ("https://broken.example/", &b"not an image"[..]),
                ] {
                    let id = session.fresh_id()?;
                    session.edit(Edit::AddShortcut {
                        id,
                        link: link.to_owned(),
                        title: link.to_owned(),
                        folder: None,
                    })?;
                    session.edit(Edit::SetIcon {
                        id,
                        icon: Some((IconId([image.len().to_le_bytes()[0]; 32]), image.to_vec())),
                    })?;
                }
                Ok(())
            })
            .unwrap();
        let mut launcher = Launcher::new(store);
        let view = launcher.view().unwrap();
        let icon = |name: &str| {
            view.rows
                .iter()
                .find(|row| row.name == name)
                .map(|row| row.icon.clone())
        };
        let redrawn = youarel_icon::normalize(PNG).unwrap();
        assert_eq!(
            icon("https://drawn.example/"),
            Some(Some(super::data_address(&redrawn)))
        );
        assert_eq!(icon("https://broken.example/"), Some(None));
        assert_eq!(launcher.view().unwrap(), view, "a drawn icon stays drawn");
    }
}
