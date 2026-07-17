//! The date→segment index: one small encrypted blob mapping every local
//! calendar date that has a day segment to that segment's opaque filename and
//! live record count. Decrypted once at startup; paging walks it
//! backward/forward without touching a segment file until the panel actually
//! lands on it, and the date/segment-id association never touches disk in
//! cleartext (only inside this encrypted blob).

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;

use chrono::NaiveDate;

use super::crypto::{self, Cipher, Key};
use super::{atomic_write, Result};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    days: BTreeMap<NaiveDate, DayEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct DayEntry {
    segment_filename: String,
    count: u32,
}

impl Manifest {
    /// Load the manifest, decrypting with `key`. A missing file (no history
    /// persisted yet) is `Ok(Manifest::default())`, not an error.
    pub fn load(path: &Path, key: &Key) -> Result<Self> {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let plaintext = crypto::unwrap(key, &data)?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    /// Encrypt and atomically write this manifest to `path`.
    pub fn save(&self, path: &Path, cipher: Cipher, key: &Key) -> Result<()> {
        let plaintext = serde_json::to_vec(self)?;
        let wrapped = crypto::wrap(cipher, key, &plaintext);
        atomic_write(path, &wrapped)
    }

    /// The segment filename for `date`, if any day segment exists for it yet.
    /// Read-only — use [`Self::segment_filename_or_create`] when about to
    /// write, which registers a fresh opaque filename on first use.
    pub fn segment_filename(&self, date: NaiveDate) -> Option<&str> {
        self.days.get(&date).map(|d| d.segment_filename.as_str())
    }

    /// The segment filename for `date`, creating and registering a fresh
    /// opaque one (via `new_filename`) if this is the first entry for that
    /// date. `new_filename` is injected rather than called internally so
    /// tests can supply a deterministic name instead of
    /// [`super::segment::random_filename`]'s real randomness.
    pub fn segment_filename_or_create(
        &mut self,
        date: NaiveDate,
        new_filename: impl FnOnce() -> String,
    ) -> String {
        self.days
            .entry(date)
            .or_insert_with(|| DayEntry {
                segment_filename: new_filename(),
                count: 0,
            })
            .segment_filename
            .clone()
    }

    /// Record `date`'s new live record count. A count of zero (every entry on
    /// that day has been deleted) drops the date from the manifest entirely —
    /// there is nothing left to page back to.
    pub fn set_count(&mut self, date: NaiveDate, count: u32) {
        if count == 0 {
            self.days.remove(&date);
        } else if let Some(entry) = self.days.get_mut(&date) {
            entry.count = count;
        }
        // A `set_count` for a date with no existing entry (count > 0 but no
        // segment registered yet) shouldn't happen in practice — the caller
        // always calls `segment_filename_or_create` first — so it's silently
        // ignored rather than panicking on a caller bug.
    }

    /// The most recent date with a segment, if any.
    pub fn latest_date(&self) -> Option<NaiveDate> {
        self.days.keys().next_back().copied()
    }

    /// The next *older* date with a segment, strictly before `before` (or the
    /// most recent date at all, if `before` is `None`) — walks the panel
    /// "up" into older history.
    pub fn previous_date(&self, before: Option<NaiveDate>) -> Option<NaiveDate> {
        match before {
            None => self.latest_date(),
            Some(d) => self
                .days
                .range((Bound::Unbounded, Bound::Excluded(d)))
                .next_back()
                .map(|(date, _)| *date),
        }
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
