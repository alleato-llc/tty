use std::path::PathBuf;

use cathode::history::PersistedCommandEntry;

use super::*;

// These test `apply_and_save` directly (deterministic, synchronous) rather
// than the real background thread (fire-and-forget over a channel, with no
// acknowledgment to test against without a sleep-and-poll). The threaded
// wiring itself is exercised by the tty-level `behavior::*` integration test.

fn tmp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tty-writer-test-{}-{name}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    p
}

fn keys() -> HistoryKeys {
    HistoryKeys::from_master(&[0x33; 32], dorado_engine::kdf::KdfPrf::Skein512)
}

fn upsert(id: u32, command: &str, epoch_ms: u64) -> HistoryEvent {
    HistoryEvent::Upsert(PersistedCommandEntry {
        id,
        command: command.to_string(),
        started_at_epoch_ms: epoch_ms,
        pane_tag: "Tab 1".to_string(),
    })
}

fn tombstone(id: u32, epoch_ms: u64) -> HistoryEvent {
    HistoryEvent::Tombstone {
        id,
        started_at_epoch_ms: epoch_ms,
    }
}

#[test]
fn event_epoch_ms_reads_from_either_event_kind() {
    assert_eq!(
        event_epoch_ms(&upsert(1, "ls", 1_750_000_000_000)),
        1_750_000_000_000
    );
    assert_eq!(
        event_epoch_ms(&tombstone(1, 1_750_000_001_000)),
        1_750_000_001_000
    );
}

#[test]
fn local_date_from_epoch_ms_is_deterministic_for_a_fixed_timestamp() {
    // 2025-06-15 12:00:00 UTC — not asserting a specific local date (that
    // depends on the machine's timezone), just that it doesn't panic and is
    // internally consistent (the same input always gives the same output).
    let a = local_date_from_epoch_ms(1_750_000_800_000);
    let b = local_date_from_epoch_ms(1_750_000_800_000);
    assert_eq!(a, b);
}

#[test]
fn apply_and_save_writes_a_new_upsert_into_a_fresh_day_segment_and_updates_the_manifest() {
    let dir = tmp_dir("new-upsert");
    let mut manifest = Manifest::default();
    let manifest_path = dir.join(MANIFEST_FILENAME);
    let epoch_ms = 1_750_000_800_000; // fixed, so the date is deterministic
    let date = local_date_from_epoch_ms(epoch_ms);

    apply_and_save(
        &dir,
        &mut manifest,
        &manifest_path,
        Cipher::ChaCha20Poly1305,
        &keys(),
        upsert(1, "ls", epoch_ms),
    )
    .unwrap();

    let filename = manifest
        .segment_filename(date)
        .expect("date registered")
        .to_string();
    let entries = segment::load(&dir.join(&filename), &keys().segments).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].command, "ls");

    // The manifest itself round-trips from disk too, not just the in-memory copy.
    let reloaded = Manifest::load(&manifest_path, &keys().manifest).unwrap();
    assert_eq!(reloaded.segment_filename(date), Some(filename.as_str()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apply_and_save_tombstone_removes_the_date_from_the_manifest_when_it_empties_the_day() {
    let dir = tmp_dir("tombstone-empties-day");
    let mut manifest = Manifest::default();
    let manifest_path = dir.join(MANIFEST_FILENAME);
    let epoch_ms = 1_750_000_800_000;
    let date = local_date_from_epoch_ms(epoch_ms);

    apply_and_save(
        &dir,
        &mut manifest,
        &manifest_path,
        Cipher::ChaCha20Poly1305,
        &keys(),
        upsert(1, "ls", epoch_ms),
    )
    .unwrap();
    assert!(manifest.segment_filename(date).is_some());

    apply_and_save(
        &dir,
        &mut manifest,
        &manifest_path,
        Cipher::ChaCha20Poly1305,
        &keys(),
        tombstone(1, epoch_ms),
    )
    .unwrap();

    assert_eq!(
        manifest.segment_filename(date),
        None,
        "deleting the only entry for a day removes it from the manifest"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apply_and_save_files_different_timestamps_into_different_day_segments() {
    let dir = tmp_dir("multi-day");
    let mut manifest = Manifest::default();
    let manifest_path = dir.join(MANIFEST_FILENAME);

    // Two timestamps far enough apart to land on different local calendar
    // days regardless of timezone (30 days apart).
    let day1_ms = 1_750_000_800_000u64;
    let day2_ms = day1_ms + 30 * 24 * 60 * 60 * 1000;
    let date1 = local_date_from_epoch_ms(day1_ms);
    let date2 = local_date_from_epoch_ms(day2_ms);
    assert_ne!(
        date1, date2,
        "sanity: the two timestamps land on different days"
    );

    apply_and_save(
        &dir,
        &mut manifest,
        &manifest_path,
        Cipher::ChaCha20Poly1305,
        &keys(),
        upsert(1, "day one", day1_ms),
    )
    .unwrap();
    apply_and_save(
        &dir,
        &mut manifest,
        &manifest_path,
        Cipher::ChaCha20Poly1305,
        &keys(),
        upsert(2, "day two", day2_ms),
    )
    .unwrap();

    let f1 = manifest.segment_filename(date1).unwrap().to_string();
    let f2 = manifest.segment_filename(date2).unwrap().to_string();
    assert_ne!(f1, f2, "different days get different segment files");

    let entries1 = segment::load(&dir.join(&f1), &keys().segments).unwrap();
    let entries2 = segment::load(&dir.join(&f2), &keys().segments).unwrap();
    assert_eq!(entries1.len(), 1);
    assert_eq!(entries1[0].command, "day one");
    assert_eq!(entries2.len(), 1);
    assert_eq!(entries2[0].command, "day two");

    let _ = std::fs::remove_dir_all(&dir);
}
