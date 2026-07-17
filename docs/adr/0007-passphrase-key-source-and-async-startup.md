# 0007 — Passphrase key source + non-blocking history startup

Status: accepted (the full key pipeline, including the later key hierarchy
and the open refinement options, is surveyed in `../history-keys.md`)

## Context

ADR 0006's encrypted history had two rough edges. First, the key could only
come from the OS keychain — no path for platforms or runtimes without a
usable backend (Linux without a Secret Service daemon, minimal containers),
and no user choice. Second, `history::start()` ran synchronously on the UI
thread, both at boot (`Tty::new`) and on toggle-enable — and the keychain
read can block on a macOS ACL dialog, which froze the whole app behind a
prompt the user was never told to expect.

## Decision

**A second key source: a passphrase, stretched with a user-chosen KDF.**
`Settings::history_key_source` (a field reserved for exactly this since ADR
0006) gains `"passphrase"`. A settings select offers it before first enable;
like the cipher, it is fixed once the archive has data — switching means a
Reset. The KDF is itself a choice (`Settings::history_kdf`, mirroring the
cipher select): **Argon2id** (default — memory-hard, the Password Hashing
Competition winner and current best practice), **scrypt** (also memory-hard,
older), or **PBKDF2-HMAC-SHA256** (compute-hard only; offered for
environments that standardize on it). scrypt and PBKDF2 use OWASP's
recommended parameters; Argon2id runs above OWASP's server floor (64 MiB,
t=3 rather than 19 MiB, t=2), since a once-per-session local unlock can spend
the extra few tenths of a second and it costs a GPU/ASIC rig far more. The
full recipe — algorithm tag, random 16-byte salt, that
algorithm's own cost parameters — lives in a *plaintext*, self-describing
JSON sidecar next to the archive (`tty.history.kdf.json`,
`history/passphrase.rs`): none of it is secret, and it must be readable
before any key exists. **The sidecar is authoritative for an existing
archive** — the setting only picks the recipe for a *new* one, so changing
the setting later can never lock the user out. A malformed sidecar (or an
unknown algorithm tag) is an error, never a silent re-mint — a fresh salt
would permanently lock the user out of an archive their passphrase still
opens.

**Wrong passphrase = the existing manifest `AuthFailed`. No verifier.**
There is no "is this passphrase right?" check value: decrypting the manifest
already authenticates, and wrong-key stays deliberately indistinguishable
from tampering (no oracle, per ADR 0006). Consequence, documented honestly:
an *empty* archive accepts any passphrase and simply starts fresh keyed to
it. There is also no recovery — lose the passphrase and the archive is
unreadable; Reset is the only way back. `Zeroizing` wipes the prompt's
drafts on drop, best-effort only (iced's text-input keeps its own internal
copy — a limitation, not a promise we can make).

**All history starts are async, on one pattern.** The blocking work — the
keychain read, the Argon2id derivation — runs on a spawned thread bridged by
`futures_channel::oneshot` into an `iced::Task` (the exact pattern
`reauth::authenticate` proved), and is lazy: a dropped task touches nothing.
`Tty::new` no longer starts history at all; `main` chains
`startup_history_task()` next to opening the window, the toggle and Reset
paths go through the same `begin_history_start`, and the result lands in
`apply_history_started` as `HistoryStarted(origin, outcome)`. The origin
preserves the long-standing failure semantics: an *enable* (or post-Reset)
failure reverts the setting — never "on but broken" — while a *startup*
failure keeps it and shows the red banner. The setting itself commits only
on a successful enable.

**One enable dialog; explain prompts before they happen.** Flipping the
toggle on opens a single "Enable encrypted history" dialog that carries
every fixed-at-enable choice — the key source select (switching it live
reshapes the dialog), the KDF + passphrase fields when Passphrase is picked,
the OS-keychain explainer ("your OS may now ask you to allow tty to access
the keychain; that prompt comes from the OS") when Keychain is, and the
cipher. The settings section shows those choices *greyed out* until the
feature is on — inert muted rows pointing at the dialog — instead of live
pickers that invited configuration in one place and enabling in another
(a real user couldn't find where the passphrase went). The status bar
narrates `unlocking history key…` while a start is in flight. The passphrase source boots *locked*: no crypto
at all until the user enters the passphrase in the unlock modal (dismissible
— dismissed means locked for the session, plainly labeled "not recording" in
the status bar and settings, reopenable via Unlock…). Commands typed before
the archive opens are not retro-recorded, and the seed is skipped if the
live log already has entries (appending yesterday's history after today's
would scramble ordering).

**Deferred starts need an id floor.** Day segments upsert by `id`, and a
fresh screen counts from 0 — so commands typed before a deferred/locked
start would collide with (and overwrite) ids already archived earlier today.
`TerminalScreen::reserve_command_ids` (new, in cathode) raises the counter;
`apply_history_started` applies `max(seed id) + 1` to every live screen
(detached windows included) and `spawn_term` applies it to every screen
created afterwards.

## Consequences

- All key derivation is `dorado_engine::kdf`, in its two standard forms —
  dorado-engine exported them (previously private) for exactly this embedder
  shape, so tty adds no KDF crates at all. `derive_from_password` (the slow
  PBKDF: Argon2id/scrypt/PBKDF2 plus `validate`, whose cost bounds tty gains
  for free — a corrupted sidecar requesting gigabytes of Argon2 memory fails
  validation instead of pinning the machine) stretches the passphrase.
  `derive_from_key_with` (the fast KBKDF, a domain-separated keyed hash) then
  fans the master — from either source, keychain or passphrase — into a **key
  hierarchy** (`HistoryKeys`): `tty/history/manifest` and `tty/history/segments`
  children, so the master never encrypts anything directly (and is dropped
  right after the fan-out), each file kind lives in its own compartment, and a
  segment blob can never be accepted where the manifest belongs. The fan-out
  PRF matches the cipher's family (`settings::HistoryFanout`, default `Auto`):
  BLAKE3 for ChaCha20-Poly1305, Skein-512 for the Threefish cipher, so each
  configuration is single-family and Skein no longer sits in the ChaCha key
  path. Both are secure PRFs and yield equally strong children, so the choice
  is exposed as an explicit override in the enable dialog but defaults to the
  family match; like the cipher, it is fixed once the archive has data (the
  same master under a different PRF derives different children, so a later
  change means a Reset). The split of responsibilities with dorado's container:
  the container stores a fresh salt per file (each independently
  decryptable, KDF per read — right for standalone files); tty stores one
  salt per archive in the sidecar, derives once per session, and runs fast
  raw AEAD per file. Same crates, same construction — the boundary is *salt
  scope*, not capability. One new direct dep: `futures-channel`, promoted
  out of the macOS-only block (already in the tree via iced).
- `Message` gains a take-once `StartedHandle` wrapper because
  `history::Started` (thread handle + key) is neither `Clone` nor `Debug`.
- The keychain failure banner now points at the passphrase source as the
  fallback for keychain-less platforms.
- `Settings::save` is a no-op under `cfg(test)`: behavior tests drive real
  `update()` paths that save, and a test run must never rewrite the settings
  of whoever ran it.
- The passphrase path is the *testable* key source (pure KDF + tempdirs);
  the keychain stays manual-verification territory per ADR 0006.
