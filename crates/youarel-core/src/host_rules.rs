//! How the window sets a link's host as type: the registrable name stands out, and the subdomains before it and the public suffix after it recede.
//!
//! The full public suffix list changes every week, so the decision takes the last label as the suffix, or the last two labels when they form a suffix of [`SECOND_LEVEL`].
//! An address, a host of one label such as `localhost`, or a host that ends with a dot stands out whole.
//! The decision reads the host's bytes and cuts it only at dots, which every encoding of a host keeps as one byte.

/// The two-label public suffixes the window sets apart, each ending a name that people register under it.
pub const SECOND_LEVEL: [&str; 32] = [
    "ac.jp",
    "co.jp",
    "go.jp",
    "ne.jp",
    "or.jp",
    "ac.uk",
    "co.uk",
    "gov.uk",
    "org.uk",
    "com.au",
    "net.au",
    "org.au",
    "co.nz",
    "com.br",
    "com.cn",
    "com.hk",
    "com.tw",
    "co.kr",
    "co.in",
    "com.mx",
    "com.sg",
    "co.za",
    "github.io",
    "gitlab.io",
    "pages.dev",
    "workers.dev",
    "vercel.app",
    "netlify.app",
    "web.app",
    "herokuapp.com",
    "appspot.com",
    "blogspot.com",
];

/// The separator of a host's labels.
const DOT: u8 = b'.';
/// The labels of an IPv4 address.
const IPV4_LABELS: usize = 4;
/// The bracket an IPv6 host starts with.
const IPV6_START: u8 = b'[';

/// A host in three runs that join back into it: the subdomains with their trailing dot, the registrable name, and the public suffix with its leading dot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct HostParts<'a> {
    pub sub: &'a str,
    pub name: &'a str,
    pub suffix: &'a str,
}

/// Where a host's name and its suffix start, as byte offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct Cuts {
    name: usize,
    suffix: usize,
}

/// Whether the host is an address, which has no name to set apart.
fn is_address(host: &[u8]) -> bool {
    let numeric = host
        .iter()
        .all(|byte| byte.is_ascii_digit() || *byte == DOT);
    let labels = host.split(|byte| *byte == DOT).count();
    host.first() == Some(&IPV6_START) || (numeric && labels == IPV4_LABELS)
}

/// The offset right after the last dot of `bytes`, or `None` without one.
fn after_last_dot(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .rposition(|byte| *byte == DOT)
        .map(|dot| dot.saturating_add(1))
}

/// Where the host's name and suffix start, by the suffixes of `second_level`.
fn cut(host: &[u8], second_level: &[&str]) -> Cuts {
    let whole = Cuts {
        name: 0,
        suffix: host.len(),
    };
    if is_address(host) || host.last() == Some(&DOT) {
        return whole;
    }
    let Some(last) = host.iter().rposition(|byte| *byte == DOT) else {
        return whole;
    };
    let known = |start: &usize| {
        host.get(start.saturating_add(1)..).is_some_and(|tail| {
            second_level
                .iter()
                .any(|suffix| tail.eq_ignore_ascii_case(suffix.as_bytes()))
        })
    };
    let suffix = host
        .get(..last)
        .and_then(|front| front.iter().rposition(|byte| *byte == DOT))
        .filter(known)
        .unwrap_or(last);
    let name = host
        .get(..suffix)
        .and_then(after_last_dot)
        .unwrap_or_default();
    if name >= suffix {
        return whole;
    }
    Cuts { name, suffix }
}

/// The host in the runs the window sets apart.
#[must_use]
pub fn split(host: &str) -> HostParts<'_> {
    let Cuts { name, suffix } = cut(host.as_bytes(), &SECOND_LEVEL);
    let whole = HostParts {
        sub: "",
        name: host,
        suffix: "",
    };
    // Every cut lies at a dot or at an end, which are boundaries of the text's characters.
    let Some((front, suffix)) = host.split_at_checked(suffix) else {
        return whole;
    };
    let Some((sub, name)) = front.split_at_checked(name) else {
        return whole;
    };
    HostParts { sub, name, suffix }
}

