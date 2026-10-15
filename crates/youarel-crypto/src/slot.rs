//! An epoch key wrapped for one device: an X-Wing encapsulation to the device and the key sealed under a key derived from it.
//!
//! The derived key seals one epoch key once, so the seal uses ChaCha20-Poly1305 with a fixed nonce, and binds the account, the epoch, and the device.

use crate::kem::{self, KeyPair};
use crate::{CryptoError, DIGEST, KEY, Label, derive};
use libcrux_chacha20poly1305::{NONCE_LEN, TAG_LEN};
use zeroize::Zeroizing;

/// The bytes of a sealed epoch key.
pub const SEALED: usize = KEY + TAG_LEN;

/// An epoch key wrapped for one device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot {
    pub device: [u8; DIGEST],
    pub encapsulation: Vec<u8>,
    pub sealed: [u8; SEALED],
}

/// Where a slot belongs: the account, the epoch, and the device, which the derivation binds.
#[derive(Clone, Copy, Debug)]
pub struct Binding<'a> {
    pub account: &'a [u8; DIGEST],
    pub epoch: u32,
    pub device: &'a [u8; DIGEST],
}

fn slot_key(
    shared: &[u8; KEY],
    binding: &Binding<'_>,
) -> Result<Zeroizing<[u8; KEY]>, CryptoError> {
    derive::<KEY>(
        &[],
        shared,
        Label::Slot,
        &[
            binding.account,
            &binding.epoch.to_be_bytes(),
            binding.device,
        ],
    )
}

/// The epoch key wrapped for the device whose X-Wing public key is `public`.
///
/// # Errors
///
/// Returns a [`CryptoError`] when the public key is malformed or a primitive fails.
pub fn wrap(
    epoch_key: &[u8; KEY],
    public: &[u8],
    binding: &Binding<'_>,
) -> Result<Slot, CryptoError> {
    let (shared, encapsulation) = kem::encapsulate(public)?;
    let key = slot_key(&shared, binding)?;
    let mut sealed = [0_u8; SEALED];
    libcrux_chacha20poly1305::encrypt(&key, epoch_key, &mut sealed, &[], &[0; NONCE_LEN])
        .map_err(|_refused| CryptoError::Primitive("ChaCha20-Poly1305 sealing"))?;
    Ok(Slot {
        device: *binding.device,
        encapsulation,
        sealed,
    })
}

/// The epoch key a slot holds for this device.
///
/// # Errors
///
/// Returns [`CryptoError::Forged`] when the slot belongs to another device, account, or epoch, or was changed.
pub fn unwrap(
    slot: &Slot,
    keys: &KeyPair,
    binding: &Binding<'_>,
) -> Result<Zeroizing<[u8; KEY]>, CryptoError> {
    if slot.device != *binding.device {
        return Err(CryptoError::Forged("a key slot for another device"));
    }
    let shared = keys.decapsulate(&slot.encapsulation)?;
    let key = slot_key(&shared, binding)?;
    let mut epoch_key = Zeroizing::new([0_u8; KEY]);
    libcrux_chacha20poly1305::decrypt(
        &key,
        epoch_key.as_mut_slice(),
        &slot.sealed,
        &[],
        &[0; NONCE_LEN],
    )
    .map_err(|_refused| CryptoError::Forged("the key slot"))?;
    Ok(epoch_key)
}

#[cfg(test)]
mod tests {
    use super::{Binding, unwrap, wrap};
    use crate::kem::KeyPair;

    #[test]
    fn a_slot_opens_only_for_its_device_account_and_epoch() {
        let keys = KeyPair::generate().unwrap();
        let other = KeyPair::generate().unwrap();
        let (account, device, stranger) = ([1_u8; 32], [2_u8; 32], [3_u8; 32]);
        let binding = Binding {
            account: &account,
            epoch: 4,
            device: &device,
        };
        let slot = wrap(&[9; 32], keys.public(), &binding).unwrap();
        assert_eq!(*unwrap(&slot, &keys, &binding).unwrap(), [9; 32]);
        unwrap(&slot, &other, &binding).unwrap_err();
        unwrap(
            &slot,
            &keys,
            &Binding {
                epoch: 5,
                ..binding
            },
        )
        .unwrap_err();
        unwrap(
            &slot,
            &keys,
            &Binding {
                account: &stranger,
                ..binding
            },
        )
        .unwrap_err();
        unwrap(
            &slot,
            &keys,
            &Binding {
                device: &stranger,
                ..binding
            },
        )
        .unwrap_err();
        let mut changed = slot;
        changed.sealed[0] ^= 1;
        unwrap(&changed, &keys, &binding).unwrap_err();
    }
}
