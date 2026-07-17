use std::path::PathBuf;

use super::super::tmp_path;
use super::*;

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tty-segment-test-{}-{name}-{}.enc",
        std::process::id(),
        rand::random::<u32>()
    ));
    p
}

fn key() -> Key {
    [0x11; 32]
}

fn entry(id: u32, command: &str) -> PersistedCommandEntry {
    PersistedCommandEntry {
        id,
        command: command.to_string(),
        started_at_epoch_ms: 1_750_000_000_000,
        pane_tag: "Tab 1".to_string(),
    }
}

#[test]
fn load_missing_file_is_empty_not_an_error() {
    let path = tmp("missing");
    let entries = load(&path, &key()).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn apply_upsert_pushes_a_new_entry() {
    let mut entries = vec![];
    apply(&mut entries, HistoryEvent::Upsert(entry(1, "ls")));
    assert_eq!(entries, vec![entry(1, "ls")]);
}

#[test]
fn apply_upsert_supersedes_an_existing_entry_by_id_in_place() {
    let mut entries = vec![entry(1, "ls"), entry(2, "pwd")];
    apply(&mut entries, HistoryEvent::Upsert(entry(1, "")));
    assert_eq!(
        entries,
        vec![entry(1, ""), entry(2, "pwd")],
        "same position, blanked"
    );
}

fn tombstone(id: u32) -> HistoryEvent {
    HistoryEvent::Tombstone {
        id,
        started_at_epoch_ms: 1_750_000_000_000,
    }
}

#[test]
fn apply_tombstone_removes_by_id() {
    let mut entries = vec![entry(1, "ls"), entry(2, "pwd")];
    apply(&mut entries, tombstone(1));
    assert_eq!(entries, vec![entry(2, "pwd")]);
}

#[test]
fn apply_tombstone_for_an_unknown_id_is_a_no_op() {
    let mut entries = vec![entry(1, "ls")];
    apply(&mut entries, tombstone(99));
    assert_eq!(entries, vec![entry(1, "ls")]);
}

#[test]
fn save_then_load_round_trips_for_every_cipher() {
    for cipher in [Cipher::ChaCha20Poly1305, Cipher::DoradoRawAuthenticated] {
        let path = tmp(&format!("roundtrip-{cipher:?}"));
        let entries = vec![entry(1, "ls"), entry(2, "pwd")];
        save(&path, cipher, &key(), &entries).unwrap();
        let back = load(&path, &key()).unwrap();
        assert_eq!(back, entries, "{cipher:?}");
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn load_rejects_a_corrupted_file_instead_of_panicking() {
    let path = tmp("corrupted");
    save(&path, Cipher::ChaCha20Poly1305, &key(), &[entry(1, "ls")]).unwrap();
    let mut data = std::fs::read(&path).unwrap();
    *data.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &data).unwrap();

    assert!(load(&path, &key()).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_stray_leftover_tmp_file_does_not_affect_loading_the_real_one() {
    // Simulates a crash mid-write: a `.tmp` file exists (from an interrupted
    // save that never got to the rename step), but the real file is whatever
    // it last successfully was. `load` must be unaffected by the stray file.
    let path = tmp("crash-mid-write");
    save(&path, Cipher::ChaCha20Poly1305, &key(), &[entry(1, "ls")]).unwrap();

    let stray_tmp = tmp_path(&path);
    std::fs::write(&stray_tmp, b"garbage, not even a valid wrapped blob").unwrap();

    let back = load(&path, &key()).unwrap();
    assert_eq!(
        back,
        vec![entry(1, "ls")],
        "unaffected by the stray .tmp file"
    );

    // A subsequent save still succeeds and cleanly overwrites the stray tmp.
    save(
        &path,
        Cipher::ChaCha20Poly1305,
        &key(),
        &[entry(1, "ls"), entry(2, "pwd")],
    )
    .unwrap();
    let back = load(&path, &key()).unwrap();
    assert_eq!(back, vec![entry(1, "ls"), entry(2, "pwd")]);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&stray_tmp);
}

#[test]
fn random_filename_looks_like_an_opaque_enc_file() {
    let a = random_filename();
    let b = random_filename();
    assert!(a.ends_with(".enc"));
    assert_ne!(a, b, "two calls should not collide in practice");
    assert!(!a.contains('-'), "no date-shaped structure leaking through");
}
