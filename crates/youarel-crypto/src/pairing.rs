//! The pairing of a new device through a short code, as `specs/PROTOCOL.md` describes it.
//!
//! SPAKE2 over the Ed25519 group turns the code's forty secret bits into a key that only the two holders of the code share, so a reader of the messages cannot test codes offline.
//! The inviter also encapsulates to the joiner's X-Wing key, and the channel key derives from both secrets and the transcript, so what travels under it stays secret while either the PAKE or the key encapsulation stands.
//! Each side starts its PAKE from a seed, so a side that waits for the other's file recreates the same state from the seed it kept.

use crate::kem::{self, KeyPair};
use crate::{CryptoError, KEY, Label, derive, equal, mac, sha512};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng as _;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::Zeroizing;

/// The symbols of a code's secret: Crockford's base32, which leaves out I, L, O, and U.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The bytes of a code's secret: forty bits.
const SECRET: usize = 5;
/// The symbols that spell a code's secret, five bits each.
const SYMBOLS: usize = 8;
/// The bits one symbol of the secret spells.
const SYMBOL_BITS: u32 = 5;
/// The bits of one symbol's value.
const SYMBOL_MASK: u8 = 0b1_1111;
/// The bytes of the 64-bit word a secret's bits sit at the low end of.
const WORD: usize = 8;
/// The symbols of the secret between two separators, as a person reads them.
const GROUP: usize = 4;
/// The bytes of the number that names an invitation.
const NUMBER: usize = 2;
/// The bytes of the seed a side's PAKE starts from.
pub const SEED: usize = 32;
/// The bytes of a confirmation.
pub const CONFIRMATION: usize = 64;

/// An invitation code: a number that names the invitation, and forty secret bits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Code {
    pub number: u16,
    secret: [u8; SECRET],
}

impl core::fmt::Debug for Code {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Code")
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}

/// The value of one symbol of a secret, read the way Crockford's base32 allows.
fn symbol_value(symbol: u8) -> Option<u64> {
    let canonical = match symbol.to_ascii_uppercase() {
        b'I' | b'L' => b'1',
        b'O' => b'0',
        other => other,
    };
    CROCKFORD
        .iter()
        .zip(0_u64..)
        .find_map(|(candidate, value)| (*candidate == canonical).then_some(value))
}

impl Code {
    /// A new code from the operating system's randomness.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Random`] when the operating system gives no randomness.
    pub fn generate() -> Result<Self, CryptoError> {
        let number = u16::from_be_bytes(crate::random::<NUMBER>()?);
        Ok(Self {
            number,
            secret: crate::random::<SECRET>()?,
        })
    }

    /// The code that `text` spells: the number, then eight symbols of the secret, with any dashes or spaces between them.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (digits, symbols) = text.trim().split_once(['-', ' '])?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let Ok(number) = digits.parse::<u16>() else {
            return None;
        };
        let symbols: Vec<u8> = symbols
            .bytes()
            .filter(|byte| !matches!(byte, b'-' | b' '))
            .collect();
        if symbols.len() != SYMBOLS {
            return None;
        }
        let mut value: u64 = 0;
        for symbol in symbols {
            value = value.wrapping_shl(SYMBOL_BITS) | symbol_value(symbol)?;
        }
        let word = value.to_be_bytes();
        let mut secret = [0_u8; SECRET];
        secret.copy_from_slice(word.get(WORD.saturating_sub(SECRET)..)?);
        Some(Self { number, secret })
    }

    /// The code as a person reads it, such as `4021-K7QM-2XRD`.
    #[must_use]
    pub fn text(self) -> String {
        let mut word = [0_u8; WORD];
        if let Some(tail) = word.get_mut(WORD.saturating_sub(SECRET)..) {
            tail.copy_from_slice(&self.secret);
        }
        let mut value = u64::from_be_bytes(word);
        let mut symbols = [b'0'; SYMBOLS];
        for slot in symbols.iter_mut().rev() {
            let low = value.to_le_bytes().first().copied().unwrap_or_default() & SYMBOL_MASK;
            if let Some(character) = CROCKFORD.get(usize::from(low)) {
                *slot = *character;
            }
            value = value.wrapping_shr(SYMBOL_BITS);
        }
        let mut text = format!("{}-", self.number);
        for (index, symbol) in symbols.iter().enumerate() {
            if index == GROUP {
                text.push('-');
            }
            text.push(char::from(*symbol));
        }
        text
    }

    fn identity(self) -> Identity {
        Identity::new(format!("{}/{}", Label::Pairing.text(), self.number).as_bytes())
    }
}

/// A side's PAKE, started from `seed`, and the message it sends.
fn start(code: Code, seed: &[u8; SEED]) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_symmetric_with_rng(
        &Password::new(code.secret),
        &code.identity(),
        ChaCha20Rng::from_seed(*seed),
    )
}

/// What both sides of a pairing saw: the two PAKE messages, the joiner's public keys, and the encapsulation.
#[derive(Clone, Copy, Debug)]
pub struct Transcript<'a> {
    pub joiner_message: &'a [u8],
    pub inviter_message: &'a [u8],
    pub joiner_public: &'a [u8],
    pub encapsulation: &'a [u8],
}

