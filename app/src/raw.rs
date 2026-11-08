//! The Tauri bindings: the window, the tray, the global shortcut, a second start that brings the window forward, and the commands the window calls.

use crate::intent::{Intent, item};
use crate::library::{Job, Library};
use crate::view::{Progress, View};
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindow, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use youarel_link::Link;
use youarel_store::Store;

/// The label of the library's one window in `tauri.conf.json`.
const WINDOW: &str = "library";
/// The global shortcut that brings the window forward with the folder focused, or hides it.
const SHORTCUT: &str = "Alt+Space";
/// The argument that toggles the window of the running library, for a desktop whose shortcuts it cannot hold.
const TOGGLE: &str = "--toggle";
const VERSION: &str = "--version";
/// The tray menu's items.
const SHOW: &str = "show";
const QUIT: &str = "quit";
/// The event that tells the page to focus the current folder after a summon.
const SUMMONED: &str = "summoned";
/// The event that tells the page shortcuts got their icons, so it draws the view again.
const ICONED: &str = "iconed";
/// The event that tells the page how far a move or a removal came, which it shows while the job works.
const PROGRESS: &str = "progress";

type Shared = Mutex<Library>;

fn locked(shared: &Shared) -> MutexGuard<'_, Library> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Carries out the next batch of a job under the library's lock, which it lets go before the next batch.
fn advance(shared: &Shared, job: &mut Job) -> Option<Progress> {
    locked(shared).advance(job)
}

/// Brings the window forward with the keyboard.
fn bring_forward(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW) else {
        return;
    };
    // A window that fails to show or take the focus stays as it was, and the next summons try again.
    drop(window.unminimize());
    drop(window.show());
    drop(window.set_focus());
}

/// Brings the window forward and tells the page to focus the folder.
fn show(app: &AppHandle) {
    bring_forward(app);
    drop(app.emit(SUMMONED, ()));
}

fn hide(window: &WebviewWindow) {
    // A window that fails to hide stays, and the person hides it with the next toggle.
    drop(window.hide());
}

/// Hides the window when it is in front, and brings it forward otherwise.
fn toggle(app: &AppHandle) {
    let front = app.get_webview_window(WINDOW).filter(|window| {
        window.is_visible().is_ok_and(|shown| shown)
            && window.is_focused().is_ok_and(|focused| focused)
    });
    match front {
        Some(window) => hide(&window),
        None => show(app),
    }
}

