#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands commands their application handle by value"
)]

use super::{
    AppHandle, Emitter as _, Manager as _, Mutex, MutexGuard, PoisonError, Shared, locked,
};
use crate::devices::{self, Devices, Job, Progress, Request, Snapshot};
use serde::Serialize;

pub(super) type SharedDevices = Mutex<Devices>;
const CHANGED: &str = "devices-changed";

fn device_lock(shared: &SharedDevices) -> MutexGuard<'_, Devices> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Display {
    snapshot: Snapshot,
    progress: Progress,
}

#[tauri::command]
pub(super) async fn devices(app: AppHandle) -> Result<Display, String> {
    let store = locked(&app.state::<Shared>()).store();
    let snapshot = tauri::async_runtime::spawn_blocking(move || devices::inspect(&store))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|_error| {
            "The device list is unavailable. Check the sync location and try again.".to_owned()
        })?;
    let progress = device_lock(&app.state::<SharedDevices>()).progress();
    Ok(Display { snapshot, progress })
}

#[tauri::command]
pub(super) fn device_action(app: AppHandle, request: Request) -> Result<Progress, String> {
    let now = youarel_cli::clock::now().map_err(|error| error.to_string())?;
    let target = match &request {
        Request::Invite { target }
        | Request::Join { target, .. }
        | Request::Recover { target, .. } => Some(target.clone()),
        Request::CancelPairing
        | Request::NameDevice { .. }
        | Request::RemoveDevice { .. }
        | Request::ShowRecoveryKey
        | Request::AcknowledgeRecovery => None,
    };
    let store = locked(&app.state::<Shared>()).store();
    let job = device_lock(&app.state::<SharedDevices>())
        .begin(store, request, now)
        .map_err(|error| error.to_string())?;
    if let Some(job) = job {
        if let Some(target) = target {
            let saved = locked(&app.state::<Shared>()).act(crate::intent::Intent::SyncSettings {
                enabled: true,
                target,
            });
            if let Err(error) = saved {
                device_lock(&app.state::<SharedDevices>()).finish(job.token, Err(error));
                return Err("The sync location could not be saved.".to_owned());
            }
        }
        start(&app, job);
    }
    let progress = device_lock(&app.state::<SharedDevices>()).progress();
    drop(app.emit(CHANGED, ()));
    Ok(progress)
}

fn start(app: &AppHandle, mut job: Job) {
    let handle = app.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let result = job.run(|update| {
            device_lock(&handle.state::<SharedDevices>()).update(update);
            drop(handle.emit(CHANGED, ()));
        });
        device_lock(&handle.state::<SharedDevices>()).finish(job.token, result);
        locked(&handle.state::<Shared>()).devices_changed();
        drop(handle.emit(CHANGED, ()));
        drop(handle.emit(super::SYNCED, ()));
    }));
}

#[tauri::command]
pub(super) fn take_recovery_key(app: AppHandle) -> Result<String, String> {
    device_lock(&app.state::<SharedDevices>())
        .take_recovery()
        .map_err(|error| error.to_string())
}
