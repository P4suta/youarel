//! The Workers bindings: the fetch handler and the two Durable Objects, which the runtime drives on one thread.
#![expect(
    clippy::future_not_send,
    clippy::too_many_arguments,
    clippy::unused_async_trait_impl,
    reason = "a Worker runs its futures on one thread, the binding macros generate the functions that take five arguments, and the Durable Object trait declares every handler async"
)]

use crate::store;
use futures_channel::oneshot;
use futures_util::future::{Either, select};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fmt::Write as _;
use store::{SqlAccount, SqlMail};
use worker::{
    Context, Date, Delay, DurableObject, Env, Headers, Method, Request, RequestInit, Response,
    Result, State, WebSocket, WebSocketIncomingMessage, WebSocketPair, durable_object, event,
};
use youarel_protocol::relay::Mail;
use youarel_relay::{MAIL, PICK_WAIT, REQUESTS, Reply, Status, StoreError, account, mailbox};

/// The bindings of `wrangler.toml`.
const ACCOUNTS: &str = "ACCOUNTS";
const MAILBOXES: &str = "MAILBOXES";
/// The name of the one object that holds every mailbox.
const MAILBOXES_NAME: &str = "mailboxes";
/// The path under which a member listens for an account's news, followed by the account in hexadecimal.
const EVENTS: &str = "/v1/events/";
/// The header in which the Worker tells an account's object which account a listener asked for.
const ACCOUNT_HEADER: &str = "X-Youarel-Account";
/// What a listener hears when a member publishes, and when the object admits it.
const PUBLISHED: &str = "published";
const LISTENING: &str = "listening";
/// The WebSocket close code of a refused listener, a policy violation in RFC 6455.
const POLICY: u16 = 1008;

fn hex(bytes: &[u8]) -> String {
    let mut text = String::new();
    for byte in bytes {
        // Writing to a string cannot fail.
        let _: core::fmt::Result = write!(text, "{byte:02x}");
    }
    text
}

fn failed(error: &StoreError) -> worker::Error {
    worker::Error::RustError(error.to_string())
}

fn answer(reply: Reply) -> Result<Response> {
    Ok(Response::from_bytes(reply.body)?.with_status(reply.status.code()))
}

/// A request to an object that carries `body`.
fn forwarded(url: &str, body: &[u8], headers: Headers) -> Result<Request> {
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(worker::js_sys::Uint8Array::from(body).into()));
    Request::new_with_init(url, &init)
}

#[event(fetch)]
async fn fetch(mut request: Request, env: Env, _context: Context) -> Result<Response> {
    let path = request.path();
    let url = request.url()?.to_string();
    if request.method() == Method::Post && path == REQUESTS {
        let body = request.bytes().await?;
        let Some(id) = account::route(&body) else {
            return answer(Reply::refused(
                Status::BadRequest,
                "the request is malformed",
            ));
        };
        let stub = env
            .durable_object(ACCOUNTS)?
            .id_from_name(&hex(&id))?
            .get_stub()?;
        return stub
            .fetch_with_request(forwarded(&url, &body, Headers::new())?)
            .await;
    }
    if request.method() == Method::Post && path == MAIL {
        let body = request.bytes().await?;
        let stub = env
            .durable_object(MAILBOXES)?
            .id_from_name(MAILBOXES_NAME)?
            .get_stub()?;
        return stub
            .fetch_with_request(forwarded(&url, &body, Headers::new())?)
            .await;
    }
    if let Some(name) = path.strip_prefix(EVENTS) {
        let headers = request.headers().clone();
        headers.set(ACCOUNT_HEADER, name)?;
        let mut init = RequestInit::new();
        init.with_headers(headers);
        let stub = env
            .durable_object(ACCOUNTS)?
            .id_from_name(name)?
            .get_stub()?;
        return stub
            .fetch_with_request(Request::new_with_init(&url, &init)?)
            .await;
    }
    answer(Reply::refused(
        Status::NotFound,
        "the relay serves no such path",
    ))
}

/// What an account's object remembers of a listening WebSocket across hibernation.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Listener {
    account: String,
    admitted: bool,
}

/// One account at the relay.
#[durable_object]
pub struct AccountObject {
    state: State,
}

impl core::fmt::Debug for AccountObject {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AccountObject")
            .finish_non_exhaustive()
    }
}

impl AccountObject {
    fn store(&self) -> Result<SqlAccount> {
        SqlAccount::open(self.state.storage().sql()).map_err(|error| failed(&error))
    }

