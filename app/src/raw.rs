//! The Tauri bindings: the window, the tray, the global shortcut, a second start that brings the window forward, and the commands the window calls.

use crate::intent::{Intent, item};
use crate::library::Library;
use crate::themes::ThemeOption;
use crate::view::View;
use crate::watch::Watch;
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindow, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::sync::Notify;
use youarel_core::folder_rules::Seen;
use youarel_core::sync_rules;
use youarel_link::Link;
use youarel_store::Store;

/// The label of the library's one window in `tauri.conf.json`.
const WINDOW: &str = "library";
/// The global shortcut that brings the window forward with the search focused, or hides it.
const SHORTCUT: &str = "Alt+Space";
/// The argument that toggles the window of the running library, for a desktop whose shortcuts it cannot hold.
const TOGGLE: &str = "--toggle";
/// The tray menu's items.
const SHOW: &str = "show";
const QUIT: &str = "quit";
/// The event that tells the page the person summoned the window, so it focuses the search.
const SUMMONED: &str = "summoned";
/// The event that tells the page a sync began or ended, so it draws the view again.
const SYNCED: &str = "synced";
/// How often the window asks whether a sync is due.
const TICK: Duration = Duration::from_secs(5);
/// How long the shared folder rests after a notice before the window asks, the span the core's schedule waits for.
const QUIET: Duration = Duration::from_millis(sync_rules::QUIET);
/// The event that tells the page shortcuts got their icons, so it draws the view again.
const ICONED: &str = "iconed";

type Shared = Mutex<Library>;

fn locked(shared: &Shared) -> MutexGuard<'_, Library> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wakes the ticker once the watch notices what may be another device's bundle.
#[derive(Debug, Default)]
struct Wake(Notify);

/// The watch of the shared folder, held apart from the library so that it drops with no lock of the library held.
#[derive(Debug, Default)]
struct Watching(Mutex<Held>);

/// The watch the window holds, and why its last start failed, which the log names once.
#[derive(Debug, Default)]
struct Held {
    watch: Option<Watch>,
    refused: Option<String>,
}

fn held(watching: &Watching) -> MutexGuard<'_, Held> {
    watching.0.lock().unwrap_or_else(PoisonError::into_inner)
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

