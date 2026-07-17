//! The passphrase key source: derives the archive's 32-byte key from a user
//! passphrase with a user-chosen KDF (Argon2id by default — memory-hard and
//! the current best-practice pick; scrypt and PBKDF2-HMAC-SHA256 as explicit
//! opt-ins, like the cipher choice), as the alternative to the OS keychain —
//! both a deliberate choice and the fallback where no keychain backend
//! exists at runtime (e.g. Linux without a Secret Service daemon).
//!
//! The KDFs themselves are `dorado_engine::kdf` — the same `derive` dispatch
//! and `validate` cost bounds dorado's own password container runs. tty owns
//! only the *scope*: dorado's container stores a fresh salt in every file
//! (each independently decryptable, KDF per read, right for standalone
//! files); tty stores one salt per archive in the sidecar below, derives
//! once per session, and feeds the cached key to the fast per-file AEAD.
//!
//! The KDF recipe — which algorithm, its salt, its cost parameters — lives
//! in a small *plaintext* JSON sidecar next to the archive
//! (`tty.history.kdf.json`): none of it is a secret, and it must be readable
//! before any key exists. **The sidecar is authoritative for an existing
//! archive**: the settings choice only picks the recipe for a *new* one, so
//! flipping the setting later can never lock the user out. There is no
//! separate "is this passphrase right?" verifier: the manifest decrypt
//! already authenticates, and keeping wrong-key indistinguishable from
//! tampering ([`Error::AuthFailed`]) is a deliberate no-oracle property of
//! the whole feature. An empty archive therefore accepts any passphrase and
//! simply starts fresh with it.

use rand::RngCore;
use zeroize::Zeroizing;

use crate::settings::HistoryKdf;

use super::crypto::{Cipher, Key};
use super::{atomic_write, Error, Result, Started};

/// Plaintext sidecar filename, in `history_dir()` alongside the manifest.
pub const SIDECAR_FILENAME: &str = "tty.history.kdf.json";

/// The shortest passphrase the enable prompt accepts. A floor against typos
/// and accidental Enter, not a strength policy — the KDF's cost parameters
/// are what make short-ish passphrases expensive to grind.
pub const MIN_PASSPHRASE_LEN: usize = 8;

const SALT_LEN: usize = 16;

/// One archive's KDF recipe: the algorithm tag plus its own cost parameters,
/// self-describing in the sidecar JSON (`"kdf": "scrypt", "log_n": …`) so an
/// unknown algorithm fails loudly instead of being misread as another's
/// parameters. Defaults follow each crate's/OWASP's current recommendation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kdf")]
pub enum KdfRecipe {
    #[serde(rename = "argon2id")]
    Argon2id {
        m_cost_kib: u32,
        t_cost: u32,
        p_cost: u32,
    },
    #[serde(rename = "scrypt")]
    Scrypt { log_n: u8, r: u32, p: u32 },
    #[serde(rename = "pbkdf2-sha256")]
    Pbkdf2Sha256 { iterations: u32 },
}

impl KdfRecipe {
    /// The default recipe for a chosen algorithm (new archives only).
    fn recommended(kdf: HistoryKdf) -> Self {
        match kdf {
            // OWASP's 19 MiB / t=2 is a floor calibrated for busy servers; a
            // once-per-session local unlock can afford far more, so we spend
            // 64 MiB and t=3 — a few tenths of a second here, a much steeper
            // bill for a GPU/ASIC guessing rig. New archives only; an existing
            // archive keeps whatever its sidecar recorded.
            HistoryKdf::Argon2id => KdfRecipe::Argon2id {
                m_cost_kib: 65_536,
                t_cost: 3,
                p_cost: 1,
            },
            // OWASP's scrypt recommendation: N = 2^17, r = 8, p = 1.
            HistoryKdf::Scrypt => KdfRecipe::Scrypt {
                log_n: 17,
                r: 8,
                p: 1,
            },
            // OWASP's PBKDF2-HMAC-SHA256 recommendation.
            HistoryKdf::Pbkdf2 => KdfRecipe::Pbkdf2Sha256 {
                iterations: 600_000,
            },
        }
    }

    /// This recipe as dorado-engine's own parameter type — the KDFs
    /// themselves are dorado's (`dorado_engine::kdf::derive`), tty only owns
    /// the JSON sidecar they're recorded in.
    fn as_dorado_params(&self) -> dorado_engine::kdf::KdfParams {
        match *self {
            KdfRecipe::Argon2id {
                m_cost_kib,
                t_cost,
                p_cost,
            } => dorado_engine::kdf::KdfParams::Argon2id {
                m_cost: m_cost_kib,
                t_cost,
                p_cost,
            },
            KdfRecipe::Scrypt { log_n, r, p } => {
                dorado_engine::kdf::KdfParams::Scrypt { log_n, r, p }
            }
            KdfRecipe::Pbkdf2Sha256 { iterations } => dorado_engine::kdf::KdfParams::Pbkdf2 {
                rounds: iterations,
                prf: dorado_engine::kdf::PrfId::HmacSha256,
            },
        }
    }
}

