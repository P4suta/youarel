//! The ways youarel reaches the web: it opens a shortcut in the default browser, reads a page's title and icon when the shortcut is added, and downloads a release when it updates itself.

use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::time::Duration;
use youarel_html::icon::icons_of;
use youarel_html::title::title_of;
use youarel_link::Link;

/// How long a fetch of a page or an icon may take in all, redirects included.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a release download may take in all, redirects included.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// How long one exchange with a relay may take, longer than a pick waits there for mail.
pub const RELAY_TIMEOUT: Duration = Duration::from_secs(60);
/// The redirects a fetch follows.
pub const REDIRECTS: u32 = 5;
/// The most bytes of a page a title fetch reads.
pub const BODY_LIMIT: u64 = 512 * 1024;
/// The media type of the pages a title fetch reads.
const HTML_TYPE: &[u8] = b"text/html";
/// The user agent a fetch names.
pub const USER_AGENT: &str = concat!("youarel/", env!("CARGO_PKG_VERSION"));

/// Opens a link in the default browser, admitting it again first, whatever the store held.
///
/// # Errors
///
/// Returns an error when the link is not one a shortcut may hold, or the browser cannot start.
pub fn open(link: &str) -> Result<()> {
    let admitted = Link::parse(link).with_context(|| format!("refusing to open `{link}`"))?;
    webbrowser::open(admitted.as_str())
        .with_context(|| format!("cannot open {admitted} in the default browser"))
}

/// The TLS settings of every fetch: the operating system verifies the certificates.
fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build()
}

/// An HTTP client whose certificates the operating system verifies, which gives up after `timeout`.
fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .tls_config(tls())
        .timeout_global(Some(timeout))
        .max_redirects(REDIRECTS)
        .user_agent(USER_AGENT)
        .build();
    ureq::Agent::new_with_config(config)
}

/// Posts one message to the relay at `url` and returns the status and the body of its answer, no more than `limit` bytes.
///
/// A refusal is an answer like any other, and the relay's answers never redirect.
///
/// # Errors
///
/// Returns an error when the relay cannot be reached within [`RELAY_TIMEOUT`] or its answer exceeds the limit.
pub fn post(url: &str, message: &[u8], limit: u64) -> Result<(u16, Vec<u8>)> {
    let config = ureq::Agent::config_builder()
        .tls_config(tls())
        .timeout_global(Some(RELAY_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build();
    let mut response = ureq::Agent::new_with_config(config)
        .post(url)
        .header("Content-Type", youarel_relay::MEDIA_TYPE)
        .send(message)
        .with_context(|| format!("cannot reach the relay at {url}"))?;
    let status = response.status().as_u16();
    Ok((status, body(&mut response, url, limit)?))
}

/// The body of a response, no more than `limit` bytes.
fn body(response: &mut ureq::http::Response<ureq::Body>, url: &str, limit: u64) -> Result<Vec<u8>> {
    match response.body_mut().with_config().limit(limit).read_to_vec() {
        Ok(bytes) => Ok(bytes),
        Err(ureq::Error::BodyExceedsLimit(_)) => bail!("{url} is larger than {limit} bytes"),
        Err(error) => Err(error).with_context(|| format!("cannot read {url}")),
    }
}

/// What a page declares: its title, and the addresses of its icons as the page writes them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Page {
    pub title: Option<String>,
    pub icons: Vec<String>,
}

/// What the page at `link` declares, or nothing when it is no HTML page.
///
/// # Errors
///
/// Returns an error when the page cannot be fetched within the limits.
pub fn fetch_page(link: &Link) -> Result<Page> {
    let mut response = agent(FETCH_TIMEOUT)
        .get(link.as_str())
        .call()
        .with_context(|| format!("cannot fetch {link}"))?;
    let html = response.headers().get("content-type").is_some_and(|value| {
        value
            .as_bytes()
            .get(..HTML_TYPE.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(HTML_TYPE))
    });
    if !html {
        return Ok(Page::default());
    }
    let bytes = body(&mut response, link.as_str(), BODY_LIMIT)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(Page {
        title: title_of(&text),
        icons: icons_of(&text),
    })
}

/// The image at an admitted `link`, no more than an icon's source may hold.
///
/// # Errors
///
/// Returns an error when the image cannot be fetched within the limits.
pub fn fetch_image(link: &Link) -> Result<Vec<u8>> {
    let mut response = agent(FETCH_TIMEOUT)
        .get(link.as_str())
        .call()
        .with_context(|| format!("cannot fetch {link}"))?;
    body(
        &mut response,
        link.as_str(),
        u64::try_from(youarel_icon::LONGEST_SOURCE)?,
    )
}

/// The bytes at `url`, no more than `limit`, for a release download.
///
/// # Errors
///
/// Returns an error when the address answers with a failure, the download exceeds the limit, or it takes longer than [`DOWNLOAD_TIMEOUT`].
pub fn fetch_bytes(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = agent(DOWNLOAD_TIMEOUT)
        .get(url)
        .call()
        .with_context(|| format!("cannot fetch {url}"))?;
    body(&mut response, url, limit)
}
