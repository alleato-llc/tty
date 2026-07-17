//! OS keychain key management for encrypted history: get-or-create a random
//! 32-byte key on first use. macOS Keychain / Windows Credential Manager /
//! Secret Service on Linux, via `keyring`. No prompt modal, no KDF — the key
//! itself is already high-entropy random bytes, stored as a raw secret
//! (`Entry::get_secret`/`set_secret`, not the string-oriented
//! `get_password`/`set_password`, since this isn't human-typed).

use keyring::Entry;
use rand::RngCore;
use zeroize::Zeroizing;

use super::crypto::Key;
use super::{Error, Result};

const SERVICE: &str = "tty";
const USERNAME: &str = "encrypted-history-key";

/// Get the stored history key, generating and storing a fresh random one the
/// first time this is called. The returned key is wiped from memory on drop.
pub fn get_or_create_key() -> Result<Zeroizing<Key>> {
    let entry = Entry::new(SERVICE, USERNAME)?;
    // This read can block on an OS keychain-access dialog when the stored
    // item was created by a differently-signed build of this binary (every
    // ad-hoc dev rebuild looks like a new app to the keychain ACL) — the
    // "before" log line makes a mid-dialog freeze diagnosable.
    tracing::info!("encrypted history: reading key from the OS keychain");
    match entry.get_secret() {
        Ok(raw) => {
            tracing::info!("encrypted history: keychain key found");
            let raw = Zeroizing::new(raw);
            let key: Key = raw
                .as_slice()
                .try_into()
                .map_err(|_| Error::MalformedKey(raw.len()))?;
            Ok(Zeroizing::new(key))
        }
        Err(keyring::Error::NoEntry) => {
            tracing::info!("encrypted history: no keychain key yet, creating one");
            let mut key = Zeroizing::new([0u8; 32]);
            rand::rngs::OsRng.fill_bytes(key.as_mut());
            entry.set_secret(key.as_ref())?;
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}