/// A failure as the page reads it.
fn reported(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

mod commands {
    #![expect(
        clippy::needless_pass_by_value,
        reason = "Tauri hands each command its arguments and its state by value"
    )]

    use super::{
        AppHandle, BTreeMap, ClipboardExt, Emitter as _, FILE_COMMANDS, Intent, Link, PROGRESS,
        Shared, View, advance, gather_icons, item, locked, reported,
    };
    use crate::library::{Addition, Begun, fetch};
    use crate::view::{Choice, RowsView};
    use tauri::{Manager as _, State};

    /// Carries out what the window asks for and answers with the view, then asks the sites of the links it opened for their missing favicons.
    ///
    /// A move or a removal runs in batches and releases the library's lock between them, so the window pages and reports progress meanwhile.
    #[tauri::command(async)]
    pub(super) fn act(
        app: AppHandle,
        shared: State<'_, Shared>,
        intent: Intent,
    ) -> Result<View, String> {
        let begun = locked(&shared).begin(intent);
        let view = match begun {
            Ok(Begun::Answered(view)) => Ok(*view),
            Ok(Begun::Working(mut job)) => {
                while let Some(progress) = advance(&shared, &mut job) {
                    // A page that misses a step of the progress hears the next one, and the view after the last.
                    drop(app.emit(PROGRESS, progress));
                }
                locked(&shared).finish(&mut job)
            }
            Err(error) => Err(error),
        };
        gather_icons(&app);
        view.map_err(|error| reported(&error))
    }

    /// Rows of the bookmark list, from a place on as the page scrolls.
    #[tauri::command(async)]
    pub(super) fn page(
        shared: State<'_, Shared>,
        first: usize,
        limit: usize,
    ) -> Result<RowsView, String> {
        locked(&shared)
            .page(first, limit)
            .map_err(|error| reported(&error))
    }

    /// The collections a picker offers for what a person typed, from a place on.
    #[tauri::command(async)]
    pub(super) fn choices(
        shared: State<'_, Shared>,
        query: String,
        first: usize,
        limit: usize,
    ) -> Result<Vec<Choice>, String> {
        locked(&shared)
            .choices(&query, first, limit)
            .map_err(|error| reported(&error))
    }

    /// Answers with the view as it stands, keeping its notice, after background icon collection.
    #[tauri::command(async)]
    pub(super) fn view(shared: State<'_, Shared>) -> Result<View, String> {
        let view = locked(&shared).view().map_err(|error| reported(&error))?;
        Ok(view)
    }

    /// Refreshes around the entries the page already shows, without moving its selection.
    #[tauri::command(async)]
    pub(super) fn view_anchored(
        shared: State<'_, Shared>,
        anchors: Vec<crate::view::ViewportAnchor>,
    ) -> Result<View, String> {
        let view = locked(&shared)
            .view_anchored(&anchors)
            .map_err(|error| reported(&error))?;
        Ok(view)
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct MenuState {
        undo: Option<String>,
        redo: Option<String>,
        field: bool,
        enabled: Vec<String>,
        in_trash: bool,
    }

    /// Updates command availability and names the library's undo and redo steps.
    #[tauri::command(async)]
    pub(super) fn history_menu(app: AppHandle, state: MenuState) -> Result<(), String> {
        let MenuState {
            undo,
            redo,
            field,
            enabled,
            in_trash,
        } = state;
        let Some(menu) = app.menu() else {
            return Ok(());
        };
        let editing = menu
            .get("editing")
            .and_then(|item| item.as_submenu().cloned())
            .ok_or("the Edit menu is missing")?;
        for (id, verb, step) in [("undo", "Undo", undo), ("redo", "Redo", redo)] {
            let item = editing
                .get(id)
                .and_then(|item| item.as_menuitem().cloned())
                .ok_or_else(|| format!("the {verb} menu item is missing"))?;
            let label = step
                .as_ref()
                .filter(|_| !field)
                .map_or_else(|| verb.to_owned(), |step| format!("{verb} {step}"));
            item.set_text(label).map_err(|error| error.to_string())?;
            item.set_enabled(field || step.is_some())
                .map_err(|error| error.to_string())?;
        }
        let file = menu
            .get("file")
            .and_then(|item| item.as_submenu().cloned())
            .ok_or("the File menu is missing")?;
        for id in FILE_COMMANDS {
            let item = file
                .get(id)
                .and_then(|item| item.as_menuitem().cloned())
                .ok_or_else(|| format!("the {id} menu item is missing"))?;
            item.set_enabled(enabled.iter().any(|command| command == id))
                .map_err(|error| error.to_string())?;
            if id == "trash-selection" {
                item.set_text(if in_trash {
                    "Delete Immediately…"
                } else {
                    "Move to Trash"
                })
                .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// Adds a link to a collection, reading its page for a missing title and for its icon before the store is locked.
    ///
    /// A link the collection holds already reads no page, and its shortcut is selected instead.
    #[tauri::command(async)]
    pub(super) fn add(
        shared: State<'_, Shared>,
        link: String,
        title: String,
        collection: Option<String>,
    ) -> Result<View, String> {
        let link =
            Link::from_input(&link).map_err(|error| format!("cannot add `{link}`: {error}"))?;
        let collection = collection
            .as_deref()
            .map(item)
            .transpose()
            .map_err(|error| reported(&error))?;
        let held = locked(&shared)
            .holder(&link, collection)
            .map_err(|error| reported(&error))?;
        if let Some(holder) = held {
            return locked(&shared)
                .reveal(holder)
                .map_err(|error| reported(&error));
        }
        let fetched = fetch(&link);
        let title = Some(title.trim().to_owned())
            .filter(|title| !title.is_empty())
            .or(fetched.title)
            .unwrap_or_else(|| link.as_str().to_owned());
        let addition = Addition {
            link,
            title,
            collection,
            icon: fetched.icon,
            missing: fetched.missing,
        };
        locked(&shared)
            .add(addition)
            .map_err(|error| reported(&error))
    }

    /// Completes a typed web address without fetching its page.
    #[tauri::command]
    pub(super) fn normalize_link(link: String) -> Option<String> {
        match Link::from_input(&link) {
            Ok(link) => Some(link.as_str().to_owned()),
            Err(_not_a_web_address) => None,
        }
    }

    /// The title of a link's page, which the add sheet offers before the person types one.
    #[tauri::command(async)]
    pub(super) fn title(link: String) -> Result<Option<String>, String> {
        let link = Link::from_input(&link).map_err(|error| error.to_string())?;
        youarel_cli::web::fetch_page(&link)
            .map(|page| page.title)
            .map_err(|error| reported(&error))
    }

    /// The link on the clipboard, normalized, or none when the clipboard holds no link.
    #[tauri::command(async)]
    pub(super) fn clipboard(app: AppHandle) -> Option<String> {
        // A clipboard that holds no text, or text that is no link, offers nothing to add.
        let text = match app.clipboard().read_text() {
            Ok(text) => text,
            Err(_no_text) => return None,
        };
        match Link::from_input(&text) {
            Ok(link) => Some(link.as_str().to_owned()),
            Err(_no_link) => None,
        }
    }

    /// Puts text on the clipboard.
    #[tauri::command(async)]
    pub(super) fn copy(app: AppHandle, text: String) -> Result<(), String> {
        app.clipboard()
            .write_text(text)
            .map_err(|error| error.to_string())
    }

    /// Copies the selected shortcuts as links and cancels a pending cut after the clipboard accepts them.
    #[tauri::command(async)]
    pub(super) fn copy_selection(
        app: AppHandle,
        shared: State<'_, Shared>,
    ) -> Result<View, String> {
        let text = locked(&shared)
            .selected_links()
            .map_err(|error| reported(&error))?;
        if text.is_empty() {
            return Err("the selection holds no shortcuts to copy".to_owned());
        }
        app.clipboard()
            .write_text(text)
            .map_err(|error| error.to_string())?;
        locked(&shared)
            .act(Intent::CancelCut)
            .map_err(|error| reported(&error))
    }

    /// Moves the cut entries or pastes the clipboard's web links.
    #[tauri::command(async)]
    pub(super) fn paste(app: AppHandle, shared: State<'_, Shared>) -> Result<View, String> {
        let text = if locked(&shared).cutting() {
            String::new()
        } else {
            app.clipboard()
                .read_text()
                .map_err(|error| error.to_string())?
        };
        act(app, shared, Intent::Paste { text })
    }

    /// Hides the window, as its close button does.
    #[tauri::command(async)]
    pub(super) fn close(app: AppHandle) -> Result<(), String> {
        app.get_webview_window(super::WINDOW)
            .ok_or("the library window is missing")?
            .hide()
            .map_err(|error| error.to_string())
    }

    /// Freezes the exact entries and links the drag's native payload names.
    #[tauri::command(async)]
    pub(super) fn prepare_drag(
        shared: State<'_, Shared>,
        target: crate::intent::TargetName,
    ) -> Result<crate::view::DragMaterial, String> {
        locked(&shared)
            .prepare_drag(&target)
            .map_err(|error| reported(&error))
    }

    /// Starts or cancels a hover, independently of a view redraw.
    #[tauri::command(async)]
    pub(super) fn hover(
        shared: State<'_, Shared>,
        token: String,
        folder: Option<String>,
    ) -> Result<Option<u64>, String> {
        let token = item(&token).map_err(|error| reported(&error))?;
        let folder =
            crate::intent::optional(folder.as_deref()).map_err(|error| reported(&error))?;
        locked(&shared)
            .hover(token, folder)
            .map_err(|error| reported(&error))
    }

    /// What the mark picker shows for a search: every category for an empty one, or the emoji whose names hold it.
    #[tauri::command(async)]
    pub(super) fn emoji(query: String) -> Vec<crate::emoji::EmojiGroup> {
        crate::emoji::pick(&query)
    }

    /// The `data:` addresses of the icons the page lacks.
    #[tauri::command(async)]
    pub(super) fn icons(
        shared: State<'_, Shared>,
        names: Vec<String>,
    ) -> Result<BTreeMap<String, String>, String> {
        locked(&shared)
            .icons(&names)
            .map_err(|error| reported(&error))
    }
}

/// Fetches the favicons of the sites whose links a person just opened and whose hosts show no icon, outside the library's lock, and tells the page once one arrives.
///
/// The browser reaches each site at that moment anyway, and the window reaches no other site on its own.
fn gather_icons(app: &AppHandle) {
    let wanted = locked(&app.state::<Shared>()).icons_wanted();
    if wanted.is_empty() {
        return;
    }
    let handle = app.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let mut kept = false;
        for link in wanted {
            // A site without a favicon keeps its link icon, and the next opening asks it again.
            let Ok(icon) = youarel_cli::icons::favicon(&link) else {
                continue;
            };
            // A store that cannot keep the icon now leaves it to the next opening too.
            kept |= locked(&handle.state::<Shared>())
                .keep_icon(&link, icon)
                .is_ok();
        }
        if kept {
            drop(handle.emit(ICONED, ()));
        }
    }));
}

