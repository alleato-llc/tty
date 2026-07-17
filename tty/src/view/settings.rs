//! The `⌘,` settings panel: the section dispatch and every section/sub-tab body
//! (Appearance, Palette, Keys, Metrics, History) plus the encrypted-history
//! prompts and archive browser. Split out of `view.rs`; it borrows the parent's
//! imports and private helpers via `use super::*`.

use iced::widget::{column, container, row, scrollable, text, Column};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, caption, color_field, labeled, modal_sized, section, select, shortcut_row, slider,
    stat, stepper, table, text_field, toggle, tooltip, TableColumn, TableMetrics, TooltipPosition,
};

use crate::history::crypto::Cipher;
use crate::message::Message;
use crate::state::Tty;

use super::util::format_age;
use super::{age_from_epoch_ms, status_bar_metrics_editor};

/// The body of the active settings section.
pub(super) fn settings_body(state: &Tty) -> Element<'_, Message> {
    match state.settings_section {
        1 => palette_section(state),
        2 => keys_section(),
        3 => metrics_section(state),
        4 => history_section(state),
        _ => appearance_section(state),
    }
}

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

fn history_section(state: &Tty) -> Element<'_, Message> {
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
    let rows: std::rc::Rc<Vec<(String, crate::state::ArchivedTarget)>> = std::rc::Rc::new(
        state
            .settings_history
            .iter()
            .map(|e| {
                let date = crate::history::local_date_from_epoch_ms(e.started_at_epoch_ms);
                let age = format_age(age_from_epoch_ms(e.started_at_epoch_ms));
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

/// Keys: a read-only reference of the keyboard shortcuts, grouped by area. This is
/// documentation, not configuration — the bindings live in `update::handle_key` and
/// `phosphor::input`; this list mirrors them so users don't have to hunt the README.
fn keys_section<'a>() -> Element<'a, Message> {
    // (group title, rows of (chord, what it does)).
    let groups: [(&str, &[(&str, &str)]); 4] = [
        (
            "Tabs & panes",
            &[
                ("⌘T / ⌘N", "New tab"),
                ("⌘⇧T", "New untracked tab (never saved to history)"),
                ("⌘1–⌘9", "Jump to tab"),
                ("⌥⌘ + arrows", "Split the focused pane (←/→/↑/↓)"),
                ("⌃⌘ + arrows", "Move focus between panes"),
                ("drag a divider", "Resize a split"),
                ("right-click / ⌃-click", "Tab or pane context menu"),
                ("⌘W", "Close pane → tab → quit"),
            ],
        ),
        (
            "Line editing",
            &[
                ("⌥← / ⌥→", "Move by word"),
                ("⌘← / ⌘→", "To line start / end"),
                ("⌥⌫", "Delete a word"),
                ("⌘⌫", "Delete to line start"),
                ("⌘C / ⌘V", "Copy selection / paste"),
                ("Ctrl+C", "Interrupt (real SIGINT)"),
            ],
        ),
        (
            "View",
            &[
                ("⌘+ / ⌘−", "Font zoom"),
                ("⌘0", "Reset zoom"),
                ("⌘F", "Find in scrollback"),
                ("⌘K", "Clear the focused pane's scrollback"),
                ("⌘⇧H", "Scrollback History (commands + output)"),
                ("wheel", "Scroll back through history"),
                ("⌘,", "Settings"),
            ],
        ),
        (
            "Find",
            &[("Enter", "Close the find bar"), ("Esc", "Cancel")],
        ),
    ];

    let mut body = Column::new().spacing(14);
    for (title, rows) in groups {
        let mut list = Column::new().spacing(6);
        for (chord, desc) in rows {
            list = list.push(shortcut_row(chord, desc));
        }
        body = body.push(column![section(title), list].spacing(8));
    }

    scrollable(body.padding(iced::Padding::ZERO.right(8)))
        .height(Length::Fill)
        .into()
}

/// Appearance: named theme, font family, font size.
/// The Appearance section's sub-tabs, in display order. Each groups a slice of
/// the (formerly one long) Appearance settings so only one pane shows at a time.
/// Indices match `Tty::appearance_tab` and the `appearance_*_pane` dispatch.
pub const APPEARANCE_TABS: [&str; 5] = ["Theme", "Tabs", "Status bar", "Terminal", "Window"];

fn appearance_section(state: &Tty) -> Element<'_, Message> {
    // A horizontal sub-tab strip splits the section into panes so it isn't one
    // long scroll; the pane below scrolls on its own for a short window.
    let strip = settings_subtabs(
        &APPEARANCE_TABS,
        state.appearance_tab,
        Message::AppearanceTab,
    );
    let pane = match state.appearance_tab {
        1 => appearance_tabs_pane(state),
        2 => appearance_statusbar_pane(state),
        3 => appearance_terminal_pane(state),
        4 => appearance_window_pane(state),
        _ => appearance_theme_pane(state),
    };
    column![
        strip,
        scrollable(container(pane).padding(iced::Padding::ZERO.right(8))).height(Length::Fill),
    ]
    .spacing(14)
    .into()
}

/// A horizontal row of sub-tab chips (the active one inked on a raised chip,
/// mirroring the settings shell's section rail), for splitting a settings section
/// into panes. `on_select(i)` switches to sub-tab `i`.
fn settings_subtabs<'a>(
    labels: &'a [&'a str],
    active: usize,
    on_select: impl Fn(usize) -> Message + 'a,
) -> Element<'a, Message> {
    let t = theme::tokens();
    let mut strip = row![].spacing(4);
    for (i, label) in labels.iter().enumerate() {
        let is_active = i == active;
        let color = if is_active { t.ink } else { t.muted };
        strip = strip.push(
            iced::widget::button(text((*label).to_string()).size(13).color(color))
                .on_press(on_select(i))
                .padding([6, 12])
                .style(move |_, _| iced::widget::button::Style {
                    background: is_active.then(|| t.bg.into()),
                    text_color: color,
                    border: Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    strip.into()
}

/// Appearance → Theme: terminal theme, font family, and font size.
fn appearance_theme_pane(state: &Tty) -> Element<'_, Message> {
    // Theme: the rime built-in set. A custom palette (base16/edit) reads as "Custom".
    let mut themes = crate::theme::theme_names();
    let current_theme = if state.settings.palette.is_some() {
        themes.insert(0, "Custom".to_string());
        "Custom".to_string()
    } else {
        state
            .settings
            .theme
            .clone()
            .unwrap_or_else(|| "Dracula".into())
    };
    let theme_pick = select(themes, Some(current_theme), Message::SetTheme);

    // Font family: a curated list; the active one (or the default label) is selected.
    let fonts: Vec<String> = crate::state::FONT_CHOICES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let current_font = state
        .settings
        .font_family
        .clone()
        .unwrap_or_else(|| crate::state::DEFAULT_FONT_LABEL.to_string());
    let font_pick = select(fonts, Some(current_font), Message::SetFont);

    column![
        labeled("Theme", theme_pick),
        labeled("Font", font_pick),
        stepper(
            "Font size",
            format!("{}px", state.font_size as u32),
            Message::FontSizeStep(-1.0),
            Message::FontSizeStep(1.0),
        ),
    ]
    .spacing(14)
    .into()
}

/// Appearance → Tabs: how loud the active tab reads. Off swaps the accent ink for
/// a subtler normal-ink emphasis (it still beats the muted inactive tabs).
fn appearance_tabs_pane(state: &Tty) -> Element<'_, Message> {
    column![
        toggle(
            "Highlight active tab",
            state.settings.tab_highlight(),
            Message::SetTabHighlight(!state.settings.tab_highlight()),
        ),
        tooltip(
            toggle(
                "Highlight the focused pane",
                state.settings.highlight_focused_pane(),
                Message::SetHighlightFocusedPane(!state.settings.highlight_focused_pane()),
            ),
            "In a tab that's split into more than one pane, outline the focused \
             pane with an accent border so it's clear where typing goes. Off keeps \
             every pane on a neutral hairline.",
            TooltipPosition::Top,
        ),
    ]
    .spacing(14)
    .into()
}

/// Appearance → Status bar: the bar's own chrome (off switch + auto-hide). The
/// machine-stat cells and everything about their drill-ins live in the separate
/// **Metrics** section.
fn appearance_statusbar_pane(state: &Tty) -> Element<'_, Message> {
    let disabled = state.settings.status_bar_disabled();
    let mut col = column![toggle(
        "Disable status bar",
        disabled,
        Message::SetStatusBarDisabled(!disabled),
    )]
    .spacing(14);
    if disabled {
        col = col.push(caption(
            "The status bar is off. Turn it back on to configure auto-hide; machine-stat cells are in the Metrics section.",
        ));
    } else {
        col = col.push(toggle(
            "Auto-hide until pointer nears the bottom",
            state.settings.status_bar_autohide(),
            Message::SetStatusBarAutohide(!state.settings.status_bar_autohide()),
        ));
    }
    col.into()
}

