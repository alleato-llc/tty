---
name: history
description: Use when working on tty's opt-in encrypted command history — the cathode event DTOs, the crypto / keychain / passphrase / segment / manifest / writer layers, the async startup path, the KDF sidecar, or untracked sessions. Carries the load-bearing invariants and routes to the ADRs. Apply whenever you touch `tty/src/history/`, `tty/src/state/encrypted_history.rs`, or `cathode/src/history.rs` — and read `docs/history-keys.md` before changing key handling.
---

# Encrypted command history

Opt-in persisted command history, **encrypted at rest**. Authoritative design:
`docs/adr/0006-encrypted-history.md` (+ `0007` key sources/async startup, `0008` untracked
sessions); the key-derivation pipeline is surveyed in `docs/history-keys.md` — **read it before
touching key handling, and update it when the design moves.** This skill is the working map +
the invariants you must not break.

## Architecture — the split stays split

- **`cathode::history`** (`cathode/src/history.rs`) — pure **DTOs + an event queue** on
  `TerminalScreen`. `PersistedCommandEntry`, `HistoryEvent::Upsert(..)`, and the single
  `queue_history_event` gate (in `screen.rs`). **No crypto, no filesystem, no settings.**
- **`tty/src/history/`** — everything stateful: `crypto`, `keychain`, `passphrase`, `segment`,
  `manifest`, `writer`, `reauth` (each with a sibling `*_tests.rs`), plus `mod.rs` (`HistoryKeys`,
  the async `start_*` fns).
- **`tty/src/state/encrypted_history.rs`** — the `Tty` methods: `startup_history_task`,
  `begin_history_start`, `apply_history_started`, `set_history_kdf`, reset/reauth.

Never move crypto/fs into cathode, and never add history logic to a drain path — it funnels
through `queue_history_event`.

## Invariants (do not break)

- **Command text only, never output.** Output routinely holds secrets; nothing in
  `PersistedCommandEntry` may ever grow an output field.
- **Never start history on the UI thread.** The keychain read can block on an OS dialog and the
  KDFs are deliberately slow. Every start goes through the async `begin_history_start` /
  `start_*_async` path (thread + oneshot, lazy), landing in `apply_history_started`. `Tty::new`
  stays crypto-free; `main` chains `startup_history_task()`.
- **The passphrase KDF sidecar is load-bearing plaintext.** A malformed `tty.history.kdf.json`
  (or unknown kdf tag) is an **error, never a silent re-mint** — a fresh salt locks the user out
  of an archive their passphrase still opens. The sidecar is *authoritative* for an existing
  archive; the `history_kdf` setting only picks the recipe for *new* ones.
- **Untracked = zero events at the source.** `TerminalScreen::untracked` queues no events at all
  (suppression lives in the one gate, never a drain). An untracked *session* is immutable until
  relaunch, does zero crypto, and must stay legible (○ marker, title suffix, status chip).
- **Eviction is not deletion.** Only explicit user Clear/Delete emits events; `MAX_COMMAND_LOG`
  eviction and RIS reset must never tombstone the archive.
- **One writer.** The background `Writer` thread (`Writer::spawn(dir, cipher, keys, manifest)`) is
  the sole writer of segment/manifest files; everything funnels through its channel. No second
  write path.
- **Fail closed, never silently-on.** Any start failure warns, disables for the session, reverts
  the setting. The off-toggle never deletes; only the dialog-confirmed Reset deletes.
- **Wrong password = the existing `AuthFailed`** — no verifier, no oracle; an empty archive
  accepts any passphrase by design.

## Keys + cipher (the dorado dependency)

The sibling `../dorado/rust/crates/dorado-engine` supplies the opt-in Threefish-256 cipher **and**
all key derivation (`dorado_engine::kdf`): `derive_from_password` + `validate` stretch the
passphrase; `derive_from_key_with` fans the master (keychain or passphrase) into the `HistoryKeys`
hierarchy under a family-matched PRF (`HistoryFanout`: BLAKE3 for ChaCha, Skein-512 for Threefish,
fixed at enable). The master never encrypts directly — manifest and each segment get their own
child key. tty owns only the sidecar/salt scope + the domains; it never re-implements the dispatch.
In UI text, "dorado" names the project; the cipher shows as "Threefish-256 (dorado)".

## Enable flow

All fixed-at-enable choices (key source, KDF, cipher, the passphrase) live in the **one** enable
dialog; the settings section greys them out until on. The dialog's keychain shape explains the OS
prompt before it appears. Wiring the settings controls is the `ui-settings` skill; the enable/reset
dialogs and their state machine live in `state/encrypted_history.rs`.

## Testing

- **Tests never touch the OS keychain or LocalAuthentication** — both side-effect the machine
  (a real keychain entry / a real auth dialog). The crypto/segment/manifest/writer/passphrase
  layers are tested against **temp dirs** (passphrase is the fully-testable source); keychain +
  the native prompt are manual-verification only.
- `reauth::authenticate` and the `start_*_async` fns are **lazy** (nothing fires until polled) —
  tests rely on that. `Settings::save` is a no-op under `cfg(test)`.
- `keyring` needs its platform features (`apple-native` / `windows-native` /
  `sync-secret-service`, set in the workspace `Cargo.toml`) — without them it silently compiles a
  non-persisting mock and every run mints a new key.
- Dev keychain gotcha: each ad-hoc `cargo build` is a new app to the keychain ACL, so reading a
  key from an older build can hit an allow/deny dialog. Reset dev state:
  `security delete-generic-password -s tty -a encrypted-history-key`.

For the History **UI** (the settings viewer, the archive browser, its date-flaky snapshots) see
the `snapshot-testing` skill (fixed-clock gotcha) and `ui-settings`.
