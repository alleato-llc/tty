//! `impl Tty` methods for the opt-in encrypted command history: enabling /
//! disabling, the key-source / KDF / fan-out / cipher config, the passphrase
//! prompt + unlock, the async start (begin / apply), re-auth gating, the reset
//! flow, per-launch startup, and the untracked-session choice. Split out of
//! `state.rs`; these are just more methods on the same `Tty`.

use std::time::Instant;

use super::{PassphrasePrompt, PassphrasePromptKind, Tty};
use crate::history;
use crate::message::Message;

impl Tty {
    /// The settings toggle, ON direction: open the one enable dialog. It
    /// carries every fixed-at-enable choice (key source, KDF, cipher) plus
    /// the passphrase fields or the OS-keychain explainer, depending on the
    /// source picked *in the dialog* — nothing touches the keychain or
    /// derives anything until the user confirms there, and the setting
    /// itself commits only when the async start succeeds. A no-op while a
    /// start is already in flight or the feature is already on.
    pub fn request_enable_encrypted_history(&mut self) {
        if self.history_starting {
            tracing::info!("encrypted history: enable ignored — a start is already in flight");
            return;
        }
        if self.settings.encrypted_history_enabled() {
            tracing::info!("encrypted history: enable ignored — already enabled");
            return;
        }
        // An untracked session keeps its promise: the setting persists (for
        // the next launch), but nothing starts recording *this* session —
        // the History section says so.
        if self.session_untracked {
            tracing::info!("encrypted history: enable persisted; session untracked until relaunch");
            self.settings.encrypted_history_enabled = Some(true);
            self.settings.save();
            return;
        }
        tracing::info!("encrypted history: enable requested — opening the enable dialog");
        self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Enable));
    }

    /// The settings toggle, OFF direction: stop the writer for this session.
    /// Never deletes the archive (that's the separate, confirmed Reset).
    pub fn disable_encrypted_history(&mut self) {
        self.history_writer = None;
        self.history_read = None;
        self.settings.encrypted_history_enabled = Some(false);
        self.history_start_failed = false;
        self.history_locked = false;
        self.passphrase_prompt = None;
        // Fail toward requiring a fresh check rather than trusting a stale
        // one — a later re-enable gets a new archive underneath it.
        self.last_history_auth = None;
        self.settings.save();
    }

    /// Pick the history key source (persisted). Like the cipher, only takes
    /// effect the next time the feature starts fresh.
    pub fn set_history_key_source(&mut self, source: String) {
        self.settings.history_key_source = Some(source);
        self.settings.save();
    }

    /// Pick the launch behavior (persisted; takes effect next launch —
    /// including this-session-untracked, which stays untracked either way).
    pub fn set_history_session_start(&mut self, mode: String) {
        self.settings.history_session_start = Some(mode);
        self.settings.save();
    }

    /// Pick the passphrase KDF (persisted). New archives only — an existing
    /// archive keeps its sidecar's recorded recipe.
    pub fn set_history_kdf(&mut self, kdf: String) {
        self.settings.history_kdf = Some(kdf);
        self.settings.save();
    }

    /// Pick the fan-out PRF (persisted). Fixed at enable like the cipher; an
    /// existing archive must decrypt under the same choice or a Reset is
    /// required.
    pub fn set_history_fanout(&mut self, fanout: String) {
        self.settings.history_fanout = Some(fanout);
        self.settings.save();
    }

    pub fn open_history_unlock(&mut self) {
        if self.history_locked && !self.history_starting && self.passphrase_prompt.is_none() {
            self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));
        }
    }

    /// The passphrase prompt's main field changed.
    pub fn set_passphrase_draft(&mut self, text: String) {
        if let Some(prompt) = self.passphrase_prompt.as_mut() {
            if !prompt.busy {
                *prompt.draft = text;
            }
        }
    }

    /// The passphrase prompt's confirm field changed (enable flow).
    pub fn set_passphrase_confirm(&mut self, text: String) {
        if let Some(prompt) = self.passphrase_prompt.as_mut() {
            if !prompt.busy {
                *prompt.confirm = text;
            }
        }
    }

    /// Dismiss the passphrase prompt (its `Zeroizing` drafts wipe on drop).
    /// Enable flow: the setting stays off. Unlock flow: history stays locked
    /// for the session; the banner's "Unlock…" reopens it.
    pub fn cancel_passphrase_prompt(&mut self) {
        self.passphrase_prompt = None;
    }

    /// Submit the passphrase prompt: validate inline (length; the enable
    /// flow's two entries matching), then derive the key + start on a
    /// background thread — Argon2id is deliberately slow and must not run on
    /// the UI thread. The result lands in [`Self::apply_history_started`]
    /// with `WrongPassphrase` mapped from `Error::AuthFailed`.
    pub fn submit_passphrase(&mut self) -> iced::Task<Message> {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};

        let Some(prompt) = self.passphrase_prompt.as_mut() else {
            return iced::Task::none();
        };
        if prompt.busy {
            return iced::Task::none();
        }
        if prompt.draft.chars().count() < history::passphrase::MIN_PASSPHRASE_LEN {
            prompt.error = Some(format!(
                "At least {} characters.",
                history::passphrase::MIN_PASSPHRASE_LEN
            ));
            return iced::Task::none();
        }
        if prompt.kind == PassphrasePromptKind::Enable && *prompt.draft != *prompt.confirm {
            prompt.error = Some("The two entries don't match.".to_string());
            return iced::Task::none();
        }
        prompt.error = None;
        prompt.busy = true;
        let origin = match prompt.kind {
            PassphrasePromptKind::Enable => HistoryStartOrigin::Enable,
            PassphrasePromptKind::Unlock => HistoryStartOrigin::Unlock,
        };
        let passphrase = prompt.draft.clone();
        self.history_starting = true;
        let cipher = self.settings.history_cipher();
        let kdf = self.settings.history_kdf();
        let prf = self.settings.history_fanout().resolve(cipher);
        iced::Task::perform(
            history::passphrase::start_async(cipher, kdf, prf, passphrase),
            move |result| {
                let outcome = match result {
                    Ok(started) => HistoryStartOutcome::Ready(StartedHandle::new(started)),
                    Err(history::Error::AuthFailed) => HistoryStartOutcome::WrongPassphrase,
                    Err(e) => {
                        tracing::warn!("encrypted history: passphrase start failed: {e}");
                        HistoryStartOutcome::Failed
                    }
                };
                Message::HistoryStarted(origin, outcome)
            },
        )
    }

    /// Kick off an async history start (the keychain read runs on its own
    /// thread — see `history::start_keychain_async`). The result comes back
    /// as `Message::HistoryStarted` and lands in
    /// [`Self::apply_history_started`].
    pub fn begin_history_start(
        &mut self,
        origin: crate::message::HistoryStartOrigin,
    ) -> iced::Task<Message> {
        use crate::message::{HistoryStartOutcome, StartedHandle};
        self.history_starting = true;
        let cipher = self.settings.history_cipher();
        let prf = self.settings.history_fanout().resolve(cipher);
        iced::Task::perform(history::start_keychain_async(cipher, prf), move |result| {
            let outcome = match result {
                Some(started) => HistoryStartOutcome::Ready(StartedHandle::new(started)),
                None => HistoryStartOutcome::Failed,
            };
            Message::HistoryStarted(origin, outcome)
        })
    }

    /// Apply a finished async history start. Success installs the writer +
    /// read key, raises the command-id floor on every live screen (ids below
    /// it belong to entries already archived today — see
    /// `TerminalScreen::reserve_command_ids`), and seeds the active pane's
    /// live log *only if it's still empty*: commands typed before the
    /// archive opened are not retro-recorded, and appending yesterday's
    /// entries after today's would scramble the panel's ordering. Failure
    /// keeps the honest long-standing semantics: an enable failure reverts
    /// the setting to off (never "on but broken"); a startup or post-Reset
    /// failure keeps the setting and shows the red banner.
    pub fn apply_history_started(
        &mut self,
        origin: crate::message::HistoryStartOrigin,
        outcome: crate::message::HistoryStartOutcome,
    ) {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome};
        self.history_starting = false;
        // Either branch gets a new key/archive underneath it (or none at
        // all) — require a fresh re-auth check rather than trusting a stale
        // one from before the (re)start.
        self.last_history_auth = None;

        match outcome {
            HistoryStartOutcome::Ready(handle) => {
                // The user can flip the toggle off while the start is in
                // flight (it isn't blocked on it). Honor that: drop the
                // handle — the writer thread exits with it. Enable is the
                // exception: the setting is deliberately still off until
                // this very moment. An untracked session never installs a
                // writer, full stop (belt-and-braces — no start should be
                // in flight in one).
                if self.session_untracked
                    || (origin != HistoryStartOrigin::Enable
                        && !self.settings.encrypted_history_enabled())
                {
                    return;
                }
                let Some(started) = handle.take() else {
                    return;
                };
                self.history_writer = Some(started.writer);
                self.history_read = Some((started.cipher, started.keys));
                self.history_start_failed = false;
                self.history_locked = false;
                self.passphrase_prompt = None;
                if origin == HistoryStartOrigin::Enable {
                    self.settings.encrypted_history_enabled = Some(true);
                    self.settings.save();
                }

                let floor = started.seed.iter().map(|e| e.id + 1).max().unwrap_or(0);
                self.history_id_floor = self.history_id_floor.max(floor);
                self.reserve_command_ids_everywhere();

                if !started.seed.is_empty() {
                    if let Some(term) = self.active_term() {
                        let mut screen = term.screen.lock();
                        if screen.command_log.is_empty() {
                            screen.seed_command_log(started.seed);
                        } else {
                            tracing::info!(
                                "encrypted history: not seeding — commands ran before the \
                                 archive opened (they are not retro-recorded)"
                            );
                        }
                    }
                }
            }
            HistoryStartOutcome::WrongPassphrase => {
                // Wrong passphrase (or a corrupted archive — deliberately
                // indistinguishable): inline error, retry in place. History
                // stays locked, the setting stays put; this is not the red
                // "broken archive" banner.
                if let Some(prompt) = self.passphrase_prompt.as_mut() {
                    prompt.busy = false;
                    prompt.draft.clear();
                    prompt.confirm.clear();
                    prompt.error = Some(match prompt.kind {
                        PassphrasePromptKind::Unlock => {
                            "Wrong passphrase (or the archive is corrupted). Try again.".into()
                        }
                        // Enabling over an archive keyed differently (e.g.
                        // one created under the keychain source): no retry
                        // can succeed — say what actually helps.
                        PassphrasePromptKind::Enable => {
                            "An existing archive is keyed differently — this passphrase \
                             can't open it. Reset encrypted history to start fresh."
                                .into()
                        }
                    });
                }
            }
            HistoryStartOutcome::Failed => {
                match origin {
                    // An unlock failure that isn't AuthFailed (an unreadable
                    // KDF sidecar, an io error): the archive exists and the
                    // setting stays on — surface it in the prompt, with the
                    // way out.
                    HistoryStartOrigin::Unlock => {
                        if let Some(prompt) = self.passphrase_prompt.as_mut() {
                            prompt.busy = false;
                            prompt.error = Some(
                                "Couldn't open the archive (see the log). \
                                 Reset encrypted history to start fresh."
                                    .into(),
                            );
                        } else {
                            self.history_start_failed = true;
                        }
                    }
                    HistoryStartOrigin::Startup => self.history_start_failed = true,
                    // Enable/post-Reset failures revert the setting — never
                    // "on but broken".
                    HistoryStartOrigin::Enable | HistoryStartOrigin::Reset => {
                        self.history_start_failed = true;
                        self.passphrase_prompt = None;
                        self.history_locked = false;
                        self.settings.encrypted_history_enabled = Some(false);
                        self.settings.save();
                    }
                }
            }
        }
    }

    /// Raise every live screen's command-id counter to the current floor —
    /// every pane of every tab, detached windows included (they persist to
    /// the same archive).
    fn reserve_command_ids_everywhere(&mut self) {
        let floor = self.history_id_floor;
        for tab in &mut self.tabs {
            for term in tab.terms_mut() {
                term.screen.lock().reserve_command_ids(floor);
            }
        }
        for tab in self.detached.values_mut() {
            for term in tab.terms_mut() {
                term.screen.lock().reserve_command_ids(floor);
            }
        }
    }

    /// Pick the history cipher (persisted). Only takes effect the next time
    /// the feature starts fresh — see `Message::SetHistoryCipher`.
    pub fn set_history_cipher(&mut self, cipher: String) {
        self.settings.history_cipher = Some(cipher);
        self.settings.save();
    }

    /// Set the re-auth idle interval (clamped, persisted). `0` disables it,
    /// leaving only the once-per-session gate.
    pub fn set_history_reauth_interval_minutes(&mut self, n: u32) {
        self.settings.history_reauth_interval_minutes = Some(n.clamp(
            crate::settings::MIN_HISTORY_REAUTH_INTERVAL_MINUTES,
            crate::settings::MAX_HISTORY_REAUTH_INTERVAL_MINUTES,
        ));
        self.settings.save();
    }

    /// Nudge the re-auth idle interval from the settings stepper.
    pub fn step_history_reauth_interval_minutes(&mut self, delta: i64) {
        let current = i64::from(self.settings.history_reauth_interval_minutes());
        self.set_history_reauth_interval_minutes((current + delta).max(0) as u32);
    }

    /// If opening the Scrollback History panel needs a fresh Touch ID/device-
    /// password check first, the reason text to show in the native prompt —
    /// `None` if it can open immediately (off macOS, no archive active this
    /// session, or the last check is still within the once-per-session/
    /// interval policy — see `history::reauth::is_due`).
    pub fn history_reauth_reason(&self) -> Option<String> {
        if !cfg!(target_os = "macos") || self.history_writer.is_none() {
            return None;
        }
        let interval = match self.settings.history_reauth_interval_minutes() {
            0 => None,
            n => Some(n),
        };
        if history::reauth::is_due(self.last_history_auth, interval, Instant::now()) {
            Some("unlock your encrypted command history".to_string())
        } else {
            None
        }
    }

    /// Record a successful re-auth (called from the `HistoryReauthResult`
    /// handler once the native prompt succeeds).
    pub fn mark_history_authenticated(&mut self) {
        self.last_history_auth = Some(Instant::now());
    }

    /// Open the "Reset encrypted history" confirmation dialog — a distinct,
    /// explicit action from the enable/disable toggle, which never deletes
    /// anything.
    pub fn request_reset_encrypted_history(&mut self) {
        self.confirm_reset_history = true;
    }

    /// Dismiss the reset confirmation without deleting anything.
    pub fn cancel_reset_encrypted_history(&mut self) {
        self.confirm_reset_history = false;
    }

    /// Permanently delete the whole encrypted history archive (every day
    /// segment and the manifest) — the whole directory goes at once, not
    /// just the unreadable parts, so there's no risk of leaving a manifest
    /// and segments out of sync with each other. If the feature is still
    /// enabled, kicks off an async start of a fresh empty archive in its
    /// place so the user isn't left toggling it off and back on themselves.
    pub fn confirm_reset_encrypted_history(&mut self) -> iced::Task<Message> {
        self.confirm_reset_history = false;
        self.history_writer = None;
        self.history_read = None;
        self.scrollback_archived.clear();
        self.scrollback_archive_cursor = None;
        self.history_start_failed = false;
        self.last_history_auth = None;

        if let Err(e) = std::fs::remove_dir_all(history::history_dir()) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("encrypted history: failed to remove the archive directory: {e}");
            }
        }

        if self.settings.encrypted_history_enabled() && !self.session_untracked {
            match self.settings.history_key_source() {
                crate::settings::KeySource::Keychain => {
                    return self.begin_history_start(crate::message::HistoryStartOrigin::Reset);
                }
                // A fresh archive under the passphrase source needs a fresh
                // passphrase (and KDF sidecar) — ask for one instead of
                // starting anything.
                crate::settings::KeySource::Passphrase => {
                    self.history_locked = true;
                    self.passphrase_prompt =
                        Some(PassphrasePrompt::new(PassphrasePromptKind::Enable));
                }
            }
        }
        iced::Task::none()
    }

    /// The boot-time history start, chained by `main` alongside opening the
    /// main window — `Tty::new` itself must never touch the keychain (a
    /// blocked OS dialog there freezes the whole launch). The passphrase
    /// source returns no task: it boots *locked*, with the unlock prompt
    /// already open (see `Tty::new`), and starts only when the user submits.
    pub fn startup_history_task(&mut self) -> iced::Task<Message> {
        if !self.settings.encrypted_history_enabled()
            || self.settings.history_key_source() == crate::settings::KeySource::Passphrase
            || self.session_untracked
            || self.show_session_start_prompt
        {
            return iced::Task::none();
        }
        self.begin_history_start(crate::message::HistoryStartOrigin::Startup)
    }

    /// The startup chooser's answer. Record: begin the start now (keychain)
    /// or open the passphrase unlock prompt (chained, never stacked). Stay
    /// untracked: the whole session goes untracked — see
    /// [`Self::make_session_untracked`].
    pub fn choose_session_start(&mut self, record: bool) -> iced::Task<Message> {
        self.show_session_start_prompt = false;
        if record {
            if self.settings.history_key_source() == crate::settings::KeySource::Passphrase {
                self.history_locked = true;
                self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));
                return iced::Task::none();
            }
            return self.begin_history_start(crate::message::HistoryStartOrigin::Startup);
        }
        self.make_session_untracked();
        iced::Task::none()
    }

    /// Flip the whole session untracked: every existing tab (main strip and
    /// detached) and every pane's screen — future tabs inherit it via
    /// [`Self::new_tab_with`]. Commands typed before this point were never
    /// persisted either: the writer doesn't start until the chooser answers.
    fn make_session_untracked(&mut self) {
        self.session_untracked = true;
        for tab in &mut self.tabs {
            tab.untracked = true;
            for term in tab.terms_mut() {
                term.screen.lock().set_untracked(true);
            }
        }
        for tab in self.detached.values_mut() {
            tab.untracked = true;
            for term in tab.terms_mut() {
                term.screen.lock().set_untracked(true);
            }
        }
    }
}