impl Transcript<'_> {
    /// The SHA-512 digest of every part, each preceded by its length.
    fn digest(&self) -> Result<[u8; 64], CryptoError> {
        let mut bytes = Vec::new();
        for part in [
            self.joiner_message,
            self.inviter_message,
            self.joiner_public,
            self.encapsulation,
        ] {
            let length = u64::try_from(part.len())
                .map_err(|_huge| CryptoError::Malformed("a transcript part"))?;
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(part);
        }
        Ok(sha512(&bytes))
    }
}

/// The channel a pairing opens: its key and the digest of its transcript.
pub struct Channel {
    key: Zeroizing<[u8; KEY]>,
    transcript: [u8; 64],
}

impl core::fmt::Debug for Channel {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("Channel").finish_non_exhaustive()
    }
}

impl Channel {
    fn open(
        pake_key: &[u8],
        shared: &[u8; KEY],
        transcript: &Transcript<'_>,
    ) -> Result<Self, CryptoError> {
        let digest = transcript.digest()?;
        let mut secret = Zeroizing::new(Vec::with_capacity(pake_key.len().saturating_add(KEY)));
        secret.extend_from_slice(pake_key);
        secret.extend_from_slice(shared);
        Ok(Self {
            key: derive::<KEY>(&digest, &secret, Label::Pairing, &[])?,
            transcript: digest,
        })
    }

    /// The key that seals what travels through the channel.
    #[must_use]
    pub fn key(&self) -> &[u8; KEY] {
        &self.key
    }

    /// The inviter's proof that it holds the channel key.
    #[must_use]
    pub fn welcomed(&self) -> [u8; CONFIRMATION] {
        mac(self.key.as_slice(), Label::Welcomed, &[&self.transcript])
    }

    /// The joiner's proof that it holds the channel key.
    #[must_use]
    pub fn joined(&self) -> [u8; CONFIRMATION] {
        mac(self.key.as_slice(), Label::Joined, &[&self.transcript])
    }

    /// Whether `confirmation` is the joiner's proof for this channel.
    #[must_use]
    pub fn confirms_joined(&self, confirmation: &[u8]) -> bool {
        equal(&self.joined(), confirmation)
    }
}

/// The joiner's first step: a seed to keep, and the PAKE message for the inviter.
///
/// # Errors
///
/// Returns [`CryptoError::Random`] when the operating system gives no randomness.
pub fn join(code: Code) -> Result<(Zeroizing<[u8; SEED]>, Vec<u8>), CryptoError> {
    let seed = Zeroizing::new(crate::random::<SEED>()?);
    let (_, message) = start(code, &seed);
    Ok((seed, message))
}

/// What the inviter sends back: its PAKE message and the encapsulation, beside the channel it opened.
#[derive(Debug)]
pub struct Accepted {
    pub channel: Channel,
    pub inviter_message: Vec<u8>,
    pub encapsulation: Vec<u8>,
}

/// The inviter's step: it completes the PAKE with the joiner's message, encapsulates to the joiner's X-Wing key, and opens the channel.
///
/// # Errors
///
/// Returns [`CryptoError::Forged`] when the joiner's message is no PAKE message, and another [`CryptoError`] when a primitive fails.
pub fn accept(
    code: Code,
    joiner_message: &[u8],
    joiner_kem: &[u8],
    joiner_public: &[u8],
) -> Result<Accepted, CryptoError> {
    let seed = Zeroizing::new(crate::random::<SEED>()?);
    let (state, inviter_message) = start(code, &seed);
    let pake_key = Zeroizing::new(
        state
            .finish(joiner_message)
            .map_err(|_refused| CryptoError::Forged("the joiner's PAKE message"))?,
    );
    let (shared, encapsulation) = kem::encapsulate(joiner_kem)?;
    let transcript = Transcript {
        joiner_message,
        inviter_message: &inviter_message,
        joiner_public,
        encapsulation: &encapsulation,
    };
    let channel = Channel::open(&pake_key, &shared, &transcript)?;
    Ok(Accepted {
        channel,
        inviter_message,
        encapsulation,
    })
}

/// What the joiner kept from its first step: the seed, its X-Wing keys, and its public keys as it sent them.
#[derive(Clone, Copy, Debug)]
pub struct Joiner<'a> {
    pub seed: &'a [u8; SEED],
    pub keys: &'a KeyPair,
    pub public: &'a [u8],
}

/// What the joiner received from the inviter.
#[derive(Clone, Copy, Debug)]
pub struct Welcome<'a> {
    pub inviter_message: &'a [u8],
    pub encapsulation: &'a [u8],
    pub welcomed: &'a [u8],
}

