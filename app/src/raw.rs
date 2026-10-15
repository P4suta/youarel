//! The Tauri bindings: the window, the tray, the global shortcut, a second start that toggles the window, and the commands the window calls.

use crate::launcher::{After, Launcher, Press, View};
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use youarel_store::Store;

/// The label of the launcher's one window in `tauri.conf.json`.
const WINDOW: &str = "launcher";
/// The global shortcut that shows or hides the window.
const SHORTCUT: &str = "Alt+Space";
/// The argument that toggles the window of the running launcher, for a desktop whose shortcuts the launcher cannot hold.
const TOGGLE: &str = "--toggle";
/// The tray menu's items.
const SHOW: &str = "show";
const QUIT: &str = "quit";

type Shared = Mutex<Launcher>;

fn locked(shared: &Shared) -> MutexGuard<'_, Launcher> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Shows the window afresh, in front and with the keyboard.
fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW) else {
        return;
    };
    locked(&app.state::<Shared>()).reset();
    // A window that fails to center, show, or take the focus stays as it was, and the next toggle tries again.
    drop(window.center());
    drop(window.show());
    drop(window.set_focus());
}

fn hide(window: &WebviewWindow) {
    // A window that fails to hide stays, and the person hides it with the next toggle.
    drop(window.hide());
}

/// Shows the window when it is hidden, and hides it otherwise.
fn toggle(app: &AppHandle) {
    let visible = app
        .get_webview_window(WINDOW)
        .is_some_and(|window| window.is_visible().is_ok_and(|shown| shown));
    if visible {
        if let Some(window) = app.get_webview_window(WINDOW) {
            hide(&window);
        }
    } else {
        show(app);
    }
}

/// The view the window draws, or none once it hides.
fn answer(window: &WebviewWindow, after: anyhow::Result<After>) -> Result<Option<View>, String> {
    match after {
        Ok(After::Draw(view)) => Ok(Some(view)),
        Ok(After::Hide) => {
            hide(window);
            Ok(None)
        }
        Err(error) => Err(format!("{error:#}")),
    }
}

mod commands {
    #![expect(
        clippy::needless_pass_by_value,
        reason = "Tauri hands each command its arguments and its state by value"
    )]

    use super::{Press, Shared, State, View, WebviewWindow, answer, locked};

    /// The view of the interface as the store holds it now.
    #[tauri::command(async)]
    pub(super) fn view(shared: State<'_, Shared>) -> Result<View, String> {
        locked(&shared).view().map_err(|error| format!("{error:#}"))
    }

    /// A key press that the window does not leave to its input field.
    #[tauri::command(async)]
    pub(super) fn press(
        window: WebviewWindow,
        shared: State<'_, Shared>,
        press: Press,
    ) -> Result<Option<View>, String> {
        let after = locked(&shared).press(&press);
        answer(&window, after)
    }

    /// Text the person typed or an input method composed.
    #[tauri::command(async)]
    pub(super) fn typed(
        window: WebviewWindow,
        shared: State<'_, Shared>,
        text: String,
    ) -> Result<Option<View>, String> {
        let after = locked(&shared).typed(&text);
        answer(&window, after)
    }

    /// Text the person pasted.
    #[tauri::command(async)]
    pub(super) fn paste(
        window: WebviewWindow,
        shared: State<'_, Shared>,
        text: String,
    ) -> Result<Option<View>, String> {
        let after = locked(&shared).paste(&text);
        answer(&window, after)
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
    reason = "the context Tauri generates ends the process itself when an embedded asset cannot be read, which the launcher cannot reach"
)]
fn context() -> tauri::Context {
    tauri::generate_context!()
}

/// Runs the launcher until the person quits it from the tray.
#[must_use]
pub fn run() -> ExitCode {
    let store = match youarel_cli::home::data_directory() {
        Ok(directory) => Store::new(directory),
        Err(error) => {
            eprintln!("youarel-launcher: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let toggled = std::env::args().any(|argument| argument == TOGGLE);
    let shortcuts = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .build();
    let ran = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _directory| {
            if arguments.iter().any(|argument| argument == TOGGLE) {
                toggle(app);
            } else {
                show(app);
            }
        }))
        .plugin(shortcuts)
        .manage(Mutex::new(Launcher::new(store)))
        .invoke_handler(tauri::generate_handler![
            commands::view,
            commands::press,
            commands::typed,
            commands::paste
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            tray(app)?;
            // A desktop that keeps global shortcuts to itself, such as Wayland's, still toggles the window through `--toggle`.
            if let Err(error) = app.global_shortcut().register(SHORTCUT) {
                eprintln!("youarel-launcher: {SHORTCUT} stays unbound ({error}); bind `youarel-launcher {TOGGLE}` to a shortcut of the desktop instead");
            }
            if toggled {
                show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // The window hides rather than closes, and hides when the person turns to another, as a launcher does.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                drop(window.hide());
            } else if matches!(event, WindowEvent::Focused(false)) {
                drop(window.hide());
            }
        })
        .run(context());
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("youarel-launcher: {error}");
            ExitCode::FAILURE
        }
    }
}
