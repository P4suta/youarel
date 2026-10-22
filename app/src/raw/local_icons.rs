use super::{AppHandle, Emitter as _, ICONED, Manager as _};
use crate::local_icons::LocalIcons;

pub(super) fn tick(app: &AppHandle) {
    let now = match youarel_cli::clock::now() {
        Ok(now) => now,
        Err(error) => {
            eprintln!("youarel-launcher: local icons: {error:#}");
            return;
        }
    };
    if !app.state::<LocalIcons>().reserve() {
        return;
    }
    let handle = app.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let result = handle.state::<LocalIcons>().tick(now);
        match result {
            Ok(report) => {
                for problem in report.problems {
                    eprintln!("youarel-launcher: local icons: {problem}");
                }
                if report.kept > 0 {
                    drop(handle.emit(ICONED, ()));
                }
            }
            Err(error) => eprintln!("youarel-launcher: local icons: {error:#}"),
        }
    }));
}
