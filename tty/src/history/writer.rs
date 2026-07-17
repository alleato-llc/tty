//! The background writer thread: the sole writer of segment/manifest files,
//! which is what makes concurrent panes safe without file locking. Receives
//! [`HistoryEvent`]s over a channel and applies each to the correct day's
//! segment (by local wall-clock date, derived from the event's own
//! timestamp — not "now", so a backlogged event still files into the day it
//! actually happened on).

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

use cathode::history::HistoryEvent;

use super::crypto::Cipher;
use super::manifest::Manifest;
use super::{local_date_from_epoch_ms, segment, HistoryKeys, Result};

pub(crate) const MANIFEST_FILENAME: &str = "tty.history.index.enc";

/// A handle to the running writer thread. Dropping it closes the channel,
/// which ends the thread's loop and joins it.
pub struct Writer {
    tx: mpsc::Sender<HistoryEvent>,
    _handle: JoinHandle<()>,
}

impl Writer {
    /// Start the background thread, taking ownership of the already-loaded
    /// `manifest` (see `manifest::Manifest::load`, called by the host before
    /// spawning this). `cipher`/`keys` are resolved once, from
    /// `Settings::history_cipher` and the master key's fan-out (see
    /// [`HistoryKeys`]), and used for every write for the life of this
    /// thread — the manifest key for the index, the segments key for the
    /// day files.
    pub fn spawn(dir: PathBuf, cipher: Cipher, keys: HistoryKeys, manifest: Manifest) -> Self {
        let (tx, rx) = mpsc::channel::<HistoryEvent>();
        let handle = std::thread::Builder::new()
            .name("tty-history-writer".to_string())
            .spawn(move || run(dir, cipher, keys, manifest, rx))
            .expect("spawning the history writer thread");
        Writer {
            tx,
            _handle: handle,
        }
    }

    /// Queue an event for the writer. Best-effort: if the thread has somehow
    /// died, the event is silently dropped rather than panicking the caller —
    /// losing one history write is not worth crashing the terminal over.
    pub fn send(&self, event: HistoryEvent) {
        let _ = self.tx.send(event);
    }
}

fn run(
    dir: PathBuf,
    cipher: Cipher,
    keys: HistoryKeys,
    mut manifest: Manifest,
    rx: mpsc::Receiver<HistoryEvent>,
) {
    let manifest_path = dir.join(MANIFEST_FILENAME);
    while let Ok(event) = rx.recv() {
        if let Err(e) = apply_and_save(&dir, &mut manifest, &manifest_path, cipher, &keys, event) {
            tracing::warn!("history writer: {e}");
        }
    }
}

fn apply_and_save(
    dir: &std::path::Path,
    manifest: &mut Manifest,
    manifest_path: &std::path::Path,
    cipher: Cipher,
    keys: &HistoryKeys,
    event: HistoryEvent,
) -> Result<()> {
    let date = local_date_from_epoch_ms(event_epoch_ms(&event));
    let filename = manifest.segment_filename_or_create(date, segment::random_filename);
    let segment_path = dir.join(&filename);

    let mut entries = segment::load(&segment_path, &keys.segments)?;
    segment::apply(&mut entries, event);
    segment::save(&segment_path, cipher, &keys.segments, &entries)?;

    manifest.set_count(date, entries.len() as u32);
    manifest.save(manifest_path, cipher, &keys.manifest)?;
    Ok(())
}

fn event_epoch_ms(event: &HistoryEvent) -> u64 {
    match event {
        HistoryEvent::Upsert(entry) => entry.started_at_epoch_ms,
        HistoryEvent::Tombstone {
            started_at_epoch_ms,
            ..
        } => *started_at_epoch_ms,
    }
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