/// The joiner's last step: it recreates its PAKE from the seed, completes it, decapsulates, and checks the inviter's proof.
///
/// # Errors
///
/// Returns [`CryptoError::Forged`] when the inviter's message or proof does not fit this code and these keys.
pub fn finish(
    code: Code,
    joiner: &Joiner<'_>,
    welcome: &Welcome<'_>,
) -> Result<Channel, CryptoError> {
    let (state, joiner_message) = start(code, joiner.seed);
    let pake_key = Zeroizing::new(
        state
            .finish(welcome.inviter_message)
            .map_err(|_refused| CryptoError::Forged("the inviter's PAKE message"))?,
    );
    let shared = joiner.keys.decapsulate(welcome.encapsulation)?;
    let transcript = Transcript {
        joiner_message: &joiner_message,
        inviter_message: welcome.inviter_message,
        joiner_public: joiner.public,
        encapsulation: welcome.encapsulation,
    };
    let channel = Channel::open(&pake_key, &shared, &transcript)?;
    if !equal(&channel.welcomed(), welcome.welcomed) {
        return Err(CryptoError::Forged("the inviter's confirmation"));
    }
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::{Channel, Code, Joiner, Welcome, accept, finish, join};
    use crate::kem::KeyPair;

    #[test]
    fn codes_read_back_from_their_text_and_forgive_the_ambiguous_letters() {
        let code = Code::generate().unwrap();
        let text = code.text();
        assert_eq!(Code::parse(&text), Some(code));
        assert_eq!(Code::parse(&text.to_lowercase()), Some(code));
        let fixed = Code {
            number: 4021,
            secret: [0, 0, 0, 0, 1],
        };
        assert_eq!(fixed.text(), "4021-0000-0001");
        assert_eq!(Code::parse("4021 oooo ooOl"), Some(fixed));
        assert_eq!(Code::parse("4021-0000-000I"), Some(fixed));
        for refused in [
            "",
            "4021",
            "4021-0000-00001",
            "4021-0000-000U",
            "-1-0000-0001",
            "65536-0000-0001",
            "+12-0000-0001",
        ] {
            assert_eq!(Code::parse(refused), None, "{refused}");
        }
        assert_eq!(
            Code::parse("65535-ZZZZ-ZZZZ").map(|parsed| parsed.secret),
            Some([0xff; 5])
        );
        assert!(!format!("{code:?}").contains(text.rsplit('-').next().unwrap()));
    }

    /// The inviter's and the joiner's channels of one pairing under a new code.
    fn paired() -> (Channel, Channel) {
        let code = Code::generate().unwrap();
        let joiner = KeyPair::generate().unwrap();
        let (seed, joiner_message) = join(code).unwrap();
        let accepted = accept(code, &joiner_message, joiner.public(), b"joiner keys").unwrap();
        let welcomed = accepted.channel.welcomed();
        let welcome = Welcome {
            inviter_message: &accepted.inviter_message,
            encapsulation: &accepted.encapsulation,
            welcomed: &welcomed,
        };
        let channel = finish(
            code,
            &Joiner {
                seed: &seed,
                keys: &joiner,
                public: b"joiner keys",
            },
            &welcome,
        )
        .unwrap();
        (accepted.channel, channel)
    }

    #[test]
    fn both_sides_of_a_pairing_open_the_same_channel() {
        let (inviter, joiner) = paired();
        assert_eq!(joiner.key(), inviter.key());
        assert!(inviter.confirms_joined(&joiner.joined()));
        assert!(!inviter.confirms_joined(&joiner.welcomed()));
        assert_ne!(joiner.joined(), joiner.welcomed());
        let (other, _) = paired();
        assert_ne!(
            other.key(),
            inviter.key(),
            "each pairing has a key of its own"
        );
        assert_ne!(other.joined(), inviter.joined());
        assert_ne!(other.welcomed(), inviter.welcomed());
    }

    #[test]
    fn a_wrong_code_a_changed_message_or_other_keys_open_no_channel() {
        let code = Code::generate().unwrap();
        let wrong = Code {
            secret: [0; 5],
            ..code
        };
        let joiner = KeyPair::generate().unwrap();
        let (seed, joiner_message) = join(code).unwrap();
        let attempt = |inviter_code: &Code, public: &[u8]| {
            let accepted = accept(*inviter_code, &joiner_message, joiner.public(), public).unwrap();
            let welcomed = accepted.channel.welcomed();
            let welcome = Welcome {
                inviter_message: &accepted.inviter_message,
                encapsulation: &accepted.encapsulation,
                welcomed: &welcomed,
            };
            finish(
                code,
                &Joiner {
                    seed: &seed,
                    keys: &joiner,
                    public: b"joiner keys",
                },
                &welcome,
            )
            .map(drop)
        };
        attempt(&wrong, b"joiner keys").unwrap_err();
        attempt(&code, b"other keys").unwrap_err();
        let stranger = KeyPair::generate().unwrap();
        let accepted = accept(code, &joiner_message, stranger.public(), b"joiner keys").unwrap();
        let welcomed = accepted.channel.welcomed();
        let welcome = Welcome {
            inviter_message: &accepted.inviter_message,
            encapsulation: &accepted.encapsulation,
            welcomed: &welcomed,
        };
        finish(
            code,
            &Joiner {
                seed: &seed,
                keys: &joiner,
                public: b"joiner keys",
            },
            &welcome,
        )
        .unwrap_err();
        accept(code, b"not a message", joiner.public(), b"joiner keys").unwrap_err();
    }
}