/// The event that hands the page a command a person chose from the app's menu, by its identifier.
const COMMAND: &str = "command";
/// The commands of the app's menu that the page carries out, as its keys do.
const COMMANDS: [&str; 23] = [
    "new-link",
    "new-collection",
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "select-all",
    "open",
    "back",
    "forward",
    "show-in-folder",
    "zoom-in",
    "zoom-out",
    "zoom-reset",
    "edit-bookmark",
    "copy-url",
    "rename-folder",
    "move-folder",
    "mark-folder",
    "trash-selection",
    "put-back",
    "empty-trash",
];

const FILE_COMMANDS: [&str; 11] = [
    "new-link",
    "new-collection",
    "open",
    "edit-bookmark",
    "copy-url",
    "rename-folder",
    "move-folder",
    "mark-folder",
    "trash-selection",
    "put-back",
    "empty-trash",
];

/// The app's menu on macOS: bookmark commands, editing, navigation, and the window.
#[cfg(target_os = "macos")]
mod app_menu {
    use super::{AppHandle, Menu, MenuItem};
    use tauri::Wry;
    use tauri::menu::{PredefinedMenuItem as Item, Submenu};

    fn command(
        app: &AppHandle,
        id: &str,
        label: &str,
        keys: Option<&str>,
    ) -> tauri::Result<MenuItem<Wry>> {
        MenuItem::with_id(app, id, label, true, keys)
    }

