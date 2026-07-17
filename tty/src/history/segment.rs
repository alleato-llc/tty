//! One day's persisted command history: load, apply an event, atomic-write.
//! Not an append-only log — a whole day is small (command-only entries), so
//! every mutation just re-encrypts the current, fully-live list. See
//! [`crate::history::crypto`] for the wrap/unwrap this builds on.

use std::path::Path;

use rand::RngCore;

use cathode::history::{HistoryEvent, PersistedCommandEntry};

use super::crypto::{self, Cipher, Key};
use super::{atomic_write, Result};

/// Load a day segment's entries, decrypting with `key`. A missing file
/// (nothing recorded yet for this day) is `Ok(empty vec)`, not an error.
pub fn load(path: &Path, key: &Key) -> Result<Vec<PersistedCommandEntry>> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let plaintext = crypto::unwrap(key, &data)?;
    Ok(serde_json::from_slice(&plaintext)?)
}

/// Apply one event to `entries` in place: `Upsert` pushes a new entry or
/// replaces an existing one by id in place (Clear supersedes this way, same
/// position); `Tombstone` removes by id (Delete).
pub fn apply(entries: &mut Vec<PersistedCommandEntry>, event: HistoryEvent) {
    match event {
        HistoryEvent::Upsert(entry) => {
            if let Some(existing) = entries.iter_mut().find(|e| e.id == entry.id) {
                *existing = entry;
            } else {
                entries.push(entry);
            }
        }
        HistoryEvent::Tombstone { id, .. } => {
            entries.retain(|e| e.id != id);
        }
    }
}

/// Encrypt `entries` with `cipher`/`key` and atomically write to `path` (temp
/// file + rename). A crash mid-write leaves the original file untouched —
/// `load` never sees a torn or partial file, only the last fully-written one.
pub fn save(
    path: &Path,
    cipher: Cipher,
    key: &Key,
    entries: &[PersistedCommandEntry],
) -> Result<()> {
    let plaintext = serde_json::to_vec(entries)?;
    let wrapped = crypto::wrap(cipher, key, &plaintext);
    atomic_write(path, &wrapped)
}

/// A random opaque filename for a new day segment, e.g. `f3a9c1e2b7d4.enc` —
/// doesn't reveal the date in cleartext; that association only ever exists
/// inside the encrypted manifest.
pub fn random_filename() -> String {
    let mut bytes = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.enc")
}

#[cfg(test)]
#[path = "segment_tests.rs"]
mod tests;
