//! The Settings panel's **History** section: the encrypted-command-history
//! config (on/off + the key-source / KDF / fan-out / cipher choices), the
//! passphrase enable/unlock prompt, and the archive browser drill-in. Split out
//! of `view/settings.rs`; the settings dispatch there calls `history_section`.

use iced::widget::{column, row, scrollable, text};
use iced::{Element, Length};

use rime::theme;
use rime::widgets::{
    button, labeled, modal_sized, section, select, stat, stepper, table, text_field, toggle,
    TableColumn, TableMetrics,
};

use crate::history::crypto::Cipher;
use crate::message::Message;
use crate::state::Tty;

use super::age_from_epoch_ms;
use super::util::format_age;

/// Encrypted, persisted command history: on/off, and (a first-enable choice —
/// see `Settings::history_cipher`) which cipher protects it. While the archive
/// browser is open it replaces this whole section (a drill-in view with a
/// Back header) so the config list stays uncluttered and the browser gets the
/// full panel height.
/// How the fan-out PRF reads in the settings section: the concrete PRF for an
/// override, and `Auto (<resolved>)` for the default so the user can see what
/// Auto lands on for the chosen cipher.
fn fanout_label(state: &Tty, cipher: crate::history::crypto::Cipher) -> String {
    use crate::settings::HistoryFanout;
    match state.settings.history_fanout() {
        HistoryFanout::Auto => format!("Auto ({})", HistoryFanout::auto_label(cipher)),
        other => other.to_string(),
    }
}