/// Brings the window forward and tells the page to focus the search.
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
        AppHandle, BTreeMap, ClipboardExt, Intent, Link, Shared, ThemeOption, View, gather_icons,
        item, locked, reported,
    };
    use crate::library::{Addition, fetch};
    use tauri::State;

    /// Carries out what the window asks for and answers with the view, then asks the sites of the links it opened for their missing favicons.
    #[tauri::command(async)]
    pub(super) fn act(
        app: AppHandle,
        shared: State<'_, Shared>,
        intent: Intent,
    ) -> Result<View, String> {
        let view = locked(&shared)
            .act(intent)
            .map_err(|error| reported(&error));
        gather_icons(&app);
        view
    }

    /// Answers with the view as it stands, keeping its notice, as after a sync began or ended.
    #[tauri::command(async)]
    pub(super) fn view(shared: State<'_, Shared>) -> Result<View, String> {
        locked(&shared).view().map_err(|error| reported(&error))
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
            Link::parse(link.trim()).map_err(|error| format!("cannot add `{link}`: {error}"))?;
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

    /// The title of a link's page, which the add sheet offers before the person types one.
    #[tauri::command(async)]
    pub(super) fn title(link: String) -> Result<Option<String>, String> {
        let link = Link::parse(link.trim()).map_err(|error| error.to_string())?;
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
        match Link::parse(text.trim()) {
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

    /// Every theme the window can wear, for the settings to draw.
    #[tauri::command(async)]
    pub(super) fn themes(shared: State<'_, Shared>) -> Vec<ThemeOption> {
        locked(&shared).themes()
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

/// Runs the sync the schedule says is due, outside the library's lock, and tells the page before and after.
///
/// A publish writes this device's bundle as `youarel sync` does, and a merge only reads the other devices' bundles.
/// The sync then learns the name of this device's own bundle, whose changes the watch passes over from then on.
async fn sync_when_due(app: &AppHandle) {
    let Ok(began) = youarel_cli::clock::now() else {
        // A clock that cannot be read leaves the sync to the next tick.
        return;
    };
    let shared = app.state::<Shared>();
    let (due, store) = {
        let mut library = locked(&shared);
        (library.begin_sync(began), library.store())
    };
    let Some((sync, target)) = due else {
        return;
    };
    drop(app.emit(SYNCED, ()));
    let ran = tauri::async_runtime::spawn_blocking(move || {
        let outcome = match sync {
            sync_rules::Sync::Publish => youarel_cli::sync::sync(&store, began, &target),
            sync_rules::Sync::Merge => youarel_cli::sync::merge(&store, began, &target),
        };
        (outcome, youarel_cli::sync::own_bundle(&store))
    })
    .await;
    let (outcome, own) = ran.unwrap_or_else(|stopped| {
        let reason = format!("the sync stopped: {stopped}");
        (
            Err(anyhow::anyhow!("{reason}")),
            Err(anyhow::anyhow!(reason)),
        )
    });
    {
        let mut library = locked(&shared);
        match own {
            Ok(own) => library.learned_own(own),
            // An account that cannot be opened names no bundle of its own yet, and the sync's outcome says why.
            Err(_unopened) => {}
        }
        library.end_sync(began, outcome);
    }
    drop(app.emit(SYNCED, ()));
}

/// What the watch does with each change it sees: the library notes it, and one that may bring another device's bundle wakes the ticker.
fn tell(app: &AppHandle) -> impl Fn(Seen<'_>) + Send + 'static {
    let handle = app.clone();
    move |seen| {
        // A clock that cannot be read leaves the change to the next scheduled sync.
        let Ok(now) = youarel_cli::clock::now() else {
            return;
        };
        if locked(&handle.state::<Shared>()).noticed(seen, now) {
            handle.state::<Wake>().0.notify_one();
        }
    }
}

/// Keeps the watch on the folder the settings name: starts it, starts it again once it is lost or a sync failed, and stops one no longer wanted.
///
/// The old watch drops with no lock held, since dropping it waits for its thread, which may wait for the library's lock to tell of a change.
fn keep_watch(app: &AppHandle) {
    let shared = app.state::<Shared>();
    let (wanted, again) = {
        let mut library = locked(&shared);
        (library.watch_wanted(), library.rewatch())
    };
    let watching = app.state::<Watching>();
    let stale = {
        let mut held = held(&watching);
        let kept = held.watch.as_ref().is_some_and(|watch| {
            !again && !watch.lost() && Some(watch.folder()) == wanted.as_deref()
        });
        if kept {
            return;
        }
        held.watch.take()
    };
    drop(stale);
    let started = wanted.map(|folder| {
        Watch::start(&folder, tell(app))
            .map_err(|error| format!("cannot watch {}: {error}", folder.display()))
    });
    let watched = matches!(started, Some(Ok(_)));
    {
        let mut held = held(&watching);
        match started {
            Some(Ok(watch)) => {
                eprintln!(
                    "youarel-launcher: watching {} for the other devices' bundles",
                    watch.folder().display()
                );
                held.watch = Some(watch);
                held.refused = None;
            }
            Some(Err(reason)) => {
                if held.refused.as_ref() != Some(&reason) {
                    eprintln!(
                        "youarel-launcher: {reason}; the window checks the folder on its schedule"
                    );
                    held.refused = Some(reason);
                }
            }
            None => held.refused = None,
        }
    }
    locked(&shared).watching(watched);
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
            // A site without a favicon keeps its tile, and the next opening asks it again.
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

/// Keeps the watch in step with the settings and asks whether a sync is due, at every tick and once the folder rests after a notice, for as long as the library runs.
fn ticker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let wake = handle.state::<Wake>();
        loop {
            keep_watch(&handle);
            sync_when_due(&handle).await;
            let nudged = tokio::time::timeout(TICK, wake.0.notified()).await;
            if nudged.is_ok() {
                // A bundle that another device or its cloud drive still writes settles while the folder rests.
                tokio::time::sleep(QUIET).await;
            }
        }
    });
}

/// The event that hands the page a command a person chose from the app's menu, by its identifier.
const COMMAND: &str = "command";
/// The commands of the app's menu that the page carries out, as its keys do.
const COMMANDS: [&str; 7] = [
    "settings",
    "new-link",
    "new-collection",
    "import",
    "cards",
    "list",
    "columns",
];

/// The app's menu on macOS: the app with its settings, the file commands, editing, the layouts, and the window.
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
                &command(app, "settings", "Settings…", Some("CmdOrCtrl+,"))?,
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
        Submenu::with_items(
            app,
            "File",
            true,
            &[
                &command(app, "new-link", "New Link…", Some("CmdOrCtrl+N"))?,
                &command(
                    app,
                    "new-collection",
                    "New Collection…",
                    Some("Shift+CmdOrCtrl+N"),
                )?,
                &Item::separator(app)?,
                &command(app, "import", "Import Bookmark File…", None)?,
                &Item::separator(app)?,
                &Item::close_window(app, None)?,
            ],
        )
    }

    fn edit(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_items(
            app,
            "Edit",
            true,
            &[
                &Item::undo(app, None)?,
                &Item::redo(app, None)?,
                &Item::separator(app)?,
                &Item::cut(app, None)?,
                &Item::copy(app, None)?,
                &Item::paste(app, None)?,
                &Item::select_all(app, None)?,
            ],
        )
    }

    fn view(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
        Submenu::with_items(
            app,
            "View",
            true,
            &[
                &command(app, "cards", "as Cards", Some("CmdOrCtrl+1"))?,
                &command(app, "list", "as List", Some("CmdOrCtrl+2"))?,
                &command(app, "columns", "as Columns", Some("CmdOrCtrl+3"))?,
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

/// Runs the library until the person quits it from the tray or the menu.
#[must_use]
pub fn run() -> ExitCode {
    let store = match youarel_cli::home::data_directory() {
        Ok(directory) => Store::new(directory),
        Err(error) => {
            eprintln!("youarel-launcher: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let shortcuts = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .build();
    let built = with_menu(tauri::Builder::default())
        .on_menu_event(|app, event| menu_command(app, event.id().as_ref()))
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _directory| {
            if arguments.iter().any(|argument| argument == TOGGLE) {
                toggle(app);
            } else {
                show(app);
            }
        }))
        .plugin(shortcuts)
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Mutex::new(Library::new(store)))
        .manage(Wake::default())
        .manage(Watching::default())
        .invoke_handler(tauri::generate_handler![
            commands::act,
            commands::view,
            commands::add,
            commands::title,
            commands::clipboard,
            commands::copy,
            commands::themes,
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
            ticker(app.handle());
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
            app.run(|handle, event| {
                if reopened(&event) {
                    show(handle);
                }
            });
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("youarel-launcher: {error}");
            ExitCode::FAILURE
        }
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
