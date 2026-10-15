//! Where the key that seals a device's secrets lives: the operating system's credential store, or a plain file a person chose for a host without one.

use crate::SyncError;
use std::path::PathBuf;
use youarel_crypto::KEY;
use youarel_store::fs;
use zeroize::Zeroizing;

/// The service under which the credential store keeps youarel's keys.
const SERVICE: &str = "youarel";
/// The variable that names the key store, `platform` by default or `file` for a host without a credential store.
pub const KEY_STORE: &str = "YOUAREL_KEY_STORE";
/// The bytes of the sealing key, as a file length.
const KEY_BYTES: u64 = 32;

/// A home for the sealing key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyStore {
    /// The Keychain on macOS, the Credential Manager on Windows, and the Secret Service elsewhere, under one entry per account directory.
    Platform { entry: String },
    /// A file beside the account, which protects nothing from a reader of the disk, for tests and hosts without a credential store.
    File(PathBuf),
}

impl KeyStore {
    /// The store [`KEY_STORE`] names for the account in `directory`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Setting`] when the variable names no known store.
    pub fn for_directory(directory: &std::path::Path) -> Result<Self, SyncError> {
        match std::env::var(KEY_STORE).as_deref() {
            Ok("file") => Ok(Self::File(directory.join("sealing.key"))),
            Ok("platform") | Err(std::env::VarError::NotPresent) => Ok(Self::Platform {
                entry: crate::hex(&youarel_crypto::digest(
                    youarel_crypto::Label::Account,
                    &[directory.display().to_string().as_bytes()],
                )),
            }),
            Ok(other) => Err(SyncError::Setting(format!(
                "{KEY_STORE} names `{other}`, which is neither `platform` nor `file`"
            ))),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(SyncError::Setting(format!("{KEY_STORE} is not UTF-8")))
            }
        }
    }

    /// The sealing key, made and kept on its first use.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::KeyStore`] when the store refuses, and [`SyncError::Crypto`] when no key can be made.
    pub fn key(&self) -> Result<Zeroizing<[u8; KEY]>, SyncError> {
        let existing = self.read()?;
        if let Some(key) = existing {
            return Ok(key);
        }
        let key = Zeroizing::new(youarel_crypto::random::<KEY>()?);
        self.write(&key)?;
        Ok(key)
    }

    fn read(&self) -> Result<Option<Zeroizing<[u8; KEY]>>, SyncError> {
        let bytes = match self {
            Self::Platform { entry } => {
                let credential = keyring::Entry::new(SERVICE, entry)
                    .map_err(|error| SyncError::KeyStore(error.to_string()))?;
                match credential.get_secret() {
                    Ok(secret) => Some(Zeroizing::new(secret)),
                    Err(keyring::Error::NoEntry) => None,
                    Err(error) => return Err(SyncError::KeyStore(error.to_string())),
                }
            }
            Self::File(path) => fs::read_bounded_if_present(path, KEY_BYTES)?.map(Zeroizing::new),
        };
        bytes
            .map(|bytes| {
                let key: [u8; KEY] = bytes.as_slice().try_into().map_err(|_wrong| {
                    SyncError::KeyStore(String::from("the sealing key has the wrong length"))
                })?;
                Ok(Zeroizing::new(key))
            })
            .transpose()
    }

    fn write(&self, key: &[u8; KEY]) -> Result<(), SyncError> {
        match self {
            Self::Platform { entry } => keyring::Entry::new(SERVICE, entry)
                .and_then(|credential| credential.set_secret(key))
                .map_err(|error| SyncError::KeyStore(error.to_string())),
            Self::File(path) => Ok(fs::replace(path, key)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KeyStore;

    #[test]
    fn a_file_store_keeps_one_key() {
        let directory = tempfile::tempdir().unwrap();
        let store = KeyStore::File(directory.path().join("sealing.key"));
        let first = store.key().unwrap();
        assert_eq!(*store.key().unwrap(), *first);
        youarel_store::fs::replace(&directory.path().join("sealing.key"), b"short").unwrap();
        store.key().unwrap_err();
    }
}
