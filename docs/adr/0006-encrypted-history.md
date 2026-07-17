# 0006 — Opt-in encrypted, persisted command history

Status: accepted (extended by ADR 0007 — passphrase key source, async
startup — and ADR 0008 — untracked sessions)

## Context

ADR 0004 built the Scrollback History panel over an in-memory `command_log` and
deliberately did **not** persist it — closing the app lost every recorded
command. The follow-up ask: keep command history **across launches**, without
turning the terminal into a plaintext keylogger of everything the user types.
That framing drove every choice below: persistence is **opt-in** (off by
default), **encrypted at rest**, and captures **command text only — never
output** (output routinely contains secrets: `cat ~/.aws/credentials`, printed
tokens, `env`).

## Decision

**Commands only, as pure DTOs in `cathode`, crypto in the app.**
`cathode::history` defines `PersistedCommandEntry { id, command,
started_at_epoch_ms, pane_tag }` and `HistoryEvent::{Upsert,
Tombstone{id, started_at_epoch_ms}}` — plain serde types, no crypto, no
filesystem, keeping cathode embeddable. `TerminalScreen` queues events as its
`command_log` mutates (`pending_history_events`, drained by the host every
tick alongside bell/clipboard); each entry carries a monotonic per-screen `id`
and a wall-clock timestamp (the existing `Instant` can't survive a restart).
A `Tombstone` carries its entry's timestamp too, because that's what locates
the day file it lives in — an id alone isn't addressable on disk.

**Eviction is not deletion.** Only an explicit user Clear/Delete emits a
blanking `Upsert` or a `Tombstone`. `MAX_COMMAND_LOG` window eviction and a
RIS terminal reset emit nothing — the archive is the longer-term record; the
live window shrinking must not erase it.

**One encrypted file per local calendar day, plus one encrypted manifest.**
Files live in the config dir under `tty/history/`. Day segments get opaque
random filenames (`<12 hex>.enc`) so filenames leak no dates; the manifest
(`tty.history.index.enc`, itself encrypted) maps local date → filename +
count. "Local" matters: a command run at 23:30 files into the day the user
experienced (`chrono::Local`), not UTC's. Every write is a full re-encrypt of
one small file via temp-file + `rename`, so a crash mid-write leaves the last
good version — verified by stray-`.tmp` crash-simulation tests.

**AEAD with a self-describing cipher byte, two ciphers.** Every blob is
`cipher_id ‖ nonce ‖ ciphertext+tag`. Cipher 0 (default): ChaCha20-Poly1305,
12-byte nonce — widely deployed and audited. Cipher 1 (explicit opt-in):
the sibling **dorado** project's raw-key authenticated construction —
Threefish-256 in CTR with a Skein-512 MAC, 32-byte IV — labeled honestly in
the UI as a sound design that is **not independently audited**. The choice is
fixed once the archive has data (no re-encryption machinery); the leading
byte exists so a corrupted file fails cleanly, not to mix ciphers. Decryption
never distinguishes wrong-key from tampering from corruption (`AuthFailed`) —
no oracle. This adds a second sibling path dependency
(`../dorado/rust/crates/dorado-engine`, alongside `../rime/rime`).

**A random 256-bit key from the OS keychain — no passphrase, no KDF.**
(*The default. ADR 0007 later added the promised second key source: a
passphrase stretched with Argon2id, for keychain-less platforms or by
preference.*) First
enable generates the key (`OsRng`) and stores it via `keyring`
(service `tty`, account `encrypted-history-key`, the raw-bytes
`get_secret`/`set_secret` API). The trust boundary is deliberate: whoever has
the user's unlocked OS session can read history, same as any app secret in
the keychain. Pitfall recorded so it is never reintroduced: `keyring` 3
**compiles a non-persisting mock backend unless a platform feature is
enabled** (`apple-native` / `windows-native` / `sync-secret-service`) —
without them every call minted a fresh key and nothing ever decrypted again.
Dev caveat: an ad-hoc `cargo build` re-signs the binary, so the keychain ACL
treats each rebuild as a new app reading the old build's item (dialog or
denial); the Developer-ID-signed `.app` has a stable identity.