    /// Tells every admitted listener that a member published.
    fn announce(&self) {
        for socket in self.state.get_websockets() {
            let admitted = socket
                .deserialize_attachment::<Listener>()
                .is_ok_and(|listener| listener.is_some_and(|listener| listener.admitted));
            if admitted {
                // A listener that has gone away hears nothing, and the next one carries on.
                drop(socket.send_with_str(PUBLISHED));
            }
        }
    }
}

impl DurableObject for AccountObject {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, mut request: Request) -> Result<Response> {
        if let Some(name) = request.headers().get(ACCOUNT_HEADER)? {
            let pair = WebSocketPair::new()?;
            self.state.accept_web_socket(&pair.server);
            pair.server.serialize_attachment(Listener {
                account: name,
                admitted: false,
            })?;
            return Response::from_websocket(pair.client);
        }
        let body = request.bytes().await?;
        let mut store = self.store()?;
        let handled = account::handle(&mut store, Date::now().as_millis(), &body)
            .map_err(|error| failed(&error))?;
        if let Some(expires) = handled.expires {
            let at = i64::try_from(expires)
                .map_err(|error| worker::Error::RustError(error.to_string()))?;
            self.state.storage().set_alarm(at).await?;
        }
        if handled.published {
            self.announce();
        }
        answer(handled.reply)
    }

    /// Deletes the account once its retention passes without a request.
    async fn alarm(&self) -> Result<Response> {
        let mut store = self.store()?;
        account::AccountStore::erase(&mut store).map_err(|error| failed(&error))?;
        for socket in self.state.get_websockets() {
            drop(socket.close(Some(POLICY), Some("the relay deleted the account")));
        }
        Response::ok("")
    }

    /// Admits a listener whose first message is a fresh collection signed by a member of the account it asked for.
    async fn websocket_message(
        &self,
        socket: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        let Some(listener) = socket.deserialize_attachment::<Listener>()? else {
            return socket.close(Some(POLICY), Some("the relay lost this listener"));
        };
        if listener.admitted {
            return Ok(());
        }
        let WebSocketIncomingMessage::Binary(bytes) = message else {
            return socket.close(
                Some(POLICY),
                Some("a listener begins with a signed request"),
            );
        };
        if account::route(&bytes).map(|id| hex(&id)).as_deref() != Some(listener.account.as_str()) {
            return socket.close(Some(POLICY), Some("the request names another account"));
        }
        let mut store = self.store()?;
        let handled = account::handle(&mut store, Date::now().as_millis(), &bytes)
            .map_err(|error| failed(&error))?;
        if handled.reply.status != Status::Ok {
            return socket.close(
                Some(POLICY),
                Some(String::from_utf8_lossy(&handled.reply.body)),
            );
        }
        socket.serialize_attachment(Listener {
            admitted: true,
            ..listener
        })?;
        socket.send_with_str(LISTENING)
    }
}

/// Every pairing's mailbox at the relay.
#[durable_object]
pub struct MailboxObject {
    state: State,
    /// The picks that wait for a drop, which a drop wakes.
    waiting: RefCell<Vec<oneshot::Sender<()>>>,
}

impl core::fmt::Debug for MailboxObject {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MailboxObject")
            .finish_non_exhaustive()
    }
}

impl MailboxObject {
    fn handle(&self, body: &[u8]) -> Result<Reply> {
        let mut store =
            SqlMail::open(self.state.storage().sql()).map_err(|error| failed(&error))?;
        mailbox::handle(&mut store, Date::now().as_millis(), body).map_err(|error| failed(&error))
    }
}

impl DurableObject for MailboxObject {
    fn new(state: State, _env: Env) -> Self {
        Self {
            state,
            waiting: RefCell::new(Vec::new()),
        }
    }

    async fn fetch(&self, mut request: Request) -> Result<Response> {
        let body = request.bytes().await?;
        let picks = matches!(Mail::decode(&body), Ok(Mail::Pick { .. }));
        let first = self.handle(&body)?;
        if !picks {
            if first.status == Status::Ok {
                for waiter in self.waiting.borrow_mut().drain(..) {
                    // A pick that gave up waiting has dropped its receiver, and the others still wake.
                    let _: core::result::Result<(), ()> = waiter.send(());
                }
            }
            return answer(first);
        }
        if first.status != Status::NoContent {
            return answer(first);
        }
        let (woken, wakes) = oneshot::channel();
        self.waiting.borrow_mut().push(woken);
        match select(wakes, Delay::from(PICK_WAIT)).await {
            Either::Left(_) | Either::Right(_) => answer(self.handle(&body)?),
        }
    }
}
