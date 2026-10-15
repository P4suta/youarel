//! The two ways youarel reaches the web: it opens a shortcut in the default browser, and it reads a page's title once when the shortcut is added.

use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::time::Duration;
use youarel_html::title::title_of;
use youarel_link::Link;

/// How long a title fetch may take in all, redirects included.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// The redirects a title fetch follows.
pub const REDIRECTS: u32 = 5;
/// The most bytes of a page a title fetch reads.
pub const BODY_LIMIT: u64 = 512 * 1024;
/// The media type of the pages a title fetch reads.
const HTML_TYPE: &[u8] = b"text/html";
/// The user agent a title fetch names.
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

/// The title the page at `link` declares, or `None` when it is no HTML page or declares none.
///
/// # Errors
///
/// Returns an error when the page cannot be fetched within the limits.
pub fn fetch_title(link: &Link) -> Result<Option<String>> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(FETCH_TIMEOUT))
        .max_redirects(REDIRECTS)
        .user_agent(USER_AGENT)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
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
        return Ok(None);
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_vec();
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(ureq::Error::BodyExceedsLimit(_)) => bail!("{link} is larger than {BODY_LIMIT} bytes"),
        Err(error) => return Err(error).with_context(|| format!("cannot read {link}")),
    };
    Ok(title_of(&String::from_utf8_lossy(&bytes)))
}