    fn application(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_items(
            app,
            app.package_info().name.clone(),
            true,
            &[
                &Item::about(app, None, None)?,
                &Item::separator(app)?,
                &Item::services(app, None)?,
                &Item::separator(app)?,
                &Item::hide(app, None)?,
                &Item::hide_others(app, None)?,
                &Item::show_all(app, None)?,
                &Item::separator(app)?,
                &Item::quit(app, None)?,
            ],
        )
    }

    fn file(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_id_and_items(
            app,
            "file",
            "File",
            true,
            &[
                &command(app, "new-link", "New Bookmark…", Some("CmdOrCtrl+N"))?,
                &command(
                    app,
                    "new-collection",
                    "New Folder…",
                    Some("Shift+CmdOrCtrl+N"),
                )?,
                &Item::separator(app)?,
                &command(app, "open", "Open", Some("CmdOrCtrl+O"))?,
                &command(app, "edit-bookmark", "Edit Bookmark…", Some("CmdOrCtrl+E"))?,
                &command(app, "copy-url", "Copy URL", None)?,
                &command(app, "rename-folder", "Rename Folder…", Some("F2"))?,
                &command(app, "move-folder", "Move to Folder…", None)?,
                &command(app, "mark-folder", "Choose Emoji…", None)?,
                &Item::separator(app)?,
                &command(
                    app,
                    "trash-selection",
                    "Move to Trash",
                    Some("CmdOrCtrl+Backspace"),
                )?,
                &command(app, "put-back", "Put Back", None)?,
                &command(app, "empty-trash", "Empty Trash…", None)?,
                &Item::separator(app)?,
                &Item::close_window(app, None)?,
            ],
        )
    }

    fn edit(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_id_and_items(
            app,
            "editing",
            "Edit",
            true,
            &[
                &command(app, "undo", "Undo", Some("CmdOrCtrl+Z"))?,
                &command(app, "redo", "Redo", Some("Shift+CmdOrCtrl+Z"))?,
                &Item::separator(app)?,
                &command(app, "cut", "Cut", Some("CmdOrCtrl+X"))?,
                &command(app, "copy", "Copy", Some("CmdOrCtrl+C"))?,
                &command(app, "paste", "Paste", Some("CmdOrCtrl+V"))?,
                &command(app, "select-all", "Select All", Some("CmdOrCtrl+A"))?,
            ],
        )
    }

    fn view(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_items(
            app,
            "View",
            true,
            &[
                &command(app, "back", "Back", Some("CmdOrCtrl+["))?,
                &command(app, "forward", "Forward", Some("CmdOrCtrl+]"))?,
                &command(app, "show-in-folder", "Show in Folder", Some("CmdOrCtrl+R"))?,
                &Item::separator(app)?,
                &command(app, "zoom-in", "Zoom In", Some("CmdOrCtrl+Plus"))?,
                &command(app, "zoom-out", "Zoom Out", Some("CmdOrCtrl+-"))?,
                &command(app, "zoom-reset", "Actual Size", Some("CmdOrCtrl+0"))?,
                &Item::separator(app)?,
                &Item::fullscreen(app, None)?,
            ],
        )
    }