/// The **Metrics** settings section: the machine-stat cell editor plus everything
/// about the drill-ins — pin popovers, graduate-into-a-pane, the reorder hold,
/// per-cell alert thresholds, and clock format.
fn metrics_section(state: &Tty) -> Element<'_, Message> {
    let mut col = column![section("Metrics")].spacing(14);
    if state.settings.status_bar_disabled() {
        col = col.push(caption(
            "The status bar is off (Appearance → Status bar). Cells only show once it's back on.",
        ));
    }
    col = col
        .push(toggle(
            "Keep metric popovers open (pin several; click away won't close)",
            state.settings.status_bar_metrics_pinned(),
            Message::SetStatusBarMetricsPinned(!state.settings.status_bar_metrics_pinned()),
        ))
        .push(tooltip(
            toggle(
                "Let drill-ins graduate into split panes (the ⊞ control)",
                state.settings.graduate_metrics(),
                Message::SetGraduateMetrics(!state.settings.graduate_metrics()),
            ),
            "When on, a metric drill-in's ⊞ moves it out of the floating popover \
             into a real split pane (Left / Right / Up / Down) with its own \
             maximize / close. Turn off to keep metrics as popovers only.",
            TooltipPosition::Top,
        ))
        .push(tooltip(
            stepper(
                "Reorder hold",
                format!("{:.1}s", state.settings.status_bar_edit_hold_secs()),
                Message::SetStatusBarEditHold(-0.5),
                Message::SetStatusBarEditHold(0.5),
            ),
            "How long to press and hold a metric before it enters \
             drag-to-reorder edit mode — the outline appears only then, never \
             on a quick click (which opens the drill-in). Scroll over the bar \
             to page through metrics that don't fit; Esc leaves edit mode.",
            TooltipPosition::Top,
        ))
        .push(status_bar_metrics_editor(state));
    // Threshold controls, only when a graded (CPU/mem/battery) cell is set.
    if state
        .settings
        .status_bar_metrics
        .iter()
        .filter_map(|c| crate::settings::MetricKind::from_setting_str(&c.metric))
        .any(|k| k.is_graded())
    {
        col = col.push(thresholds_editor(state));
    }
    // Clock format options, only when a clock cell is configured.
    if state
        .settings
        .status_bar_metrics()
        .iter()
        .any(|m| m.kind == crate::settings::MetricKind::Clock)
    {
        col = col.push(clock_format_editor(state));
    }
    col.into()
}

