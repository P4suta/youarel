//! Links as people type or paste them, parsed by the WHATWG URL standard and admitted by the core.
//!
//! The `url` crate owns the syntax and its normalized serialization, and [`youarel_core::link_rules`] owns which links a shortcut may hold.

use core::fmt;
use youarel_core::link_rules::{self, Parts, Refusal, Scheme};

/// A link the core admitted, held in the parser's normalized serialization.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Link {
    url: url::Url,
    scheme: Scheme,
}

/// Why text is not a link a shortcut may hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkError {
    /// The text is not a URL.
    Syntax(url::ParseError),
    /// The text is a URL the core refuses.
    Refused(Refusal),
}

impl Link {
    /// The link `text` names, normalized by the WHATWG URL standard.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError::Syntax`] when `text` is not a URL, and [`LinkError::Refused`] when the core refuses the URL.
    pub fn parse(text: &str) -> Result<Self, LinkError> {
        let url = url::Url::parse(text).map_err(LinkError::Syntax)?;
        let scheme = link_rules::admit(Parts {
            scheme: url.scheme(),
            host: url.host_str(),
        })
        .map_err(LinkError::Refused)?;
        Ok(Self { url, scheme })
    }

    /// The normalized serialization, which parses back to the same link.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// The admitted scheme.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }
}

impl fmt::Display for Link {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(formatter, "not a URL: {error}"),
            Self::Refused(Refusal::Scheme) => {
                formatter.write_str("only http and https links can be shortcuts")
            }
            Self::Refused(Refusal::Host) => formatter.write_str("the link names no host"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Syntax(error) => Some(error),
            Self::Refused(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Link, LinkError};
    use youarel_core::link_rules::{Refusal, Scheme};

    #[test]
    fn web_links_are_admitted_in_their_normalized_form() {
        let link = Link::parse("  HTTPS://Example.COM:443/a/../b?q#f ").unwrap();
        assert_eq!(link.as_str(), "https://example.com/b?q#f");
        assert_eq!(link.scheme(), Scheme::Https);
        assert_eq!(link.to_string(), link.as_str());
        let plain = Link::parse("http://例え.jp").unwrap();
        assert_eq!(plain.as_str(), "http://xn--r8jz45g.jp/");
        assert_eq!(plain.scheme(), Scheme::Http);
    }

    #[test]
    fn links_that_could_start_another_program_are_refused() {
        for text in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<p>",
            "blob:https://example.com/id",
            "vscode://file/x",
            "mailto:me@example.com",
            "ftp://example.com/",
        ] {
            assert_eq!(
                Link::parse(text),
                Err(LinkError::Refused(Refusal::Scheme)),
                "{text}"
            );
        }
    }

    #[test]
    fn text_that_is_not_a_url_is_a_syntax_error() {
        for text in ["", "example.com", "http://", "https://exa mple.com/"] {
            let error = Link::parse(text).unwrap_err();
            assert!(matches!(error, LinkError::Syntax(_)), "{text}: {error:?}");
            assert!(error.to_string().starts_with("not a URL: "), "{error}");
            assert!(std::error::Error::source(&error).is_some());
        }
    }

    #[test]
    fn refusals_explain_themselves_without_a_source() {
        let scheme = LinkError::Refused(Refusal::Scheme);
        assert_eq!(
            scheme.to_string(),
            "only http and https links can be shortcuts"
        );
        assert!(std::error::Error::source(&scheme).is_none());
        let host = LinkError::Refused(Refusal::Host);
        assert_eq!(host.to_string(), "the link names no host");
        assert!(std::error::Error::source(&host).is_none());
    }

    #[test]
    fn the_normalized_form_parses_back_to_the_same_link() {
        for text in [
            "https://example.com",
            "HTTP://EXAMPLE.com:80/%7euser/./x/../y?a=b c#a fragment",
            "https://[::1]:8443/",
            "http://192.168.0.1./",
            "https://user:pass@example.com/",
            "https://example.com/日本語?q=値",
        ] {
            let link = Link::parse(text).unwrap();
            assert_eq!(Link::parse(link.as_str()).unwrap(), link, "{text}");
        }
    }
}
