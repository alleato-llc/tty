//! Encrypted, persisted command history — everything crypto- and
//! settings-specific stays here (host/UI concerns); the pure data types
//! (`HistoryEvent`, `PersistedCommandEntry`) live in `cathode::history`
//! instead, since cathode has no knowledge of crypto or the filesystem.
//!
//! Modules: [`crypto`] (dual-cipher AEAD wrap/unwrap), [`keychain`] (OS
//! keychain key management), [`passphrase`] (the Argon2id passphrase key
//! source), [`segment`] (one day's file), [`manifest`] (the date→segment
//! index), [`writer`] (the background writer thread), [`reauth`] (Touch
//! ID/password re-auth gating before the panel opens).

pub mod crypto;
pub mod keychain;
pub mod manifest;
pub mod passphrase;
pub mod reauth;
pub mod segment;
pub mod writer;

use chrono::TimeZone;

/// The local calendar date a timestamp falls on — not UTC's, so a command run
/// late at night files into the day the user actually experienced it as.
/// Shared by [`writer`] (deciding which day a `HistoryEvent` belongs to) and
/// the panel view (labeling a paged-in archive entry with its date).
pub fn local_date_from_epoch_ms(epoch_ms: u64) -> chrono::NaiveDate {
    chrono::Local
        .timestamp_millis_opt(epoch_ms as i64)
        .single()
        .unwrap_or_else(chrono::Local::now)
        .date_naive()
}

/// Errors from any part of the encrypted-history feature. Per the design's
/// failure-handling policy: every one of these means "refuse to load /
/// refuse to enable, warn, never crash, never silently produce garbage" —
/// callers are expected to log and degrade gracefully, not `unwrap`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unrecognized cipher id {0}")]
    UnknownCipher(u8),
    #[error("truncated or malformed data")]
    Truncated,
    /// Covers a tampered/corrupted file *and* a wrong key — deliberately not
    /// distinguished, so a caller can't be used as an oracle for which.
    #[error("authentication failed (corrupted, tampered, or wrong key)")]
    AuthFailed,
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("passphrase key derivation error: {0}")]
    Kdf(String),
    #[error("stored key has the wrong length ({0} bytes, expected 32)")]
    MalformedKey(usize),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Write `data` to `path` atomically: a temp file, then a rename over the
/// original. A crash mid-write leaves the original file exactly as it was —
/// readers never see a torn or partial file, only the last fully-written one.
/// Shared by [`segment`] and [`manifest`], both of which always rewrite their
/// whole (small) file rather than appending.
pub(crate) fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn tmp_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::path::PathBuf::from(tmp)
}

/// `~/.config/tty/history/` (or the platform equivalent) — a dedicated
/// subdirectory for day segments and the manifest, distinct from the config
/// file (`tty.toml`) in the directory itself.
pub fn history_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tty")
        .join("history")
}

/// The archive's key hierarchy: per-purpose keys fanned out of the master
/// via key-based derivation (`dorado_engine::kdf::derive_from_key_with`, a
/// domain-separated keyed hash). The PRF is the archive's fan-out choice
/// (`settings::HistoryFanout`, resolved against the cipher): Skein-512 or
/// BLAKE3, both secure PRFs, picked to match the cipher's family. The master
/// itself — keychain randomness or the passphrase KDF's output — is dropped
/// immediately after this fan-out, so the root secret never lingers for the
/// session; only the children do, and each encrypts exactly one kind of file.
/// Compartments: a bug in one context can't leak the other's key, and a
/// segment blob can never be accepted where the manifest belongs (different
/// key, so it fails authentication).
#[derive(Clone)]
pub struct HistoryKeys {
    /// Encrypts the manifest (the date→segment index).
    pub manifest: zeroize::Zeroizing<crypto::Key>,
    /// Encrypts the day segments (the command entries themselves).
    pub segments: zeroize::Zeroizing<crypto::Key>,
}

