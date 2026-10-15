//! A relay that serves HTTP on this host and keeps everything in memory, for the end-to-end tests and for people who keep their own relay.

use crate::memory::MemoryRelay;
use crate::{LOCATE, MAIL, MEDIA_TYPE, PICK_WAIT, REQUESTS, Reply, Status};
use std::io::Read as _;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use youarel_protocol::relay::{LONGEST_FRAME, LONGEST_PAYLOAD, Mail};

/// A running local relay.
pub struct Local {
    server: Arc<tiny_http::Server>,
    worker: Option<JoinHandle<()>>,
    port: u16,
}

impl core::fmt::Debug for Local {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Local")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Local {
    /// The port the relay listens on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            drop(worker.join());
        }
    }
}

/// The relay's state, with a signal for the picks that wait for mail.
struct Shared {
    relay: Mutex<MemoryRelay>,
    mail: Condvar,
}

fn failed(error: &crate::StoreError) -> Reply {
    Reply::refused(Status::Busy, &error.to_string())
}

fn locked(shared: &Shared) -> MutexGuard<'_, MemoryRelay> {
    shared.relay.lock().unwrap_or_else(PoisonError::into_inner)
}

mod raw {
    #![expect(
        clippy::significant_drop_tightening,
        reason = "the condition variable takes the guard for its wait, which the lint does not follow"
    )]

    use super::{PICK_WAIT, PoisonError, Reply, Shared, Status, failed, locked};

    /// Takes mail for one side, and when none waits, waits once for a drop before it looks again, under one lock so that no drop slips between.
    pub(super) fn pick(shared: &Shared, clock: &(impl Fn() -> u64 + ?Sized), body: &[u8]) -> Reply {
        let mut relay = locked(shared);
        let first = relay
            .mail(clock(), body)
            .unwrap_or_else(|error| failed(&error));
        if first.status != Status::NoContent {
            return first;
        }
        let (mut relay, _) = shared
            .mail
            .wait_timeout(relay, PICK_WAIT)
            .unwrap_or_else(PoisonError::into_inner);
        relay
            .mail(clock(), body)
            .unwrap_or_else(|error| failed(&error))
    }
}

use raw::pick;

/// Answers one message.
fn answer(shared: &Shared, clock: &(impl Fn() -> u64 + ?Sized), url: &str, body: &[u8]) -> Reply {
    match url {
        REQUESTS => locked(shared)
            .request(clock(), body)
            .unwrap_or_else(|error| failed(&error)),
        LOCATE => locked(shared)
            .locate(body)
            .unwrap_or_else(|error| failed(&error)),
        MAIL if matches!(Mail::decode(body), Ok(Mail::Pick { .. })) => pick(shared, clock, body),
        MAIL => {
            let reply = locked(shared)
                .mail(clock(), body)
                .unwrap_or_else(|error| failed(&error));
            shared.mail.notify_all();
            reply
        }
        _ => Reply::refused(Status::NotFound, "the relay serves no such path"),
    }
}

/// Serves a relay on `address`, such as `127.0.0.1:0`, with `clock` for its time, until the returned value drops.
///
/// # Errors
///
/// Returns the reason when the address cannot be bound.
pub fn serve(
    address: &str,
    clock: impl Fn() -> u64 + Send + Sync + 'static,
) -> Result<Local, Box<dyn std::error::Error + Send + Sync>> {
    let server = Arc::new(tiny_http::Server::http(address)?);
    let port = server
        .server_addr()
        .to_ip()
        .map(|bound| bound.port())
        .ok_or("the relay listens on no IP address")?;
    let limit = u64::try_from(LONGEST_PAYLOAD + LONGEST_FRAME)?;
    let shared = Arc::new(Shared {
        relay: Mutex::new(MemoryRelay::default()),
        mail: Condvar::new(),
    });
    let clock: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(clock);
    let listening = Arc::clone(&server);
    let worker = std::thread::spawn(move || {
        for mut request in listening.incoming_requests() {
            let shared = Arc::clone(&shared);
            let clock = Arc::clone(&clock);
            std::thread::spawn(move || {
                let mut body = Vec::new();
                let read = request.as_reader().take(limit).read_to_end(&mut body);
                let reply = match (request.method(), read) {
                    (tiny_http::Method::Post, Ok(_)) => {
                        answer(&shared, clock.as_ref(), request.url(), &body)
                    }
                    _ => Reply::refused(
                        Status::BadRequest,
                        "the relay takes only a POST of one message",
                    ),
                };
                let mut response = tiny_http::Response::from_data(reply.body)
                    .with_status_code(reply.status.code());
                if let Ok(header) =
                    tiny_http::Header::from_bytes(b"Content-Type", MEDIA_TYPE.as_bytes())
                {
                    response.add_header(header);
                }
                drop(request.respond(response));
            });
        }
    });
    Ok(Local {
        server,
        worker: Some(worker),
        port,
    })
}
