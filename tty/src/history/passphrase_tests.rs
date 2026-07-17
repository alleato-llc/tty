//! Pure passphrase-path tests: KDFs + sidecar + the end-to-end start core,
//! all against tempdirs. Nothing here touches the OS keychain (this path is
//! exactly the one that never needs it).

use dorado_engine::kdf::KdfPrf;

use crate::settings::HistoryKdf;

use super::super::crypto::Cipher;
use super::super::Error;
use super::{derive_key, load, load_or_create, start_in, KdfRecipe};

/// The fan-out PRF is orthogonal to these passphrase/sidecar tests; Skein-512
/// (the Auto default for a non-dorado cipher) keeps them focused on the KDF.
const PRF: KdfPrf = KdfPrf::Skein512;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tty-history-passphrase-{}-{name}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&p).expect("create temp history dir");
    p
}

#[test]
fn sidecar_roundtrips_and_is_stable_once_created() {
    let dir = tmp_dir("sidecar");
    assert!(load(&dir).unwrap().is_none(), "fresh dir has no sidecar");

    let first = load_or_create(&dir, HistoryKdf::Argon2id).unwrap();
    assert!(matches!(first.recipe, KdfRecipe::Argon2id { .. }));
    assert_eq!(first.salt.len(), 32, "16 random bytes, hex-encoded");

    // A second load reuses the salt — and the recorded recipe beats a
    // *different* settings choice: the sidecar is authoritative for an
    // existing archive, so flipping the KDF setting can't lock anyone out.
    let second = load_or_create(&dir, HistoryKdf::Scrypt).unwrap();
    assert_eq!(second.salt, first.salt, "never re-mint the salt");
    assert_eq!(
        second.recipe, first.recipe,
        "an existing sidecar's recipe wins over the setting"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_malformed_sidecar_is_an_error_not_a_fresh_start() {
    let dir = tmp_dir("malformed");
    std::fs::write(dir.join(super::SIDECAR_FILENAME), b"not json").unwrap();
    assert!(
        load(&dir).is_err(),
        "silently re-minting a salt would lock the user out of their archive"
    );

    // Same for a structurally-valid sidecar naming an unknown algorithm —
    // its parameters must not be misread as some other KDF's.
    std::fs::write(
        dir.join(super::SIDECAR_FILENAME),
        br#"{"version":1,"salt":"00112233445566778899aabbccddeeff","kdf":"bcrypt"}"#,
    )
    .unwrap();
    assert!(load(&dir).is_err(), "unknown kdf tag fails loudly");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn derive_key_is_deterministic_and_sensitive_to_passphrase_and_salt() {
    let dir = tmp_dir("derive");
    let sidecar = load_or_create(&dir, HistoryKdf::Argon2id).unwrap();

    let a1 = derive_key("correct horse battery", &sidecar).unwrap();
    let a2 = derive_key("correct horse battery", &sidecar).unwrap();
    assert_eq!(*a1, *a2, "same passphrase + salt => same key");

    let b = derive_key("correct horse battery staple", &sidecar).unwrap();
    assert_ne!(*a1, *b, "a different passphrase => a different key");

    let mut other = sidecar.clone();
    other.salt = other.salt.chars().rev().collect();
    let c = derive_key("correct horse battery", &other).unwrap();
    assert_ne!(*a1, *c, "a different salt => a different key");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every offered KDF round-trips the full path: enable a fresh archive,
/// refuse the wrong passphrase, reopen with the right one — and the sidecar
/// on disk records that algorithm's own parameters.
#[test]
fn every_kdf_choice_creates_refuses_and_reopens() {
    for (kdf, name) in [
        (HistoryKdf::Argon2id, "argon2id"),
        (HistoryKdf::Scrypt, "scrypt"),
        (HistoryKdf::Pbkdf2, "pbkdf2"),
    ] {
        let dir = tmp_dir(name);

        {
            let started = start_in(
                dir.clone(),
                Cipher::ChaCha20Poly1305,
                kdf,
                PRF,
                "passphrase-a",
            )
            .unwrap();
            started.writer.send(cathode::history::HistoryEvent::Upsert(
                cathode::history::PersistedCommandEntry {
                    id: 1,
                    command: "$ ls".to_string(),
                    started_at_epoch_ms: 1_700_000_000_000,
                    pane_tag: "Tab 1".to_string(),
                },
            ));
            let manifest_path = dir.join(super::super::writer::MANIFEST_FILENAME);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !manifest_path.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "[{name}] timed out waiting for the writer to persist"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        let sidecar = load(&dir).unwrap().expect("sidecar written");
        let matches_choice = matches!(
            (kdf, &sidecar.recipe),
            (HistoryKdf::Argon2id, KdfRecipe::Argon2id { .. })
                | (HistoryKdf::Scrypt, KdfRecipe::Scrypt { .. })
                | (HistoryKdf::Pbkdf2, KdfRecipe::Pbkdf2Sha256 { .. })
        );
        assert!(matches_choice, "[{name}] sidecar records the chosen KDF");

        let err = start_in(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            kdf,
            PRF,
            "passphrase-b",
        )
        .err()
        .expect("the wrong passphrase must not open the archive");
        assert!(
            matches!(err, Error::AuthFailed),
            "[{name}] wrong passphrase surfaces as AuthFailed, got {err}"
        );

        // Reopening passes a *different* KDF choice on purpose: the sidecar's
        // recorded recipe must win, so the original passphrase still works.
        let other_choice = match kdf {
            HistoryKdf::Argon2id => HistoryKdf::Scrypt,
            _ => HistoryKdf::Argon2id,
        };
        let reopened = start_in(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            other_choice,
            PRF,
            "passphrase-a",
        )
        .expect("the original passphrase still opens it, sidecar recipe winning");
        assert_eq!(reopened.seed.len(), 1);
        assert_eq!(reopened.seed[0].command, "$ ls");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// dorado-engine's `kdf::validate` bounds the sidecar's cost parameters —
/// the sidecar comes off disk, and a corrupted one must fail cleanly rather
/// than pin the machine allocating gigabytes or derive a garbage key.
#[test]
fn absurd_sidecar_costs_fail_closed() {
    let dir = tmp_dir("absurd");
    let mut sidecar = load_or_create(&dir, HistoryKdf::Argon2id).unwrap();

    sidecar.recipe = KdfRecipe::Argon2id {
        m_cost_kib: 1 << 30, // ~1 TiB of Argon2 memory
        t_cost: 2,
        p_cost: 1,
    };
    assert!(derive_key("whatever", &sidecar).is_err());

    sidecar.recipe = KdfRecipe::Pbkdf2Sha256 { iterations: 0 };
    assert!(
        derive_key("whatever", &sidecar).is_err(),
        "zero rounds would silently derive an all-zero key"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_archive_accepts_any_passphrase() {
    let dir = tmp_dir("empty");
    // No manifest on disk: there is nothing to authenticate against, so any
    // passphrase starts a fresh archive keyed to itself (documented in the
    // module docs — no verifier, no oracle).
    let started = start_in(
        dir.clone(),
        Cipher::ChaCha20Poly1305,
        HistoryKdf::Argon2id,
        PRF,
        "whatever",
    )
    .unwrap();
    assert!(started.seed.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
