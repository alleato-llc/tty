use super::*;

const CIPHERS: [Cipher; 2] = [Cipher::ChaCha20Poly1305, Cipher::DoradoRawAuthenticated];

fn key(byte: u8) -> Key {
    [byte; 32]
}

#[test]
fn round_trips_for_every_cipher() {
    for cipher in CIPHERS {
        let k = key(0x11);
        let plaintext = b"a command line worth persisting";
        let wrapped = wrap(cipher, &k, plaintext);
        let back = unwrap(&k, &wrapped).unwrap_or_else(|e| panic!("{cipher:?}: {e}"));
        assert_eq!(back, plaintext, "{cipher:?} round-trip");
    }
}

#[test]
fn wrapped_output_carries_the_right_cipher_id() {
    let k = key(0x11);
    let cc = wrap(Cipher::ChaCha20Poly1305, &k, b"x");
    assert_eq!(cc[0], 0);
    let dorado = wrap(Cipher::DoradoRawAuthenticated, &k, b"x");
    assert_eq!(dorado[0], 1);
}

#[test]
fn rejects_tampering_for_every_cipher() {
    for cipher in CIPHERS {
        let k = key(0x22);
        let mut wrapped = wrap(cipher, &k, b"a command line worth persisting");
        *wrapped.last_mut().unwrap() ^= 1;
        assert!(
            matches!(unwrap(&k, &wrapped), Err(Error::AuthFailed)),
            "{cipher:?} must reject a tampered byte"
        );
    }
}

#[test]
fn rejects_wrong_key_for_every_cipher() {
    for cipher in CIPHERS {
        let wrapped = wrap(cipher, &key(0x33), b"a command line worth persisting");
        assert!(
            matches!(unwrap(&key(0x44), &wrapped), Err(Error::AuthFailed)),
            "{cipher:?} must reject the wrong key"
        );
    }
}

#[test]
fn rejects_an_unrecognized_cipher_id() {
    let mut wrapped = wrap(Cipher::ChaCha20Poly1305, &key(0x55), b"x");
    wrapped[0] = 0xff;
    assert!(matches!(
        unwrap(&key(0x55), &wrapped),
        Err(Error::UnknownCipher(0xff))
    ));
}

#[test]
fn rejects_truncated_data() {
    assert!(matches!(unwrap(&key(0x66), &[]), Err(Error::Truncated)));
    // A cipher id with no nonce/ciphertext following it.
    assert!(matches!(unwrap(&key(0x66), &[0]), Err(Error::Truncated)));
    // A full ChaCha20-Poly1305 nonce but nothing else.
    let short = vec![0u8; 1 + 12];
    assert!(matches!(
        unwrap(&key(0x66), &short),
        Err(Error::AuthFailed | Error::Truncated)
    ));
}

#[test]
fn empty_plaintext_round_trips() {
    for cipher in CIPHERS {
        let k = key(0x77);
        let wrapped = wrap(cipher, &k, b"");
        let back = unwrap(&k, &wrapped).unwrap();
        assert!(back.is_empty(), "{cipher:?} empty round-trip");
    }
}

#[test]
fn setting_str_round_trips_for_every_cipher() {
    for cipher in CIPHERS {
        assert_eq!(
            Cipher::from_setting_str(Some(cipher.as_setting_str())),
            cipher
        );
    }
}

#[test]
fn setting_str_falls_back_to_chacha20poly1305_for_absent_or_unknown() {
    assert_eq!(Cipher::from_setting_str(None), Cipher::ChaCha20Poly1305);
    assert_eq!(
        Cipher::from_setting_str(Some("something-malformed")),
        Cipher::ChaCha20Poly1305
    );
}
