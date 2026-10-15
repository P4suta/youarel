//! The optional relay: it admits a member's signed request, keeps each member's newest sealed bundle, hands the bundles to the members, and carries the mail of pairings.
//!
//! The relay never holds a key that opens a state; it checks the membership log and the signatures with the code the devices run.
//! Its storage sits behind traits, so the Worker keeps it in a Durable Object's database, and a test or a local relay keeps it in memory.

pub mod account;
pub mod directory;
#[cfg(feature = "local")]
pub mod local;
pub mod mailbox;
pub mod memory;

/// The HTTP path of a member's requests.
pub const REQUESTS: &str = "/v1/requests";
/// The HTTP path of the directory's questions.
pub const LOCATE: &str = "/v1/locate";
/// The HTTP path of a pairing's mail.
pub const MAIL: &str = "/v1/mail";
/// The media type of every body the relay reads and writes.
pub const MEDIA_TYPE: &str = "application/vnd.youarel.v1";
/// How long a pick that finds no mail waits for a drop before the relay answers that nothing came, so a pairing device asks again rather than sleeps.
pub const PICK_WAIT: core::time::Duration = core::time::Duration::from_secs(25);

/// How the relay answers, as an HTTP status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ok,
    /// A mailbox holds no message for that side yet.
    NoContent,
    BadRequest,
    /// A stale, replayed, or forged request.
    Unauthorized,
    /// A request from a device that is no member, or a bundle another device signed.
    Forbidden,
    NotFound,
    /// A bundle or a message beyond what the relay keeps.
    TooLarge,
    /// The relay keeps as many mailboxes as it allows.
    Busy,
}

/// The status codes of RFC 9110 that the relay answers with.
const HTTP_OK: u16 = 200;
const HTTP_NO_CONTENT: u16 = 204;
const HTTP_BAD_REQUEST: u16 = 400;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_CONTENT_TOO_LARGE: u16 = 413;
const HTTP_SERVICE_UNAVAILABLE: u16 = 503;

impl Status {
    /// The HTTP status code.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Ok => HTTP_OK,
            Self::NoContent => HTTP_NO_CONTENT,
            Self::BadRequest => HTTP_BAD_REQUEST,
            Self::Unauthorized => HTTP_UNAUTHORIZED,
            Self::Forbidden => HTTP_FORBIDDEN,
            Self::NotFound => HTTP_NOT_FOUND,
            Self::TooLarge => HTTP_CONTENT_TOO_LARGE,
            Self::Busy => HTTP_SERVICE_UNAVAILABLE,
        }
    }
}

/// The relay's answer: a status, and a body that holds a message or, for a refusal, its reason as text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub status: Status,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub const fn ok(body: Vec<u8>) -> Self {
        Self {
            status: Status::Ok,
            body,
        }
    }

    #[must_use]
    pub fn refused(status: Status, reason: &str) -> Self {
        Self {
            status,
            body: reason.as_bytes().to_vec(),
        }
    }
}

/// A failure of the relay's storage, which the adapter reports as a server error.
#[derive(Debug)]
pub struct StoreError(pub String);

impl core::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "the relay's storage failed: {}", self.0)
    }
}

impl std::error::Error for StoreError {}