/// The properties Kani proves over every host of up to [`proofs::HOST_BOUND`] bytes drawn from a letter, a digit, a dot, and a bracket.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    use super::cut;
    use super::{Cuts, DOT};

    /// The longest host the harness draws.
    pub(super) const HOST_BOUND: usize = 5;
    /// The one two-label suffix the harness knows, which its letters spell.
    pub(super) const KNOWN: [&str; 1] = ["a.a"];

    /// What the cuts of a host promise: they lie in order within it, a nonempty host keeps a nonempty name, the subdomains end with a dot, the suffix starts with one, and the name holds none unless it is the host whole.
    pub(super) fn spec(host: &[u8], cuts: Cuts) -> bool {
        let Cuts { name, suffix } = cuts;
        let (Some(sub_run), Some(name_run), Some(suffix_run)) =
            (host.get(..name), host.get(name..suffix), host.get(suffix..))
        else {
            return false;
        };
        let named = host.is_empty() || !name_run.is_empty();
        let sub_ends = sub_run.is_empty() || sub_run.last() == Some(&DOT);
        let suffix_starts = suffix_run.is_empty() || suffix_run.first() == Some(&DOT);
        let whole = name == 0 && suffix == host.len();
        let name_clean = whole || !name_run.contains(&DOT);
        named && sub_ends && suffix_starts && name_clean
    }

    /// A byte as the harness draws it.
    #[cfg(kani)]
    const fn symbol(choice: u8) -> u8 {
        match choice {
            0 => b'.',
            1 => b'1',
            2 => b'[',
            _ => b'a',
        }
    }

    /// Every host of up to five drawn bytes scans its labels and the known suffix within a few unwindings.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(7)]
    fn a_split_host_joins_back_and_keeps_a_name() {
        let choices: [u8; HOST_BOUND] = kani::any();
        let bytes = choices.map(symbol);
        let length: usize = kani::any();
        kani::assume(length <= HOST_BOUND);
        let Some(host) = bytes.get(..length) else {
            return;
        };
        let cuts = cut(host, &KNOWN);
        kani::assert(spec(host, cuts), "the runs join back into the host");
        kani::cover!(
            cuts.name > 0 && cuts.suffix < host.len(),
            "a subdomain, a name, and a suffix"
        );
        kani::cover!(host.len() - cuts.suffix == 4, "a two-label suffix");
        kani::cover!(
            cuts.name == 0 && cuts.suffix == host.len() && host.contains(&DOT),
            "an address stands whole"
        );
    }

    /// The name of `a.ab.a` is `ab`; this probe claims the name keeps its suffix and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(7)]
    fn rejects_a_name_with_its_suffix() {
        let cuts = cut(b"a.ab.a", &[]);
        kani::assert(cuts.suffix == 6, "NEGATIVE_HOST_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{HOST_BOUND, KNOWN, spec};
    use super::{HostParts, SECOND_LEVEL, cut, split};
    use alloc::vec::Vec;

    fn runs(host: &str) -> (&str, &str, &str) {
        let HostParts { sub, name, suffix } = split(host);
        (sub, name, suffix)
    }

    #[test]
    fn the_name_stands_between_the_subdomains_and_the_suffix() {
        assert_eq!(runs("url.spec.whatwg.org"), ("url.spec.", "whatwg", ".org"));
        assert_eq!(runs("www.w3.org"), ("www.", "w3", ".org"));
        assert_eq!(runs("oklch.com"), ("", "oklch", ".com"));
        assert_eq!(runs("tenki.jp"), ("", "tenki", ".jp"));
        assert_eq!(runs("www.example.co.jp"), ("www.", "example", ".co.jp"));
        assert_eq!(runs("docs.EXAMPLE.CO.UK"), ("docs.", "EXAMPLE", ".CO.UK"));
        assert_eq!(runs("p4suta.github.io"), ("", "p4suta", ".github.io"));
        assert_eq!(
            runs("co.jp"),
            ("", "co", ".jp"),
            "a bare suffix keeps its last label apart"
        );
        assert_eq!(
            runs("xn--eckwd4c7c.xn--zckzah"),
            ("", "xn--eckwd4c7c", ".xn--zckzah")
        );
    }

    #[test]
    fn an_address_or_a_single_label_stands_whole() {
        for host in [
            "localhost",
            "192.168.1.1",
            "[::1]",
            "[2001:db8::1]",
            "",
            "a.",
            ".a",
        ] {
            assert_eq!(runs(host), ("", host, ""), "{host}");
        }
        assert_eq!(
            runs("1.2.3"),
            ("1.", "2", ".3"),
            "three numeric labels are no address"
        );
        assert_eq!(
            runs("1.2.3.4.5"),
            ("1.2.3.", "4", ".5"),
            "five numeric labels are no address"
        );
    }

    #[test]
    fn every_known_suffix_has_two_lowercase_labels() {
        for suffix in SECOND_LEVEL {
            assert_eq!(suffix.split('.').count(), 2, "{suffix}");
            assert_eq!(suffix, suffix.to_ascii_lowercase());
        }
    }

    #[test]
    fn the_specification_agrees_over_a_drawn_domain() {
        let symbols = *b".1[a";
        let mut hosts: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        for _ in 0..HOST_BOUND {
            let longer: Vec<Vec<u8>> = hosts
                .iter()
                .flat_map(|host| {
                    symbols.iter().map(move |symbol| {
                        let mut grown = host.clone();
                        grown.push(*symbol);
                        grown
                    })
                })
                .collect();
            hosts.extend(longer);
        }
        for host in &hosts {
            assert!(spec(host, cut(host, &KNOWN)), "{host:?}");
            assert!(spec(host, cut(host, &SECOND_LEVEL)), "{host:?}");
        }
        for host in ["url.spec.whatwg.org", "www.example.co.jp", "192.168.1.1"] {
            assert!(
                spec(host.as_bytes(), cut(host.as_bytes(), &SECOND_LEVEL)),
                "{host}"
            );
        }
        assert_eq!(
            cut(b"a.a.a", &KNOWN).suffix,
            1,
            "the known suffix takes two labels"
        );
    }
}
