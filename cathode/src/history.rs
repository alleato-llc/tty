//! Data types for persisted command history — pure data, no crypto, no I/O.
//! `tty` encrypts and writes these to disk; cathode only knows the shapes.
//! Deliberately narrower than [`crate::screen::CommandEntry`]: captured output
//! is never part of what persists, only the command text and metadata.

use std::time::SystemTime;

use crate::screen::CommandEntry;

/// One command as persisted to disk.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersistedCommandEntry {
    /// Unique within the file it's stored in (a single day's segment) — not a
    /// list position, so it stays stable across other entries being deleted.
    pub id: u32,
    pub command: String,
    pub started_at_epoch_ms: u64,
    /// Which pane ran this command, for context in the archive UI. Not part of
    /// this entry's identity (that's `id`), just a display label.
    pub pane_tag: String,
}

impl From<&CommandEntry> for PersistedCommandEntry {
    fn from(entry: &CommandEntry) -> Self {
        Self {
            id: entry.id,
            command: entry.command.clone(),
            started_at_epoch_ms: epoch_ms(entry.started_at_wall),
            pane_tag: entry.pane_tag.clone(),
        }
    }
}

/// A change to persist, queued by [`crate::screen::TerminalScreen`] and drained
/// by the host into its background writer. `Upsert` covers both a brand new
/// command and Clear (a superseding upsert with the command text blanked);
/// `Tombstone` is Delete, a full removal by id. Both carry a timestamp: the
/// host's writer files each day segment by wall-clock date, and cathode
/// deliberately has no notion of "days" itself (that's the host's concern,
/// including local-timezone handling) — so a `Tombstone` carries the
/// timestamp of the entry it's removing (copied from that entry before
/// removal) purely so the host can derive which day's segment to touch, the
/// same way it already would for the `Upsert` that originally created it.
#[derive(Debug, Clone)]
pub enum HistoryEvent {
    Upsert(PersistedCommandEntry),
    Tombstone { id: u32, started_at_epoch_ms: u64 },
}

/// Milliseconds since the Unix epoch, saturating to 0 on a clock before it
/// (never expected in practice, just avoids a panic).
pub(crate) fn epoch_ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