impl HistoryKeys {
    /// Fan `master` out into the per-purpose children under the given fan-out
    /// PRF (`settings::HistoryFanout::resolve`). The same master under a
    /// different PRF yields different children, so this is fixed per archive.
    pub fn from_master(master: &crypto::Key, prf: dorado_engine::kdf::KdfPrf) -> Self {
        let mut manifest = zeroize::Zeroizing::new([0u8; 32]);
        let mut segments = zeroize::Zeroizing::new([0u8; 32]);
        dorado_engine::kdf::derive_from_key_with(
            prf,
            master,
            "tty/history/manifest",
            manifest.as_mut(),
        );
        dorado_engine::kdf::derive_from_key_with(
            prf,
            master,
            "tty/history/segments",
            segments.as_mut(),
        );
        Self { manifest, segments }
    }
}

/// What [`start`] hands back to the host on success.
pub struct Started {
    /// The background writer thread — the sole writer of segment/manifest
    /// files for the rest of this process's life.
    pub writer: writer::Writer,
    /// Up to `cathode::screen::MAX_COMMAND_LOG` of the most recent persisted
    /// entries, for seeding the first tab's live view.
    pub seed: Vec<cathode::history::PersistedCommandEntry>,
    /// A second, independent copy of the cipher/keys, for the main thread's
    /// own *reads* (paging into older days). Deliberately separate from what
    /// the writer thread privately owns: paging is a low-frequency,
    /// main-thread-only operation, not worth a request/response channel to
    /// the writer for, and reading a day segment never needs to coordinate
    /// with writes to *other* days.
    pub cipher: crypto::Cipher,
    pub keys: HistoryKeys,
}

/// Bootstrap encrypted history with an already-resolved key: load the
/// manifest, collect the most recent entries (see [`Started::seed`]), and
/// spawn the background writer. Knows nothing about *where* the key came
/// from (OS keychain, a passphrase KDF) — that's the caller's business. The
/// error matters to callers: `Error::AuthFailed` on the manifest load is how
/// a passphrase caller learns the passphrase was wrong.
pub fn start_with_key(
    cipher: crypto::Cipher,
    key: zeroize::Zeroizing<crypto::Key>,
    prf: dorado_engine::kdf::KdfPrf,
) -> Result<Started> {
    start_with_key_in(history_dir(), cipher, key, prf)
}

/// [`start_with_key`] against an explicit directory — the testable core
/// (`history_dir()` is a fixed path, and tests must never touch the real
/// archive).
pub(crate) fn start_with_key_in(
    dir: std::path::PathBuf,
    cipher: crypto::Cipher,
    master: zeroize::Zeroizing<crypto::Key>,
    prf: dorado_engine::kdf::KdfPrf,
) -> Result<Started> {
    // Fan the master out into the per-purpose keys and let it drop (zeroized)
    // right here — nothing downstream ever needs the root secret again.
    let keys = HistoryKeys::from_master(&master, prf);
    drop(master);

    let manifest_path = dir.join(writer::MANIFEST_FILENAME);
    let manifest = manifest::Manifest::load(&manifest_path, &keys.manifest)?;

    let seed = collect_seed_entries(&dir, &manifest, &keys.segments);
    let writer = writer::Writer::spawn(dir, cipher, keys.clone(), manifest);
    Ok(Started {
        writer,
        seed,
        cipher,
        keys,
    })
}

/// Bootstrap encrypted history with the OS-keychain key: resolve (or mint)
/// the key, then [`start_with_key`]. `None` (feature stays off for this
/// session, warned) on any failure — a keychain problem or a corrupt
/// manifest — the same "refuse to load, warn, never crash" policy as
/// everything else here. Callers only call this when
/// `Settings::encrypted_history_enabled()` is true; there is no "disabled"
/// `Result` variant, just `None`.
pub fn start(cipher: crypto::Cipher, prf: dorado_engine::kdf::KdfPrf) -> Option<Started> {
    let key = match keychain::get_or_create_key() {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("encrypted history: keychain error, disabling for this session: {e}");
            return None;
        }
    };
    match start_with_key(cipher, key, prf) {
        Ok(started) => Some(started),
        Err(e) => {
            tracing::warn!("encrypted history: manifest error, disabling for this session: {e}");
            None
        }
    }
}