**A single background writer thread owns the files.** Panes across all
tabs/windows funnel events over one `mpsc` channel; the writer applies each
to the right day segment (by the event's own timestamp, not "now") and
rewrites segment + manifest. Sole-writer means concurrent panes need no file
locking. Sends are best-effort — losing one history write is not worth
crashing the terminal. The main thread keeps its own copy of the cipher+key
for **reads only**: paging the panel back a day re-reads the manifest fresh
from disk (paging is rare and the manifest tiny — simpler than a
request/response channel to the writer).

**Startup seeds; the panel pages.** On launch (when enabled) the newest
`MAX_COMMAND_LOG` entries seed the first tab's live view via
`seed_command_log` (no events emitted, id counter advanced past loaded ids).
The panel's "Load older day" / "Back to today" buttons walk the manifest;
archived rows are addressed by stable `(date, id)` (`ArchivedTarget`, unified
with the live index-based `ScrollbackTarget` under `HistoryRowTarget`), and
their Clear/Delete go straight to the writer — there is no in-memory
`CommandEntry` behind a paged-in row. The settings History section **drills
into** a second archive browser (the "View archived commands…" button swaps
the whole section for a full-height list with a Back header — no cramped
inline table next to the config controls; same paging, its own cursor).
Right-click a row to **Copy** or **Delete…** it; double-click copies.
Copying a command strips its captured shell prompt
(`cathode::commands::strip_prompt`, best-effort) — the stored line is the
full echoed row, but what the user wants on the clipboard is what they
typed. Unlike
the panel's immediate per-row Delete, the browser's confirms first
(`rime::dialog`) — in the panel the row sits in context under ⌘⇧H, in
settings it's further from the live session, so the destructive action gets
one more gate. A confirmed delete tombstones through the same writer path and
drops the entry from **both** surfaces' paged-in copies. Both surfaces drop
their decrypted entries when closed.

**Failure policy: refuse, warn, never crash, never silently-on.** A keychain
error or unreadable manifest disables the feature for the session with a
`tracing` warning and a red banner in settings; toggling **on** that fails
reverts the setting to off rather than persisting "on but broken." Toggling
**off** only stops the writer — it never deletes the archive. Destruction is
a separate, explicit **Reset encrypted history** action behind a
confirmation dialog (`rime::dialog`): it removes the whole directory at once
(manifest and segments can never go out of sync with each other) and, if the
feature is on, immediately starts a fresh empty archive.

**Re-authentication gate (macOS).** Opening the Scrollback History panel
while an archive is active requires Touch ID or the device password via
LocalAuthentication (`LAPolicy::DeviceOwnerAuthentication`) — once per app
session always, plus an optional idle interval (settings stepper, 0 = off,
max 480 min). The pure policy (`reauth::is_due`) is unit-tested; the native
prompt runs on its own thread, bridged into an `iced::Task` lazily (nothing
fires until the task is polled — a dropped task must not flash a prompt).
Every surface that reads the archive routes through the gate — the ⌘⇧H
chord, the context-menu item (both through one gated helper, after a
regression where the chord bypassed it), and the settings archive browser.
Failure or cancel fails **closed**: the surface stays shut. Off macOS the
gate is a documented no-op, not an oversight; passive background *writes*
are never gated — the gate protects reading the archive.

## Consequences

- Two sibling repos are now required checkouts: `../rime` (as before) and
  `../dorado`. The dorado dependency is the engine crate only, and only for
  cipher 1; the default path never calls into it at runtime, but it always
  compiles.
- Settings gain `encrypted_history_enabled`, `history_key_source` (a field,
  not an inference, precisely so a passphrase mode needed no migration —
  ADR 0007 filled in its second value), `history_cipher`, and
  `history_reauth_interval_minutes`. The cipher's settings string
  (`"dorado"`) is an identifier; the UI displays "Threefish-256 (dorado)" —
  dorado is the project, Threefish is the cipher.
- The macOS dependency set moved: `objc2`/`objc2-app-kit`/`objc2-foundation`
  bumped to 0.6/0.3 (required by `objc2-local-authentication`), plus
  `block2` and `futures-channel`.
- Tests deliberately never touch the real OS keychain or LocalAuthentication
  (both would side-effect the machine running `cargo test`); their
  correctness is covered by crypto/unit tests plus manual verification on a
  real build. `history_dir()` is likewise a fixed path, so the reset action's
  deletion is exercised manually, not in tests.
- ADR 0004's "deliberately does not persist `command_log`" consequence is
  superseded — persistence now exists, but only as this opt-in.
