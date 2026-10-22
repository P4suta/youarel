//! The icons of shortcuts: the page's own icon, or else its site's favicon, drawn as one small square PNG and kept on this device under the site's host.
//!
//! A device gathers an icon only when a person acts: an addition reads the page anyway, and an opening reaches the site through the browser anyway.

use crate::web::{self, Page};
use anyhow::{Context, Result, anyhow};
use youarel_crypto::{Label, digest};
use youarel_link::Link;
use youarel_store::icons::IconId;
use youarel_store::{Session, Store, StoreError};

pub mod local;

/// The icon every site may keep at its root, which a page that names no icon still has.
const FAVICON: &str = "/favicon.ico";

/// An icon's identifier: the digest of its drawn image.
#[must_use]
pub fn identify(image: &[u8]) -> IconId {
    IconId(digest(Label::Icon, &[image]))
}

/// The admitted addresses to try for the icon of the page at `link`: the ones the page names, best first, then the site's favicon.
#[must_use]
pub fn candidates(link: &Link, page: &Page) -> Vec<Link> {
    let Ok(base) = url::Url::parse(link.as_str()) else {
        return Vec::new();
    };
    let mut found: Vec<Link> = Vec::new();
    for address in page.icons.iter().map(String::as_str).chain([FAVICON]) {
        let admitted = base
            .join(address)
            .map_err(anyhow::Error::from)
            .and_then(|joined| Link::parse(joined.as_str()).map_err(|error| anyhow!(error)));
        if let Ok(admitted) = admitted
            && !found.contains(&admitted)
        {
            found.push(admitted);
        }
    }
    found
}

/// The icon of the page at `link`: the first candidate that fetches and draws, with its identifier.
///
/// # Errors
///
/// Returns the last candidate's failure when none yields an icon.
pub fn fetch(link: &Link, page: &Page) -> Result<(IconId, Vec<u8>)> {
    let mut last = anyhow!("{link} names no icon");
    for candidate in candidates(link, page) {
        let drawn = web::fetch_image(&candidate).and_then(|image| {
            youarel_icon::normalize(&image).with_context(|| format!("{candidate} holds no icon"))
        });
        match drawn {
            Ok(image) => return Ok((identify(&image), image)),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// The favicon of the site of `link`, drawn, with its identifier; the page itself is not read.
///
/// # Errors
///
/// Returns the failure when the site offers no favicon that draws.
pub fn favicon(link: &Link) -> Result<(IconId, Vec<u8>)> {
    fetch(link, &Page::default())
}

/// Keeps an icon on this device for the host of the link it came for.
///
/// # Errors
///
/// Returns [`StoreError::Image`] for an image larger than the device keeps.
pub fn keep(
    session: &mut Session,
    link: &Link,
    (image, bytes): (IconId, Vec<u8>),
) -> Result<(), StoreError> {
    session.keep_icon(link.host(), image, &bytes)
}

/// Whether this device keeps no icon for the host of `link`.
///
/// # Errors
///
/// Returns an error when the store cannot be read.
pub fn lacking(store: &Store, now: u64, link: &Link) -> Result<bool> {
    Ok(store.session(now, |session| Ok(session.icon(link.host())?.is_none()))?)
}

/// Gathers the favicon of the site of a link a person opened, when this device keeps no icon for its host.
///
/// # Errors
///
/// Returns an error when the store cannot be read or written; a site that offers no favicon is no error, and its host keeps its tile until the next opening asks again.
pub fn opened(store: &Store, now: u64, link: &Link) -> Result<()> {
    if !lacking(store, now, link)? {
        return Ok(());
    }
    match favicon(link) {
        Ok(icon) => Ok(store.session(now, |session| keep(session, link, icon))?),
        // A site without a favicon keeps its tile, and the next opening asks it again.
        Err(_no_favicon) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Page, candidates, identify};
    use youarel_link::Link;

    #[test]
    fn a_page_s_icons_come_first_and_its_favicon_last_each_once() {
        let link = Link::parse("https://example.com/docs/page").unwrap();
        let page = Page {
            title: None,
            icons: vec![
                "/touch.png".to_owned(),
                "icon.png".to_owned(),
                "https://cdn.example.net/i.png".to_owned(),
                "javascript:alert(1)".to_owned(),
                "/favicon.ico".to_owned(),
            ],
        };
        let found: Vec<String> = candidates(&link, &page)
            .iter()
            .map(|link| link.as_str().to_owned())
            .collect();
        assert_eq!(
            found,
            [
                "https://example.com/touch.png",
                "https://example.com/docs/icon.png",
                "https://cdn.example.net/i.png",
                "https://example.com/favicon.ico",
            ]
        );
        assert_eq!(
            candidates(&link, &Page::default())
                .iter()
                .map(Link::as_str)
                .collect::<Vec<_>>(),
            ["https://example.com/favicon.ico"],
            "a site's favicon stands alone without a page"
        );
        assert_ne!(identify(b"one"), identify(b"two"));
        assert_eq!(identify(b"one"), identify(b"one"));
    }

    #[test]
    fn a_drawn_icon_fits_what_the_device_keeps() {
        let longest = u64::try_from(youarel_icon::LONGEST_ICON).unwrap();
        assert!(longest <= youarel_store::icons::LONGEST_IMAGE);
    }
}
