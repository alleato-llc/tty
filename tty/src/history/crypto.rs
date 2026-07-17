//! Dual-cipher AEAD wrap/unwrap for persisted history files (day segments and
//! the manifest). Every wrapped blob starts with a `cipher_id` byte that fixes
//! the nonce length that follows — self-describing the same way dorado's own
//! container format tags its `kdf_id`/`mac_id`. Cipher choice is fixed for the
//! life of an archive (see `Settings::history_cipher`); this tag exists so a
//! corrupted byte fails cleanly instead of being misread as the wrong nonce
//! length, not to support switching ciphers file-to-file.

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use rand::RngCore;

use super::{Error, Result};

/// A resolved 32-byte key for either cipher — both take a raw 32-byte key
/// directly (dorado's raw-authenticated path splits it into subkeys
/// internally; ChaCha20-Poly1305 uses it as-is).
pub type Key = [u8; 32];

/// A non-secret, fixed tweak for dorado's raw-authenticated construction.
/// Tweaks are Threefish's non-secret tuning input, not a key component —
/// there is nothing tty needs to vary it for.
const DORADO_TWEAK: [u8; 16] = [0u8; 16];

/// Chunk size for dorado's raw-authenticated construction. History files are
/// tiny (a day's worth of command-only entries, or the manifest), so this
/// only ever needs to fit everything in a single frame; reusing dorado's own
/// default keeps this consistent with dorado's own convention rather than
/// inventing a new constant.
const DORADO_CHUNK_BYTES: u32 = dorado_engine::DEFAULT_CHUNK_BYTES;

/// Which cipher encrypted a blob — the leading byte of every wrapped file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cipher {
    ChaCha20Poly1305,
    /// dorado's raw-key authenticated construction (Threefish-256-CTR +
    /// Skein-512-MAC). See `../../../../../dorado/docs/spec.md`'s "Raw-key
    /// modes" for the byte-level construction.
    DoradoRawAuthenticated,
}

impl Cipher {
    fn id(self) -> u8 {
        match self {
            Cipher::ChaCha20Poly1305 => 0,
            Cipher::DoradoRawAuthenticated => 1,
        }
    }

    fn from_id(id: u8) -> Result<Self> {
        match id {
            0 => Ok(Cipher::ChaCha20Poly1305),
            1 => Ok(Cipher::DoradoRawAuthenticated),
            other => Err(Error::UnknownCipher(other)),
        }
    }

    /// The `Settings::history_cipher` string this cipher is stored/selected
    /// as — distinct from [`Self::id`], which is the on-disk numeric tag.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            Cipher::ChaCha20Poly1305 => "chacha20poly1305",
            Cipher::DoradoRawAuthenticated => "dorado",
        }
    }

    /// Parse a `Settings::history_cipher` value. Unrecognized or absent
    /// (`None`) falls back to the default, `ChaCha20Poly1305` — cipher choice
    /// is meant to be a deliberate, explicit opt-in for the alternative, not
    /// something a malformed settings file can accidentally select.
    pub fn from_setting_str(s: Option<&str>) -> Self {
        match s {
            Some("dorado") => Cipher::DoradoRawAuthenticated,
            _ => Cipher::ChaCha20Poly1305,
        }
    }

    /// Nonce/IV length this cipher needs — ChaCha20-Poly1305's is fixed at 12
    /// bytes by the construction; dorado's raw-authenticated IV must match its
    /// block length, 32 bytes for the Threefish-256 variant used here.
    fn nonce_len(self) -> usize {
        match self {
            Cipher::ChaCha20Poly1305 => 12,
            Cipher::DoradoRawAuthenticated => 32,
        }
    }
}

/// A human-readable name for the settings UI — distinct from
/// [`Cipher::as_setting_str`], which is an on-disk/settings identifier, not a
/// display label. In particular, "dorado" alone names the sibling project,
/// not a cipher; the cipher it provides here is Threefish-256.
impl std::fmt::Display for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cipher::ChaCha20Poly1305 => write!(f, "ChaCha20-Poly1305"),
            Cipher::DoradoRawAuthenticated => write!(f, "Threefish-256 (dorado)"),
        }
    }
}

/// Encrypt `plaintext` with `cipher`, keyed by `key`. Returns the concatenation
/// of a `cipher_id` byte, the nonce, and the ciphertext+tag — everything a
/// matching [`unwrap`] needs, with nothing else written to the stream (no
/// header beyond that one self-describing byte).
pub fn wrap(cipher: Cipher, key: &Key, plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = vec![0u8; cipher.nonce_len()];
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let ciphertext = match cipher {
        Cipher::ChaCha20Poly1305 => {
            let aead = ChaCha20Poly1305::new(key.into());
            let n = chacha20poly1305::Nonce::from_slice(&nonce);
            aead.encrypt(n, plaintext)
                .expect("chacha20poly1305 encryption over a valid key/nonce never fails")
        }
        Cipher::DoradoRawAuthenticated => dorado_engine::encrypt_raw_authenticated_bytes(
            dorado_engine::Variant::T256,
            key,
            &DORADO_TWEAK,
            &nonce,
            dorado_engine::MacId::Skein512,
            DORADO_CHUNK_BYTES,
            plaintext,
        )
        .expect("valid fixed-length key/tweak/iv over a bounded chunk size never fails"),
    };

    let mut out = Vec::with_capacity(1 + nonce.len() + ciphertext.len());
    out.push(cipher.id());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

/// Decrypt data produced by [`wrap`], given `key`. Never panics and never
/// returns plaintext without having verified it: truncation, an unrecognized
/// `cipher_id`, and authentication failure (tampering, corruption, or a wrong
/// key — deliberately indistinguishable) are all reported as an [`Error`],
/// never as garbage bytes.
pub fn unwrap(key: &Key, data: &[u8]) -> Result<Vec<u8>> {
    let (&cipher_id, rest) = data.split_first().ok_or(Error::Truncated)?;
    let cipher = Cipher::from_id(cipher_id)?;
    let nonce_len = cipher.nonce_len();
    if rest.len() < nonce_len {
        return Err(Error::Truncated);
    }
    let (nonce, ciphertext) = rest.split_at(nonce_len);

    match cipher {
        Cipher::ChaCha20Poly1305 => {
            let aead = ChaCha20Poly1305::new(key.into());
            let n = chacha20poly1305::Nonce::from_slice(nonce);
            aead.decrypt(n, ciphertext).map_err(|_| Error::AuthFailed)
        }
        Cipher::DoradoRawAuthenticated => dorado_engine::decrypt_raw_authenticated_bytes(
            dorado_engine::Variant::T256,
            key,
            &DORADO_TWEAK,
            nonce,
            dorado_engine::MacId::Skein512,
            DORADO_CHUNK_BYTES,
            ciphertext,
        )
        .map_err(|_| Error::AuthFailed),
    }
}

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;