pub(super) fn history_section(state: &Tty) -> Element<'_, Message> {
    if state.show_settings_history {
        return settings_history_browser(state);
    }

    let t = theme::tokens();
    let enabled = state.settings.encrypted_history_enabled();
    let cipher = state.settings.history_cipher();
    let key_source = state.settings.history_key_source();

    let mut body = column![
        section("Encrypted History"),
        text(
            "Persist the Scrollback History panel's commands across launches, \
             encrypted at rest. Off by default. Captured output is never \
             persisted, only the command text."
        )
        .size(12)
        .color(t.muted),
        toggle(
            "Persist encrypted command history",
            enabled,
            Message::SetEncryptedHistoryEnabled(!enabled),
        ),
    ]
    .spacing(14);

    if state.history_starting {
        body = body.push(
            text("Starting encrypted history — reading the key…")
                .size(12)
                .color(t.muted),
        );
    }

    if state.history_start_failed {
        let mut copy = "The encrypted history archive couldn't be read (a key \
                        mismatch, or corruption) — history is off for this session. \
                        Reset below to start a fresh archive, or leave it off."
            .to_string();
        if key_source == crate::settings::KeySource::Keychain {
            copy.push_str(
                " If your platform has no usable keychain (e.g. Linux without a \
                 Secret Service), switch the key source to Passphrase and re-enable.",
            );
        }
        body = body.push(text(copy).size(12).color(t.danger));
    }

    if state.session_untracked {
        let cause = if state.untracked_forced_by_cli {
            " (launched with --untracked)"
        } else {
            ""
        };
        body = body.push(
            text(format!(
                "This session is untracked{cause} — nothing typed this session \
                 is saved, and that can't change until the next launch."
            ))
            .size(12)
            .color(t.danger),
        );
    }

    if enabled {
        if state.history_locked {
            body = body.push(
                row![
                    text("Encrypted history is locked — commands are not being recorded.")
                        .size(12)
                        .color(t.danger),
                    iced::widget::Space::new().width(Length::Fill),
                    button::primary("Unlock…", Message::OpenHistoryUnlock),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        body = body.push(stat("Key source", key_source.to_string()));
        if key_source == crate::settings::KeySource::Passphrase {
            body = body.push(stat("KDF", state.settings.history_kdf().to_string()));
        }
        body = body.push(stat("Key fan-out", fanout_label(state, cipher)));
        body = body.push(stat("Cipher", cipher.to_string()));
        body = body.push(
            text(
                "To change the key source, KDF, fan-out PRF, or cipher, turn \
                 the toggle off first — and an archive that already has data \
                 keeps its originals until you Reset it.",
            )
            .size(11)
            .color(t.muted),
        );
        let session_start_pick = select(
            vec![
                crate::settings::SessionStart::Record,
                crate::settings::SessionStart::Ask,
                crate::settings::SessionStart::Untracked,
            ],
            Some(state.settings.history_session_start()),
            |s: crate::settings::SessionStart| {
                Message::SetHistorySessionStart(s.as_setting_str().to_string())
            },
        );
        body = body.push(labeled("At startup", session_start_pick));
        body = body.push(
            text(
                "What each launch does: record right away, ask first, or start \
                 the whole session untracked (nothing recorded, no key read). \
                 One launch can also be forced untracked with tty --untracked.",
            )
            .size(11)
            .color(t.muted),
        );
        if state.history_writer.is_some() {
            body = body.push(
                row![
                    text("Browse the archive — most recent day first.")
                        .size(11)
                        .color(t.muted),
                    iced::widget::Space::new().width(Length::Fill),
                    button::ghost(
                        "View archived commands…",
                        Message::ToggleSettingsHistoryViewer
                    ),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
    } else {
        // Greyed-out until enabled: these are decided in the enable dialog,
        // not here — showing them inert (current values, muted) signals
        // "configured on enable" instead of inviting dead clicks.
        let mut rows = vec![
            format!("Key source — {key_source}"),
            format!("Key fan-out — {}", fanout_label(state, cipher)),
            format!("Cipher — {cipher}"),
        ];
        if key_source == crate::settings::KeySource::Passphrase {
            rows.insert(1, format!("KDF — {}", state.settings.history_kdf()));
        }
        for label in rows {
            body = body.push(text(label).size(12).color(t.muted));
        }
        body = body.push(
            text(
                "You choose all of these in the dialog that opens when you \
                 turn the toggle on — with a passphrase, that's also where \
                 you set it. Fixed once the archive has data.",
            )
            .size(11)
            .color(t.muted),
        );
    }

    if cfg!(target_os = "macos") {
        let minutes = state.settings.history_reauth_interval_minutes();
        body = body.push(stepper(
            "Also re-authenticate every (minutes, 0 = once per session)",
            if minutes == 0 {
                "0 (off)".to_string()
            } else {
                minutes.to_string()
            },
            Message::HistoryReauthIntervalStep(-5),
            Message::HistoryReauthIntervalStep(5),
        ));
        body = body.push(
            text(
                "Opening the Scrollback History panel always asks for Touch ID \
                 (or your device password) once per launch, once history is on. \
                 Set an interval to ask again after that much idle time too.",
            )
            .size(11)
            .color(t.muted),
        );
    }

    body = body.push(
        text(
            "Permanently delete the archive — separate from the toggle above, \
             which never deletes anything on its own.",
        )
        .size(11)
        .color(t.muted),
    );
    body = body.push(button::danger(
        "Reset encrypted history…",
        Message::RequestResetEncryptedHistory,
    ));

    // The section outgrew the settings panel (key source + KDF + cipher +
    // startup + reauth + reset): without a scrollable, iced silently clips
    // whatever falls below the panel's height — the Reset button vanished.
    scrollable(body.padding(iced::Padding::ZERO.right(8)))
        .height(Length::Fill)
        .into()
}

/// The one history dialog, in two shapes. **Enable**: every fixed-at-enable
/// choice lives here — key source (switching it live reshapes the dialog),
/// the passphrase + KDF when Passphrase is picked (or the OS-keychain
/// explainer when Keychain is), and the cipher — so the settings section can
/// stay greyed out until the feature is actually on. **Unlock**: just the
/// existing archive's passphrase. While a derivation runs (deliberately
/// slow, on a background thread) the actions row becomes a progress label.
pub(super) fn passphrase_prompt_view<'a>(
    state: &'a Tty,
    prompt: &'a crate::state::PassphrasePrompt,
    base: Element<'a, Message>,
) -> Element<'a, Message> {
    use crate::state::PassphrasePromptKind;
    let t = theme::tokens();
    let key_source = state.settings.history_key_source();

    let mut body = column![].spacing(12);
    // The keychain path never derives anything, so Continue is safe to offer
    // the moment the dialog opens; the passphrase path submits its fields.
    let mut submit: (&str, Message) = ("Enable", Message::SubmitHistoryPassphrase);

    match prompt.kind {
        PassphrasePromptKind::Enable => {
            body = body.push(text("Enable encrypted history").size(16));
            body = body.push(
                text(
                    "These choices are fixed once the archive has data \
                     (changing them later means a Reset):",
                )
                .size(12)
                .color(t.muted),
            );
            let source_pick = select(
                vec![
                    crate::settings::KeySource::Keychain,
                    crate::settings::KeySource::Passphrase,
                ],
                Some(key_source),
                |s: crate::settings::KeySource| {
                    Message::SetHistoryKeySource(s.as_setting_str().to_string())
                },
            );
            body = body.push(labeled("Key source", source_pick));

            match key_source {
                crate::settings::KeySource::Keychain => {
                    submit = ("Continue", Message::ConfirmEnableHistory);
                    body = body.push(
                        text(
                            "A random key, stored in your OS keychain. Your \
                             system may now ask you to allow tty to access it \
                             — that prompt comes from the OS, not from tty, \
                             and Deny leaves history off.",
                        )
                        .size(12)
                        .color(t.muted),
                    );
                }
                crate::settings::KeySource::Passphrase => {
                    let kdf_pick = select(
                        vec![
                            crate::settings::HistoryKdf::Argon2id,
                            crate::settings::HistoryKdf::Scrypt,
                            crate::settings::HistoryKdf::Pbkdf2,
                        ],
                        Some(state.settings.history_kdf()),
                        |k: crate::settings::HistoryKdf| {
                            Message::SetHistoryKdf(k.as_setting_str().to_string())
                        },
                    );
                    body = body.push(labeled("KDF (Argon2id recommended)", kdf_pick));
                    body = body.push(
                        text(
                            "The key is derived from this passphrase. There is \
                             no recovery: lose it and the archive is unreadable \
                             — a Reset is the only way back.",
                        )
                        .size(12)
                        .color(t.muted),
                    );
                    body = body.push(
                        text_field(
                            "Passphrase…",
                            &prompt.draft,
                            Message::HistoryPassphraseChanged,
                        )
                        .secure(true)
                        .on_submit(Message::SubmitHistoryPassphrase),
                    );
                    body = body.push(
                        text_field(
                            "Confirm passphrase…",
                            &prompt.confirm,
                            Message::HistoryPassphraseConfirmChanged,
                        )
                        .secure(true)
                        .on_submit(Message::SubmitHistoryPassphrase),
                    );
                }
            }

            // Presented in pipeline order: the fan-out PRF runs first (it
            // splits the key into the per-file subkeys), then the cipher's
            // per-file AEAD consumes those subkeys — so the fan-out picker
            // comes above the cipher picker.
            use crate::settings::HistoryFanout;
            let cipher = state.settings.history_cipher();

            let fanout_pick = select(
                vec![
                    HistoryFanout::Auto,
                    HistoryFanout::Skein512,
                    HistoryFanout::Blake3,
                ],
                Some(state.settings.history_fanout()),
                |fo: HistoryFanout| Message::SetHistoryFanout(fo.as_setting_str().to_string()),
            );
            body = body.push(labeled("Key fan-out PRF (Auto recommended)", fanout_pick));
            body = body.push(
                text(format!(
                    "The keyed hash that fans the key out into per-file subkeys, \
                     before anything is encrypted. Auto pairs it with the cipher \
                     below — BLAKE3 with ChaCha20-Poly1305, Skein-512 with \
                     Threefish (here: {}). Both are equally strong; overriding \
                     only keeps the whole construction in one family.",
                    HistoryFanout::auto_label(cipher),
                ))
                .size(12)
                .color(t.muted),
            );

            let cipher_pick = select(
                vec![Cipher::ChaCha20Poly1305, Cipher::DoradoRawAuthenticated],
                Some(cipher),
                |c: Cipher| Message::SetHistoryCipher(c.as_setting_str().to_string()),
            );
            body = body.push(labeled(
                "Cipher (ChaCha20-Poly1305 recommended)",
                cipher_pick,
            ));
        }
        PassphrasePromptKind::Unlock => {
            submit = ("Unlock", Message::SubmitHistoryPassphrase);
            body = body.push(text("Unlock encrypted history").size(16));
            body = body.push(
                text(
                    "Enter the archive's passphrase. Until it's unlocked, \
                     commands are not being recorded.",
                )
                .size(12)
                .color(t.muted),
            );
            body = body.push(
                text_field(
                    "Passphrase…",
                    &prompt.draft,
                    Message::HistoryPassphraseChanged,
                )
                .secure(true)
                .on_submit(Message::SubmitHistoryPassphrase),
            );
        }
    }

    if let Some(error) = &prompt.error {
        body = body.push(text(error.clone()).size(12).color(t.danger));
    }

    let actions: Element<'_, Message> = if prompt.busy {
        text("Deriving the key…").size(12).color(t.muted).into()
    } else {
        row![
            iced::widget::Space::new().width(Length::Fill),
            button::ghost("Cancel", Message::CancelHistoryPassphrase),
            button::primary(submit.0, submit.1),
        ]
        .spacing(8)
        .into()
    };
    body = body.push(actions);

    modal_sized(base, body, Message::CancelHistoryPassphrase, 480.0)
}

/// The archive browser the History section drills into (behind the same
/// re-auth gate as the panel): a Back header, then a full-height scrollable
/// table of persisted commands (command text only; output is never
/// persisted). Right-click a row for Copy / Delete… (Delete confirms first);
/// double-click copies directly.
fn settings_history_browser(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let mut body = column![row![
        button::ghost("‹ Back", Message::ToggleSettingsHistoryViewer),
        section("Archived Commands"),
        iced::widget::Space::new().width(Length::Fill),
        button::ghost("Load older day", Message::SettingsHistoryPageOlder),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)]
    .spacing(14)
    .height(Length::Fill);

    if state.settings_history.is_empty() {
        return body
            .push(
                text("No archived commands yet — run a command and it will appear here.")
                    .size(12)
                    .color(t.muted),
            )
            .into();
    }

    let cursor_label = match state.settings_history_cursor {
        Some(date) => format!("Archived back to {date} — right-click a row to copy or delete it."),
        None => String::new(),
    };
    body = body.push(text(cursor_label).size(11).color(t.muted));

    // (cell text, archive address) per row — Copy and double-click use the
    // target's command (without the metadata suffix); Delete needs the rest
    // to tombstone the entry on disk.
    let now_ms = state.now_ms();
    let rows: std::rc::Rc<Vec<(String, crate::state::ArchivedTarget)>> = std::rc::Rc::new(
        state
            .settings_history
            .iter()
            .map(|e| {
                let date = crate::history::local_date_from_epoch_ms(e.started_at_epoch_ms);
                let age = format_age(age_from_epoch_ms(now_ms, e.started_at_epoch_ms));
                (
                    format!("{}  · {} · {} · {}", e.command, date, e.pane_tag, age),
                    crate::state::ArchivedTarget {
                        date,
                        id: e.id,
                        started_at_epoch_ms: e.started_at_epoch_ms,
                        pane_tag: e.pane_tag.clone(),
                        command: e.command.clone(),
                    },
                )
            })
            .collect(),
    );
    let row_count = rows.len();
    let cell_rows = rows.clone();
    let activate_rows = rows.clone();
    let right_click_rows = rows;

    let archive_table = table(
        row_count,
        vec![TableColumn::fill("Command")],
        move |row, _col| cell_rows[row].0.clone(),
    )
    .metrics(TableMetrics {
        row_height: 18.0,
        header_height: 0.0,
    })
    .offset(state.settings_history_scroll)
    .selected(state.settings_history_selected)
    .font(iced::Font::MONOSPACE)
    .on_scroll(Message::SettingsHistoryScrolled)
    .on_select(Message::SettingsHistoryRowSelected)
    .on_activate(move |row| {
        Message::SettingsHistoryRowActivated(
            row,
            cathode::commands::strip_prompt(&activate_rows[row].1.command).to_string(),
        )
    })
    .on_right_click(move |row| {
        Message::SettingsHistoryRowRightClick(row, right_click_rows[row].1.clone())
    })
    .width(Length::Fill)
    .height(Length::Fill);

    body.push(archive_table).into()
}