/// [`start`] on its own thread, bridged into a future — the keychain read
/// can block on an OS access dialog and must never run on the UI thread
/// (the same spawn-plus-oneshot pattern as [`reauth::authenticate`]). Lazy
/// like `reauth`: nothing happens until the future is polled, so a dropped
/// task never touches the keychain. The distinction between "wrong key" and
/// any other failure is meaningless for the keychain source (the keychain
/// key is never wrong, only missing or unreadable), so this collapses to
/// `Option` like the sync version.
pub async fn start_keychain_async(
    cipher: crypto::Cipher,
    prf: dorado_engine::kdf::KdfPrf,
) -> Option<Started> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(start(cipher, prf));
    });
    rx.await.unwrap_or(None)
}

/// Page one day older than `cursor` (`None` = start from the most recent
/// day), re-reading the manifest fresh from disk — paging is infrequent and
/// the manifest is small, so this is simpler than keeping a second in-memory
/// copy in sync with the writer thread's. Returns the new cursor date and
/// that day's entries, or `None` if there's nothing older (or the manifest
/// itself is unreadable, warned). No `cipher` parameter: [`crypto::unwrap`]
/// reads it from each file's own leading byte, so a reader never needs to be
/// told which cipher a file used.
pub fn page_older(
    keys: &HistoryKeys,
    cursor: Option<chrono::NaiveDate>,
) -> Option<(
    chrono::NaiveDate,
    Vec<cathode::history::PersistedCommandEntry>,
)> {
    let dir = history_dir();
    let manifest_path = dir.join(writer::MANIFEST_FILENAME);
    let manifest = match manifest::Manifest::load(&manifest_path, &keys.manifest) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("encrypted history: manifest error while paging: {e}");
            return None;
        }
    };
    let date = manifest.previous_date(cursor)?;
    let filename = manifest.segment_filename(date)?;
    match segment::load(&dir.join(filename), &keys.segments) {
        Ok(entries) => Some((date, entries)),
        Err(e) => {
            tracing::warn!("encrypted history: segment for {date} unreadable: {e}");
            None
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

/// Walk the manifest backward from the most recent day, decrypting segments
/// and accumulating their entries (oldest-collected-first overall) until
/// there are at least `cathode::screen::MAX_COMMAND_LOG` of them or the
/// archive runs out — so a large archive's startup cost is bounded by the
/// live window's own cap, not by total archive size. A single unreadable
/// segment is skipped and warned about, not fatal to the rest.
fn collect_seed_entries(
    dir: &std::path::Path,
    manifest: &manifest::Manifest,
    key: &crypto::Key,
) -> Vec<cathode::history::PersistedCommandEntry> {
    let mut collected: Vec<cathode::history::PersistedCommandEntry> = Vec::new();
    let mut cursor = None;
    while collected.len() < cathode::screen::MAX_COMMAND_LOG {
        let Some(date) = manifest.previous_date(cursor) else {
            break;
        };
        cursor = Some(date);
        let Some(filename) = manifest.segment_filename(date) else {
            continue;
        };
        match segment::load(&dir.join(filename), key) {
            Ok(entries) => {
                // Collected so far is newer days (walked first, since we go
                // backward from the most recent); this day is older, so it
                // goes in front to keep the whole list oldest-first.
                let mut day = entries;
                day.extend(collected);
                collected = day;
            }
            Err(e) => {
                tracing::warn!("encrypted history: segment for {date} unreadable, skipping: {e}");
            }
        }
    }
    collected
}