/// Per-cell caution/alarm threshold steppers for the graded metrics (CPU, memory,
/// battery) currently configured. Shown under the metrics editor when any graded
/// cell is present; keyed by the raw-list index so the messages line up.
fn thresholds_editor(state: &Tty) -> Element<'_, Message> {
    use crate::settings::MetricKind;
    let t = theme::tokens();
    let mut col = column![caption("ALERT THRESHOLDS")].spacing(10);
    for (i, cfg) in state.settings.status_bar_metrics.iter().enumerate() {
        let Some(kind) = MetricKind::from_setting_str(&cfg.metric) else {
            continue;
        };
        let Some((dw, da, inverted)) = kind.default_thresholds() else {
            continue;
        };
        let warn = cfg.warn.unwrap_or(dw);
        let alarm = cfg.alarm.unwrap_or(da);
        // Battery alarms when charge falls *below* the cutoffs; note that so the
        // ordering (alarm < warn) doesn't read as a mistake.
        let note = if inverted { " (low)" } else { "" };
        col = col
            .push(text(format!("{kind}{note}")).size(11).color(t.muted))
            .push(stepper(
                "Caution at",
                format!("{}%", warn as i32),
                Message::StatusBarMetricThreshold(i, true, -5.0),
                Message::StatusBarMetricThreshold(i, true, 5.0),
            ))
            .push(stepper(
                "Alarm at",
                format!("{}%", alarm as i32),
                Message::StatusBarMetricThreshold(i, false, -5.0),
                Message::StatusBarMetricThreshold(i, false, 5.0),
            ));
    }
    col.into()
}

/// The clock cell's format toggles (24-hour, seconds, date), shown under the
/// metrics editor when a clock cell is present.
fn clock_format_editor(state: &Tty) -> Element<'_, Message> {
    column![
        caption("CLOCK FORMAT"),
        toggle(
            "24-hour time",
            state.settings.clock_24h.unwrap_or(false),
            Message::SetClock24h(!state.settings.clock_24h.unwrap_or(false)),
        ),
        toggle(
            "Show seconds",
            state.settings.clock_seconds.unwrap_or(false),
            Message::SetClockSeconds(!state.settings.clock_seconds.unwrap_or(false)),
        ),
        toggle(
            "Show date",
            state.settings.clock_date.unwrap_or(false),
            Message::SetClockDate(!state.settings.clock_date.unwrap_or(false)),
        ),
    ]
    .spacing(14)
    .into()
}

