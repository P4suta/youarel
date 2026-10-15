//! Which links a shortcut may hold, decided over the parts a WHATWG URL parser reports.
//!
//! A shortcut opens through the operating system's handler for its scheme, so any scheme but `http` and `https` could start another program.
//! The parser lowercases a scheme, so the decision compares it exactly and refuses every other spelling.

/// The scheme of plain web links.
pub const HTTP: &str = "http";
/// The scheme of encrypted web links.
pub const HTTPS: &str = "https";

/// The schemes a shortcut may hold, each of which the default browser opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Scheme {
    Http,
    Https,
}

/// Why a link is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Refusal {
    /// The scheme is neither `http` nor `https`.
    Scheme,
    /// The link names no host.
    Host,
}

/// The parts of a parsed URL the decision reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Parts<'a> {
    /// The scheme as the parser serializes it, without the colon.
    pub scheme: &'a str,
    /// The host as the parser serializes it, or `None` when the URL has none.
    pub host: Option<&'a str>,
}

/// The scheme of an admitted link.
///
/// # Errors
///
/// Returns [`Refusal::Scheme`] for any scheme but `http` and `https`, and otherwise [`Refusal::Host`] when the host is absent or empty.
pub const fn admit(parts: Parts<'_>) -> Result<Scheme, Refusal> {
    let scheme = match parts.scheme.as_bytes() {
        b"http" => Scheme::Http,
        b"https" => Scheme::Https,
        _ => return Err(Refusal::Scheme),
    };
    match parts.host {
        Some(host) if !host.is_empty() => Ok(scheme),
        Some(_) | None => Err(Refusal::Host),
    }
}

/// The properties Kani proves over every scheme of up to [`proofs::SCHEME_BOUND`] bytes and every kind of host.
#[cfg(any(test, kani))]
mod proofs {
    use super::{HTTP, HTTPS, Parts, Refusal, Scheme, admit};

    /// The longest scheme the harness draws, one byte past `https`.
    pub(super) const SCHEME_BOUND: usize = 6;

    /// The decision restated as three properties that together fix every verdict.
    pub(super) fn spec(parts: Parts<'_>) -> bool {
        let verdict = admit(parts);
        let hosted = parts.host.is_some_and(|host| !host.is_empty());
        let named = match parts.scheme {
            HTTP => Some(Scheme::Http),
            HTTPS => Some(Scheme::Https),
            _ => None,
        };
        let admitted_only_web_links = verdict.is_err() || (named.is_some() && hosted);
        let web_links_admitted = match named {
            Some(scheme) if hosted => verdict == Ok(scheme),
            Some(_) | None => true,
        };
        let scheme_refused_first = named.is_some() || verdict == Err(Refusal::Scheme);
        admitted_only_web_links && web_links_admitted && scheme_refused_first
    }

    /// A host as the harness draws it: absent, empty, or a name.
    #[cfg(kani)]
    const fn host(choice: u8) -> Option<&'static str> {
        match choice {
            0 => None,
            1 => Some(""),
            _ => Some("a"),
        }
    }

    /// Every scheme of up to six bytes compares within seven unwindings.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(8)]
    fn only_web_links_with_a_host_are_admitted() {
        let bytes: [u8; SCHEME_BOUND] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= SCHEME_BOUND);
        let Some(prefix) = bytes.get(..length) else {
            return;
        };
        let Ok(scheme) = core::str::from_utf8(prefix) else {
            kani::assume(false);
            return;
        };
        let parts = Parts {
            scheme,
            host: host(kani::any()),
        };
        kani::assert(
            spec(parts),
            "the verdict follows the scheme and then the host",
        );
        let verdict = admit(parts);
        kani::cover!(verdict == Ok(Scheme::Http), "an http link is admitted");
        kani::cover!(verdict == Ok(Scheme::Https), "an https link is admitted");
        kani::cover!(verdict == Err(Refusal::Scheme), "a scheme is refused");
        kani::cover!(verdict == Err(Refusal::Host), "a missing host is refused");
    }

    /// A `file:` link could open any program, so it must be refused; this probe claims it is admitted and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_file_link_as_admitted() {
        let parts = Parts {
            scheme: "file",
            host: Some("localhost"),
        };
        kani::assert(admit(parts).is_ok(), "NEGATIVE_LINK_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{SCHEME_BOUND, spec};
    use super::{Parts, Refusal, Scheme, admit};

    const fn parts<'a>(scheme: &'a str, host: Option<&'a str>) -> Parts<'a> {
        Parts { scheme, host }
    }

    #[test]
    fn web_links_with_a_host_are_admitted() {
        assert_eq!(admit(parts("http", Some("example.com"))), Ok(Scheme::Http));
        assert_eq!(
            admit(parts("https", Some("example.com"))),
            Ok(Scheme::Https)
        );
    }

    #[test]
    fn every_other_scheme_is_refused_before_the_host() {
        for scheme in [
            "file",
            "javascript",
            "data",
            "blob",
            "ftp",
            "ws",
            "wss",
            "web+app",
            "HTTP",
            "http ",
            "",
        ] {
            for host in [None, Some(""), Some("example.com")] {
                assert_eq!(
                    admit(parts(scheme, host)),
                    Err(Refusal::Scheme),
                    "{scheme:?} {host:?}"
                );
            }
        }
    }

    #[test]
    fn a_web_link_without_a_host_is_refused() {
        for scheme in ["http", "https"] {
            assert_eq!(admit(parts(scheme, None)), Err(Refusal::Host));
            assert_eq!(admit(parts(scheme, Some(""))), Err(Refusal::Host));
        }
    }

    #[test]
    fn the_specification_agrees_over_a_drawn_domain() {
        let schemes = ["", "h", "http", "https", "httpss", "file", "data"];
        assert!(schemes.iter().all(|scheme| scheme.len() <= SCHEME_BOUND));
        for scheme in schemes {
            for host in [None, Some(""), Some("a")] {
                assert!(spec(parts(scheme, host)), "{scheme:?} {host:?}");
            }
        }
    }
}