    fn window(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_items(
            app,
            "Window",
            true,
            &[
                &Item::minimize(app, None)?,
                &Item::maximize(app, None)?,
                &Item::separator(app)?,
                &Item::close_window(app, None)?,
            ],
        )
    }

    /// The whole menu bar, in the order a Mac app keeps it.
    pub(super) fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
        Menu::with_items(
            app,
            &[
                &application(app)?,
                &file(app)?,
                &edit(app)?,
                &view(app)?,
                &window(app)?,
            ],
        )
    }
}

/// The builder with the app's menu, which only macOS shows in a bar of its own.
#[cfg(target_os = "macos")]
fn with_menu(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.menu(app_menu::menu)
}

/// The builder as it is, since a menu bar would sit inside the window off macOS.
#[cfg(not(target_os = "macos"))]
const fn with_menu(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}

/// Hands the page a command of the app's menu; the tray's items carry their own handler.
fn menu_command(app: &AppHandle, id: &str) {
    if COMMANDS.contains(&id) {
        bring_forward(app);
        drop(app.emit(COMMAND, id));
    }
}

/// The tray's icon and menu, which shows the window or quits.
fn tray(app: &tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, SHOW, "Show youarel", true, Some(SHORTCUT))?;
    let quit_item = MenuItem::with_id(app, QUIT, "Quit youarel", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("youarel")
        .menu(&menu)
        .on_menu_event(|handle, event| match event.id().as_ref() {
            SHOW => show(handle),
            QUIT => handle.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// The configuration, the interface, and the icons that `tauri.conf.json` names, embedded at build time.
#[expect(
    clippy::disallowed_methods,
    reason = "the context Tauri generates ends the process itself when an embedded asset cannot be read, which the library cannot reach"
)]
fn context() -> tauri::Context {
    tauri::generate_context!()
}

fn shortcuts() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .build()
}

/// Runs the library until the person quits it from the tray or the menu.
#[must_use]
pub fn run() -> ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == VERSION)
    {
        println!("youarel-launcher {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let store = match youarel_cli::home::data_directory() {
        Ok(directory) => Store::new(directory),
        Err(error) => {
            eprintln!("youarel-launcher: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let built = with_menu(tauri::Builder::default())
        .on_menu_event(|app, event| menu_command(app, event.id().as_ref()))
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _directory| {
            if arguments.iter().any(|argument| argument == TOGGLE) {
                toggle(app);
            } else {
                show(app);
            }
        }))
        .plugin(shortcuts())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Mutex::new(Library::new(store)))
        .invoke_handler(tauri::generate_handler![
            commands::act,
            commands::page,
            commands::choices,
            commands::view,
            commands::view_anchored,
            commands::history_menu,
            commands::add,
            commands::normalize_link,
            commands::title,
            commands::clipboard,
            commands::copy,
            commands::copy_selection,
            commands::paste,
            commands::close,
            commands::prepare_drag,
            commands::hover,
            commands::emoji,
            commands::icons
        ])
        .setup(|app| {
            tray(app)?;
            // A desktop that keeps global shortcuts to itself, such as Wayland's, still toggles the window through `--toggle`.
            if let Err(error) = app.global_shortcut().register(SHORTCUT) {
                eprintln!("youarel-launcher: {SHORTCUT} stays unbound ({error}); bind `youarel-launcher {TOGGLE}` to a shortcut of the desktop instead");
            }
            show(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it, and the tray, the shortcut, or the Dock brings it back.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                drop(window.hide());
            }
        })
        .build(context());
    match built {
        Ok(app) => {
            app.run(|handle, event| handle_event(handle, &event));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("youarel-launcher: {error}");
            ExitCode::FAILURE
        }
    }
}

fn handle_event(handle: &AppHandle, event: &RunEvent) {
    if reopened(event) {
        show(handle);
    }
}

/// Whether the person asked for the window again from the Dock, which only macOS reports.
#[cfg(target_os = "macos")]
const fn reopened(event: &RunEvent) -> bool {
    matches!(event, RunEvent::Reopen { .. })
}

#[cfg(not(target_os = "macos"))]
const fn reopened(_event: &RunEvent) -> bool {
    false
}