/// Appearance → Terminal: scrollback depth and per-command output caps.
fn appearance_terminal_pane(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    // Per-command output-cap overrides — read-only here (mirrors the Local History
    // exclude-list convention in fed-ide's settings): edited by hand in the JSON file.
    let overrides_text = if state.settings.output_line_overrides.is_empty() {
        "None — add entries (e.g. {\"pattern\": \"tail *\", \"max_lines\": 200}) in \
         tty.settings.json."
            .to_string()
    } else {
        state
            .settings
            .output_line_overrides
            .iter()
            .map(|o| format!("{} → {} lines", o.pattern, o.max_lines))
            .collect::<Vec<_>>()
            .join(", ")
    };

    column![
        stepper(
            "Max scrollback lines",
            state.settings.max_scrollback().to_string(),
            Message::MaxScrollbackStep(-500),
            Message::MaxScrollbackStep(500),
        ),
        stepper(
            "Default output lines per command",
            state.settings.default_output_lines().to_string(),
            Message::DefaultOutputLinesStep(-10),
            Message::DefaultOutputLinesStep(10),
        ),
        caption("PER-COMMAND OVERRIDES"),
        text(overrides_text).size(12).color(t.muted),
    ]
    .spacing(14)
    .into()
}

/// Appearance → Window: keep-on-top, and the two transparency amounts (active
/// and on-blur). Each transparency is shown as a 0–max% amount and stored as the
/// resulting opacity (1 − amount).
fn appearance_window_pane(state: &Tty) -> Element<'_, Message> {
    // Always on top: keep the window above other apps' windows.
    let on_top = toggle(
        "Keep window on top of other windows",
        state.settings.window_always_on_top(),
        Message::SetWindowAlwaysOnTop(!state.settings.window_always_on_top()),
    );

    // Active transparency: fades even while focused, capped at 50% so an in-use
    // window stays readable.
    let active = 1.0 - state.settings.focused_opacity();
    let active_max = 1.0 - crate::settings::MIN_FOCUSED_OPACITY;
    let active_control = tooltip(
        slider(
            "Transparency When Active",
            0.0..=active_max,
            active,
            format!("{}%", (active * 100.0).round() as i32),
            |t| Message::SetFocusedOpacity(1.0 - t),
        ),
        "Fades the window even while you're using it, so what's behind shows \
         through. Capped at 50% so it stays readable. At 0% it stays opaque.",
        TooltipPosition::Top,
    );

    // Blur transparency: fades further when the window loses focus (up to 95%).
    let blur = 1.0 - state.settings.unfocused_opacity();
    let blur_max = 1.0 - crate::settings::MIN_OPACITY;
    let blur_control = tooltip(
        slider(
            "Transparency On Blur",
            0.0..=blur_max,
            blur,
            format!("{}%", (blur * 100.0).round() as i32),
            |t| Message::SetUnfocusedOpacity(1.0 - t),
        ),
        "Fades the whole window when it loses focus, so what's behind it \
         shows through. At 0% it stays opaque.",
        TooltipPosition::Top,
    );

    column![on_top, active_control, blur_control]
        .spacing(14)
        .into()
}

/// Palette: import a base16 scheme, or tweak the 16 ANSI colors + fg/bg/cursor directly.
fn palette_section(state: &Tty) -> Element<'_, Message> {
    let import = column![
        labeled(
            "base16 scheme",
            text_field(
                "Paste 16 hex colors (base00…base0F)",
                &state.base16_input,
                Message::Base16Changed,
            ),
        ),
        row![
            button::primary("Import", Message::ApplyBase16),
            button::secondary("Reset to default", Message::ResetPalette),
        ]
        .spacing(8),
    ]
    .spacing(8);

    // The live palette, slot by slot. Editing one composes onto the current colors.
    let style = state.theme.terminal;
    let labels = [
        "ANSI 0 · black",
        "ANSI 1 · red",
        "ANSI 2 · green",
        "ANSI 3 · yellow",
        "ANSI 4 · blue",
        "ANSI 5 · magenta",
        "ANSI 6 · cyan",
        "ANSI 7 · white",
        "ANSI 8 · br black",
        "ANSI 9 · br red",
        "ANSI 10 · br green",
        "ANSI 11 · br yellow",
        "ANSI 12 · br blue",
        "ANSI 13 · br magenta",
        "ANSI 14 · br cyan",
        "ANSI 15 · br white",
    ];
    let mut swatches = Column::new().spacing(8);
    for (i, label) in labels.iter().enumerate() {
        swatches = swatches.push(color_field(label, style.ansi[i], move |c| {
            Message::EditColor(i, c)
        }));
    }
    swatches = swatches
        .push(color_field("Foreground", style.fg, |c| {
            Message::EditColor(16, c)
        }))
        .push(color_field("Background", style.bg, |c| {
            Message::EditColor(17, c)
        }))
        .push(color_field("Cursor", style.cursor, |c| {
            Message::EditColor(18, c)
        }));

    scrollable(
        column![section("Palette"), import, swatches]
            .spacing(14)
            .padding([0, 8]),
    )
    .height(Length::Fill)
    .into()
}