/// The KDF sidecar for one archive: created at first passphrase enable,
/// fixed for the archive's life (like the cipher).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KdfSidecar {
    pub version: u8,
    /// Hex-encoded random salt (not a secret — it exists so identical
    /// passphrases on different machines derive different keys).
    pub salt: String,
    #[serde(flatten)]
    pub recipe: KdfRecipe,
}

impl KdfSidecar {
    fn fresh(kdf: HistoryKdf) -> Self {
        let mut salt = [0u8; SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        Self {
            version: 1,
            salt: hex_encode(&salt),
            recipe: KdfRecipe::recommended(kdf),
        }
    }
}

/// Load the sidecar from `dir`. `Ok(None)` when it doesn't exist (a fresh
/// archive); an unreadable or malformed sidecar (including an unknown `kdf`
/// tag) is an error, not a silent fresh start — regenerating it would change
/// the salt and permanently lock the user out of an archive their passphrase
/// still opens.
pub fn load(dir: &std::path::Path) -> Result<Option<KdfSidecar>> {
    let path = dir.join(SIDECAR_FILENAME);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(Some(serde_json::from_str(&raw)?))
}

/// Load the sidecar, or mint (and persist) a fresh one for a new archive
/// under the chosen KDF. An existing sidecar wins outright — its recorded
/// recipe is what derives the key, whatever `kdf` says today.
pub fn load_or_create(dir: &std::path::Path, kdf: HistoryKdf) -> Result<KdfSidecar> {
    if let Some(existing) = load(dir)? {
        return Ok(existing);
    }
    let sidecar = KdfSidecar::fresh(kdf);
    let json = serde_json::to_string_pretty(&sidecar)?;
    atomic_write(&dir.join(SIDECAR_FILENAME), json.as_bytes())?;
    Ok(sidecar)
}

/// Derive the archive key from `passphrase` under the sidecar's recipe —
/// dorado-engine's own KDF dispatcher (the exact code the dorado container
/// uses), with its `validate` bounding the costs first: the sidecar comes
/// off disk, and an absurd cost value must fail cleanly rather than pin the
/// machine deriving for minutes or allocating gigabytes. Deliberately slow
/// (every one of these is a password stretcher) — run it off the UI thread.
pub fn derive_key(passphrase: &str, sidecar: &KdfSidecar) -> Result<Zeroizing<Key>> {
    let salt = hex_decode(&sidecar.salt)
        .ok_or_else(|| Error::Kdf("sidecar salt is not valid hex".to_string()))?;
    let params = sidecar.recipe.as_dorado_params();
    dorado_engine::kdf::validate(&params).map_err(|e| Error::Kdf(e.to_string()))?;
    let mut key = Zeroizing::new([0u8; 32]);
    dorado_engine::kdf::derive_from_password(&params, passphrase.as_bytes(), &salt, key.as_mut())
        .map_err(|e| Error::Kdf(e.to_string()))?;
    Ok(key)
}

/// Bootstrap encrypted history from a passphrase: sidecar (creating one
/// under `kdf` for a fresh archive; an existing sidecar's recipe wins), the
/// derivation, then the shared key-agnostic core. Runs the blocking work on
/// its own thread (the KDF takes a deliberate fraction of a second), lazily
/// — nothing happens until the future is polled. The error distinction
/// matters here, unlike the keychain path: [`Error::AuthFailed`] means the
/// passphrase was wrong (or the archive is corrupted — deliberately
/// indistinguishable), which the unlock prompt shows inline for a retry.
pub async fn start_async(
    cipher: Cipher,
    kdf: HistoryKdf,
    prf: dorado_engine::kdf::KdfPrf,
    passphrase: Zeroizing<String>,
) -> Result<Started> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(start_in(
            super::history_dir(),
            cipher,
            kdf,
            prf,
            &passphrase,
        ));
    });
    match rx.await {
        Ok(result) => result,
        Err(_) => Err(Error::Kdf("passphrase start thread vanished".to_string())),
    }
}

/// The synchronous, directory-explicit core of [`start_async`] — testable
/// against a tempdir.
pub(crate) fn start_in(
    dir: std::path::PathBuf,
    cipher: Cipher,
    kdf: HistoryKdf,
    prf: dorado_engine::kdf::KdfPrf,
    passphrase: &str,
) -> Result<Started> {
    let sidecar = load_or_create(&dir, kdf)?;
    let key = derive_key(passphrase, &sidecar)?;
    super::start_with_key_in(dir, cipher, key, prf)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
#[path = "passphrase_tests.rs"]
mod tests;
