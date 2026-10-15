//! Files sealed under the device's sealing key: the commitment, the nonce, and the ciphertext, bound to the file's purpose.

use crate::SyncError;
use std::path::Path;
use youarel_crypto::KEY;
use youarel_crypto::seal::{self, COMMITMENT, NONCE, Sealed};
use youarel_protocol::wire::{Reader, Writer};
use youarel_store::fs;
use zeroize::Zeroizing;

/// The longest sealed file of an account, which an identity or a long list of epoch keys fits.
const LONGEST: u64 = 1024 * 1024;

/// Writes `plaintext` sealed under `key` for `purpose`, replacing the file whole.
///
/// # Errors
///
/// Returns a [`SyncError`] when the sealing or the write fails.
pub fn write(
    path: &Path,
    key: &[u8; KEY],
    purpose: &[u8],
    plaintext: &[u8],
) -> Result<(), SyncError> {
    let sealed = seal::seal(key, purpose, plaintext)?;
    let mut writer = Writer::new();
    writer
        .raw(&sealed.commitment)
        .raw(&sealed.nonce)
        .raw(&sealed.ciphertext);
    Ok(fs::replace(path, &writer.finish())?)
}

/// The plaintext of a sealed file, or `None` when the file does not exist.
///
/// # Errors
///
/// Returns a [`SyncError`] when the file cannot be read, is no sealed file, or does not open under `key` for `purpose`.
pub fn read(
    path: &Path,
    key: &[u8; KEY],
    purpose: &[u8],
) -> Result<Option<Zeroizing<Vec<u8>>>, SyncError> {
    let Some(bytes) = fs::read_bounded_if_present(path, LONGEST)? else {
        return Ok(None);
    };
    let mut reader = Reader::new(&bytes);
    let commitment = reader.array::<COMMITMENT>()?;
    let nonce = reader.array::<NONCE>()?;
    let ciphertext = reader
        .take(bytes.len().saturating_sub(COMMITMENT + NONCE))?
        .to_vec();
    reader.finish()?;
    let sealed = Sealed {
        commitment,
        nonce,
        ciphertext,
    };
    Ok(Some(seal::open(key, purpose, &sealed)?))
}

#[cfg(test)]
mod tests {
    use super::{read, write};

    #[test]
    fn a_sealed_file_opens_only_with_its_key_and_purpose() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secret");
        assert!(read(&path, &[1; 32], b"identity").unwrap().is_none());
        write(&path, &[1; 32], b"identity", b"keys").unwrap();
        assert_eq!(
            read(&path, &[1; 32], b"identity")
                .unwrap()
                .unwrap()
                .as_slice(),
            b"keys"
        );
        read(&path, &[2; 32], b"identity").unwrap_err();
        read(&path, &[1; 32], b"epochs").unwrap_err();
    }
}
