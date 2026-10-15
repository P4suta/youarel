//! The icons of shortcuts as the launchers and the command line fetch them: the page's own icon, or else its site's favicon, drawn as one small square PNG.

use crate::web::{self, Page};
use anyhow::{Context, Result, anyhow};
use youarel_core::state::{Edit, IconId, ItemId};
use youarel_crypto::{Label, digest};
use youarel_link::Link;
use youarel_store::Store;

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

/// Fetches the icon of every live shortcut that shows none, one by one, and reports each outcome.
///
/// # Errors
///
/// Returns an error when the store cannot be read or written.
pub fn fill(store: &Store, now: u64) -> Result<Vec<String>> {
    let missing: Vec<(ItemId, String)> = store
        .session(now, |session| {
            Ok(session
                .state()
                .shortcuts
                .iter()
                .filter(|(_, shortcut)| !shortcut.removed.value && shortcut.icon.value.is_none())
                .map(|(id, shortcut)| (*id, shortcut.link.value.clone()))
                .collect())
        })
        .map_err(|error| anyhow!(error))?;
    if missing.is_empty() {
        return Ok(vec![String::from("every shortcut shows an icon already")]);
    }
    let mut lines = Vec::new();
    let mut found: usize = 0;
    for (id, text) in &missing {
        let icon = Link::parse(text)
            .map_err(|error| anyhow!(error))
            .and_then(|link| {
                // A page that cannot be read still leaves its site's favicon to try.
                let page = web::fetch_page(&link).unwrap_or_else(|_unread| Page::default());
                fetch(&link, &page)
            });
        match icon {
            Ok(icon) => {
                store
                    .session(now, |session| {
                        session.edit(Edit::SetIcon {
                            id: *id,
                            icon: Some(icon),
                        })
                    })
                    .map_err(|error| anyhow!(error))?;
                found = found.saturating_add(1);
            }
            Err(error) => lines.push(format!("no icon for {text}: {error:#}")),
        }
    }
    lines.push(format!(
        "fetched {found} of {} missing icons",
        missing.len()
    ));
    Ok(lines)
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
        assert_ne!(identify(b"one"), identify(b"two"));
        assert_eq!(identify(b"one"), identify(b"one"));
    }
}
