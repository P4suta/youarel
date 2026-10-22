//! Device actions run outside the library lock and return only display facts to the page.

mod jobs;
mod qr;
pub use jobs::Update;
pub use jobs::{Job, Outcome};
pub use qr::CodeImage;
#[cfg(test)]
mod tests;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use youarel_cli::sync::{self, pairing::Control};
use youarel_core::device_rules::{Event, MemberId, Role, Stage};
use youarel_store::Store;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Invite { target: String },
    Join { target: String, code: String },
    CancelPairing,
    NameDevice { id: String, name: String },
    RemoveDevice { id: String, confirmed: bool },
    ShowRecoveryKey,
    AcknowledgeRecovery,
    Recover { target: String, key: String },
}

impl core::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DeviceRequest")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Preparing,
    Waiting,
    Confirming,
    Syncing,
    Done,
    Expired,
    Refused,
    Cancelled,
}

impl From<Stage> for Phase {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Offered => Self::Preparing,
            Stage::Waiting => Self::Waiting,
            Stage::Confirming => Self::Confirming,
            Stage::Syncing => Self::Syncing,
            Stage::Done => Self::Done,
            Stage::Expired => Self::Expired,
            Stage::Refused => Self::Refused,
            Stage::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub token: u64,
    pub phase: Phase,
    pub busy: bool,
    pub code: Option<String>,
    pub qr: Option<CodeImage>,
    pub error: Option<String>,
    pub recovery_ready: bool,
    pub recovery_acknowledgment: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub short: String,
    pub name: String,
    pub own: bool,
    pub added: Option<u64>,
    pub recovery: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub this_device: String,
    pub joined: bool,
    pub member: bool,
    pub devices: Vec<Device>,
}

/// # Errors
///
/// Reports unreadable membership or device-name records.
pub fn inspect(store: &Store) -> Result<Snapshot> {
    const SHORT_ID: usize = 16;
    let facts = sync::devices::inspect(store, youarel_cli::clock::now()?)?;
    let own = facts.own;
    let devices = facts
        .members
        .into_iter()
        .map(|held| {
            let id = sync::devices::identity_text(MemberId(held.device));
            Device {
                short: id.chars().take(SHORT_ID).collect(),
                id,
                name: facts
                    .names
                    .get(&MemberId(held.device))
                    .cloned()
                    .unwrap_or_default(),
                own: held.device == own,
                added: youarel_core::device_rules::displayed_time(held.added),
                recovery: held.recovery,
            }
        })
        .collect();
    Ok(Snapshot {
        this_device: sync::devices::identity_text(MemberId(own)),
        joined: facts.joined,
        member: facts.member,
        devices,
    })
}

#[derive(Default)]
pub struct Devices {
    progress: Progress,
    control: Option<Control>,
    recovery: Option<String>,
}

impl core::fmt::Debug for Devices {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Devices")
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

impl Devices {
    #[must_use]
    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }

    #[must_use]
    pub const fn busy(&self) -> bool {
        self.progress.busy
    }

    /// # Errors
    ///
    /// Refuses overlapping work, a missing recovery acknowledgment, or an exhausted run identifier.
    pub fn begin(&mut self, store: Store, request: Request, now: u64) -> Result<Option<Job>> {
        match request {
            Request::CancelPairing => {
                if let Some(control) = &self.control {
                    control.event(Event::Cancel, now);
                    self.progress.phase = control.stage().into();
                    if control.stage().terminal() {
                        self.progress.code = None;
                        self.progress.qr = None;
                    }
                }
                return Ok(None);
            }
            Request::AcknowledgeRecovery => {
                self.recovery = None;
                self.progress.recovery_ready = false;
                self.progress.recovery_acknowledgment = false;
                return Ok(None);
            }
            Request::Invite { .. }
            | Request::Join { .. }
            | Request::NameDevice { .. }
            | Request::RemoveDevice { .. }
            | Request::ShowRecoveryKey
            | Request::Recover { .. } => {}
        }
        ensure!(
            !self.progress.busy,
            "Wait for the current device action to finish."
        );
        ensure!(
            !self.progress.recovery_acknowledgment,
            "Save the recovery key and acknowledge it first."
        );
        let token = youarel_core::device_rules::next_run(self.progress.token)
            .context("The device action counter is exhausted.")?;
        let role = match request {
            Request::Invite { .. } => Some(Role::Invite),
            Request::Join { .. } => Some(Role::Join),
            Request::CancelPairing
            | Request::NameDevice { .. }
            | Request::RemoveDevice { .. }
            | Request::ShowRecoveryKey
            | Request::AcknowledgeRecovery
            | Request::Recover { .. } => None,
        };
        self.control = role.map(|role| Control::new(role, now));
        self.progress = Progress {
            token,
            phase: Phase::Preparing,
            busy: true,
            ..Progress::default()
        };
        Ok(Some(Job::new(
            store,
            request,
            (token, self.control.clone()),
        )))
    }

    pub fn update(&mut self, update: Update) {
        if self.progress.token != update.token || !self.progress.busy {
            return;
        }
        self.progress.phase = self
            .control
            .as_ref()
            .map_or(update.phase, |control| control.stage().into());
        let code = if self
            .control
            .as_ref()
            .is_some_and(|control| !control.stage().terminal())
        {
            update.code
        } else {
            None
        };
        if self.progress.code != code {
            self.progress.qr = code
                .as_deref()
                .and_then(|text| match CodeImage::encode(text) {
                    Ok(image) => Some(image),
                    Err(error) => {
                        eprintln!("{error}");
                        None
                    }
                });
            self.progress.code = code;
        }
    }

    pub fn finish(&mut self, token: u64, result: Result<Outcome>) {
        if self.progress.token != token || !self.progress.busy {
            return;
        }
        self.progress.busy = false;
        self.progress.code = None;
        self.progress.qr = None;
        match result {
            Ok(outcome) => {
                self.progress.phase = self
                    .control
                    .as_ref()
                    .map_or(Phase::Done, |control| control.stage().into());
                self.progress.recovery_ready = outcome.recovery.is_some();
                self.progress.recovery_acknowledgment = outcome.recovery.is_some();
                self.recovery = outcome.recovery;
            }
            Err(error) => {
                let terminal = self.control.as_ref().map(Control::stage);
                if matches!(terminal, Some(Stage::Cancelled | Stage::Expired)) {
                    self.progress.phase = terminal.map_or(Phase::Refused, Phase::from);
                } else {
                    self.progress.phase = Phase::Refused;
                    self.progress.error = Some(error.to_string());
                }
            }
        }
        self.control = None;
    }

    /// # Errors
    ///
    /// Refuses a second read of the one-time recovery material.
    pub fn take_recovery(&mut self) -> Result<String> {
        let key = self
            .recovery
            .take()
            .context("The recovery key has already been shown.")?;
        self.progress.recovery_ready = false;
        Ok(key)
    }
}
