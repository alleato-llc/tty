//! Tests for the key-agnostic bootstrap core (`start_with_key_in`). These
//! never call `start()`/`start_keychain_async` — those read the real OS
//! keychain, which a test run must not touch (its correctness is covered by
//! `crypto`'s tests plus manual verification on a real build).

use zeroize::Zeroizing;

use dorado_engine::kdf::KdfPrf;

use super::crypto::{Cipher, Key};
use super::writer::MANIFEST_FILENAME;
use super::{start_with_key_in, Error, HistoryKeys};

/// The default fan-out PRF (Skein-512) — these tests exercise the bootstrap
/// core, not the PRF choice; `history_keys_fanout_prf_changes_children` covers
/// that the choice matters.
const PRF: KdfPrf = KdfPrf::Skein512;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tty-history-start-{}-{name}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&p).expect("create temp history dir");
    p
}

fn key(byte: u8) -> Zeroizing<Key> {
    Zeroizing::new([byte; 32])
}

/// The key hierarchy's compartment property: the master fans out into
/// distinct, deterministic children, and material encrypted under one child
/// is rejected under the other — so a segment blob can never be accepted
/// where the manifest belongs (the anti-file-swap guarantee).
#[test]
fn history_keys_children_are_distinct_and_compartmented() {
    let a = HistoryKeys::from_master(&[0x11; 32], PRF);
    let b = HistoryKeys::from_master(&[0x11; 32], PRF);
    assert_eq!(*a.manifest, *b.manifest, "deterministic");
    assert_eq!(*a.segments, *b.segments, "deterministic");
    assert_ne!(*a.manifest, *a.segments, "children differ from each other");
    assert_ne!(*a.manifest, [0x11; 32], "children differ from the master");

    let dir = tmp_dir("compartments");
    super::manifest::Manifest::default()
        .save(
            &dir.join(MANIFEST_FILENAME),
            Cipher::ChaCha20Poly1305,
            &a.segments, // wrong compartment on purpose
        )
        .unwrap();
    let err = super::manifest::Manifest::load(&dir.join(MANIFEST_FILENAME), &a.manifest)
        .expect_err("a blob from the segments compartment must not open as the manifest");
    assert!(matches!(err, Error::AuthFailed));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fan-out PRF is part of the archive's identity: the same master under
/// Skein-512 and under BLAKE3 derives different children, which is exactly why
/// changing the setting on an existing archive means a Reset (its keys no
/// longer match). Both are deterministic on their own.
#[test]
fn history_keys_fanout_prf_changes_children() {
    let skein = HistoryKeys::from_master(&[0x5A; 32], KdfPrf::Skein512);
    let blake = HistoryKeys::from_master(&[0x5A; 32], KdfPrf::Blake3);
    assert_ne!(
        *skein.manifest, *blake.manifest,
        "different PRF must give a different manifest key"
    );
    assert_ne!(
        *skein.segments, *blake.segments,
        "different PRF must give a different segments key"
    );
    // Each PRF is still deterministic for the same master.
    assert_eq!(
        *blake.manifest,
        *HistoryKeys::from_master(&[0x5A; 32], KdfPrf::Blake3).manifest
    );
}

#[test]
fn an_empty_archive_dir_starts_fresh_with_no_seed() {
    let dir = tmp_dir("fresh");
    let started = start_with_key_in(dir.clone(), Cipher::ChaCha20Poly1305, key(0x11), PRF)
        .expect("a missing manifest means a brand-new archive, not an error");
    assert!(started.seed.is_empty());
    drop(started);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_written_with_one_key_refuses_another() {
    let dir = tmp_dir("wrong-key");

    // Create an archive with key A (spawning the writer persists the
    // manifest once it applies an event; simpler: save a manifest directly).
    super::manifest::Manifest::default()
        .save(
            &dir.join(MANIFEST_FILENAME),
            Cipher::ChaCha20Poly1305,
            &HistoryKeys::from_master(&[0xAA; 32], PRF).manifest,
        )
        .unwrap();

    let err = start_with_key_in(dir.clone(), Cipher::ChaCha20Poly1305, key(0xBB), PRF)
        .err()
        .expect("key B must not open key A's archive");
    assert!(
        matches!(err, Error::AuthFailed),
        "wrong key must surface as AuthFailed (the wrong-passphrase signal), got: {err}"
    );

    // And the right key still works.
    let started = start_with_key_in(dir.clone(), Cipher::ChaCha20Poly1305, key(0xAA), PRF)
        .expect("the original key still opens it");
    drop(started);
    let _ = std::fs::remove_dir_all(&dir);
}
