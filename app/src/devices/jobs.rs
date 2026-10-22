use super::{Phase, Request};
use anyhow::{Context as _, Result, bail, ensure};
use youarel_cli::{
    clock,
    sync::{
        self,
        pairing::{Control, Pairing},
    },
};
use youarel_core::device_rules::{PAIRING_PATIENCE, Stage};
use youarel_store::Store;

#[derive(Clone, Debug)]
pub struct Update {
    pub token: u64,
    pub phase: Phase,
    pub code: Option<String>,
}

#[derive(Default)]
pub struct Outcome {
    pub(super) recovery: Option<String>,
}

impl core::fmt::Debug for Outcome {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DeviceOutcome")
    }
}

#[derive(Debug)]
pub struct Job {
    store: Store,
    request: Option<Request>,
    pub token: u64,
    control: Option<Control>,
}

impl Job {
    pub(super) fn new(
        store: Store,
        request: Request,
        (token, control): (u64, Option<Control>),
    ) -> Self {
        Self {
            store,
            request: Some(request),
            token,
            control,
        }
    }

    /// # Errors
    ///
    /// Returns a display sentence for a failed device action without exposing its code or recovery material.
    pub fn run(&mut self, mut emit: impl FnMut(Update)) -> Result<Outcome> {
        let request = self
            .request
            .take()
            .context("This device action has already run.")?;
        let result = self.execute(request, &mut emit);
        if let Err(cause) = result {
            let message = if self
                .control
                .as_ref()
                .is_some_and(|control| control.stage() == Stage::Syncing)
            {
                "The device joined, but syncing stopped. Check the sync location and try Sync Now."
            } else {
                "The device action stopped. Check the code, device name, and sync location, then try again."
            };
            if let Some(control) = &self.control {
                control.refuse()?;
            }
            Err(cause.context(message))
        } else {
            result
        }
    }

    fn execute(&self, request: Request, emit: &mut impl FnMut(Update)) -> Result<Outcome> {
        let now = clock::now()?;
        match request {
            Request::Invite { target } => {
                let control = self
                    .control
                    .clone()
                    .context("The invitation has no progress control.")?;
                let (pairing, code) =
                    Pairing::invite(self.store.clone(), target, control.clone(), now)?;
                self.pair(pairing, (control, Some(code)), emit)?;
            }
            Request::Join { target, code } => {
                let control = self
                    .control
                    .clone()
                    .context("The join has no progress control.")?;
                let pairing =
                    Pairing::join(self.store.clone(), (target, &code), control.clone(), now)?;
                self.pair(pairing, (control, None), emit)?;
            }
            Request::NameDevice { id, name } => self.name(&id, name.trim(), now)?,
            Request::RemoveDevice { id, confirmed } => self.remove(&id, confirmed, now)?,
            Request::ShowRecoveryKey => {
                return Ok(Outcome {
                    recovery: Some(sync::devices::recovery_key(&self.store, now)?),
                });
            }
            Request::Recover { target, key } => {
                sync::devices::recover(&self.store, now, (&key, &target))?;
            }
            Request::CancelPairing | Request::AcknowledgeRecovery => {
                bail!("This action needs no worker.")
            }
        }
        Ok(Outcome::default())
    }

    fn name(&self, text: &str, name: &str, now: u64) -> Result<()> {
        let id = sync::devices::identity(text)?;
        sync::devices::name(&self.store, now, (id, name))
    }

    fn remove(&self, text: &str, confirmed: bool, now: u64) -> Result<()> {
        ensure!(confirmed, "Confirm the device removal first.");
        let id = sync::devices::identity(text)?;
        sync::devices::remove(&self.store, now, id)
    }

    fn pair(
        &self,
        mut pairing: Pairing,
        (control, code): (Control, Option<String>),
        emit: &mut impl FnMut(Update),
    ) -> Result<()> {
        let began = clock::Stopwatch::start();
        let mut shown = None;
        let ran = (|| {
            loop {
                let now = clock::now()?;
                let stage = pairing.advance(now)?;
                let phase = Phase::from(stage);
                if shown != Some(phase) {
                    emit(Update {
                        token: self.token,
                        phase,
                        code: code.clone(),
                    });
                    shown = Some(phase);
                }
                if stage.terminal() {
                    break;
                }
                ensure!(
                    began.elapsed()? < PAIRING_PATIENCE,
                    "The device joined; syncing will continue from the Devices pane."
                );
                control.wait()?;
            }
            Ok(())
        })();
        let cleanup = pairing.cleanup();
        match (ran, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("Pairing cleanup also failed: {cleanup:#}")))
            }
        }
    }
}
