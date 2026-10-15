//! The icons a web page names, which a launcher shows beside its shortcut.

use crate::scan::{Tag, Token, tokens};

/// The most icon addresses a page yields, which bounds how many a launcher tries.
pub const MOST_ICONS: usize = 4;
/// The side in pixels from which a declared size counts as large enough for a launcher's icon.
const LARGE_SIDE: u32 = 64;

/// How well an icon link suits a launcher, best first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Rank {
    /// A touch icon, which pages draw large and square.
    Touch,
    /// An icon that declares a size of at least [`LARGE_SIDE`].
    Large,
    /// Any other icon.
    Plain,
}

/// The rank of an icon link, or `None` for a link that names no icon a launcher decodes.
fn rank(tag: &Tag) -> Option<Rank> {
    if tag.name != "link" || tag.closing {
        return None;
    }
    let relations = tag.attribute("rel")?.to_ascii_lowercase();
    let vector = tag
        .attribute("type")
        .is_some_and(|kind| kind.eq_ignore_ascii_case("image/svg+xml"));
    if vector {
        return None;
    }
    let mut words = relations.split_ascii_whitespace();
    if words
        .clone()
        .any(|word| word == "apple-touch-icon" || word == "apple-touch-icon-precomposed")
    {
        return Some(Rank::Touch);
    }
    if !words.any(|word| word == "icon") {
        return None;
    }
    let large = tag.attribute("sizes").is_some_and(|sizes| {
        sizes.split_ascii_whitespace().any(|size| {
            size.split_once(['x', 'X']).is_some_and(|(width, _)| {
                width.parse::<u32>().is_ok_and(|width| width >= LARGE_SIDE)
            })
        })
    });
    Some(if large { Rank::Large } else { Rank::Plain })
}

/// The addresses of the icons a page names, as the page writes them, the ones that suit a launcher best first, and at most [`MOST_ICONS`].
///
/// A link to a vector image is left out, since a launcher draws its icons from pixels.
#[must_use]
pub fn icons_of(html: &str) -> Vec<String> {
    let mut found: Vec<(Rank, usize, String)> = tokens(html)
        .iter()
        .filter_map(|token| match token {
            Token::Tag(tag) => {
                let address = tag.attribute("href")?.trim();
                let rank = rank(tag)?;
                (!address.is_empty()).then(|| (rank, address.to_owned()))
            }
            Token::Text(_) => None,
        })
        .enumerate()
        .map(|(order, (rank, address))| (rank, order, address))
        .collect();
    found.sort();
    let mut addresses: Vec<String> = Vec::new();
    for (_, _, address) in found {
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    addresses.truncate(MOST_ICONS);
    addresses
}

#[cfg(test)]
mod tests {
    use super::{MOST_ICONS, icons_of};

    #[test]
    fn a_touch_icon_comes_before_a_large_icon_and_a_large_one_before_the_rest() {
        let page = r#"<head>
            <link rel="icon" href="/favicon.ico">
            <link rel="icon" type="image/png" sizes="96x96" href="/96.png">
            <link rel="mask-icon" href="/mask.svg">
            <link rel="icon" type="image/svg+xml" href="/icon.svg">
            <LINK REL="Apple-Touch-Icon" HREF="/touch.png">
            <link rel="shortcut icon" sizes="16x16 32x32" href="/small.png">
            <link rel="stylesheet" href="/style.css">
            <link rel="icon" href="">
        </head>"#;
        assert_eq!(
            icons_of(page),
            ["/touch.png", "/96.png", "/favicon.ico", "/small.png"]
        );
    }

    #[test]
    fn repeated_icons_count_once_and_at_most_a_few_come_back() {
        let page: String = (0..10)
            .flat_map(|index| {
                [
                    String::from("<link rel=icon href=/"),
                    index.to_string(),
                    String::from(".png><link rel=icon href=/0.png>"),
                ]
            })
            .collect();
        let found = icons_of(&page);
        assert_eq!(found.len(), MOST_ICONS);
        assert_eq!(found.first().map(String::as_str), Some("/0.png"));
        assert_eq!(found.get(1).map(String::as_str), Some("/1.png"));
        assert_eq!(icons_of("<p>no icons</p>"), Vec::<String>::new());
        assert_eq!(
            icons_of(
                "</link rel=icon href=/closing.png><link rel=icon sizes=any href=/any.png><link rel=icon sizes=64x64 href=/64.png>"
            ),
            ["/64.png", "/any.png"]
        );
    }
}
