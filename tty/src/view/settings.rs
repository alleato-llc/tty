//! The `⌘,` settings panel: the section dispatch and the Appearance sub-tabs,
//! the Palette, Keys, and Metrics sections, and the per-section editors. The
//! History section (encrypted-history config, passphrase prompts, archive
//! browser) lives in the sibling `settings_history` module. Split out of
//! `view.rs`.

use iced::widget::{column, container, row, scrollable, text, Column};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, caption, color_field, labeled, section, select, shortcut_row, slider, stepper,
    text_field, toggle, tooltip, TooltipPosition,
};

use crate::message::Message;
use crate::state::Tty;

use super::settings_history::history_section;
use super::status_bar_metrics_editor;

/// The body of the active settings section.
pub(super) fn settings_body(state: &Tty) -> Element<'_, Message> {
    match state.settings_section {
        1 => palette_section(state),
        2 => keys_section(),
        3 => metrics_section(state),
        4 => history_section(state),
        5 => shell_section(state),
        _ => appearance_section(state),
    }
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
                ("⌘↑ / ⌘↓", "Jump to previous / next command prompt"),
                ("⌘⇧↑ / ⌘⇧↓", "Jump between failed commands only"),
                ("⌘⇧O", "Copy the last command's output"),
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
    // exclude-list convention in fed-ide's settings): edited by hand in the config file.
    let overrides_text = if state.settings.output_line_overrides.is_empty() {
        "None — add entries (e.g. { pattern = \"tail *\", max_lines = 200 }) under \
         [[output_line_overrides]] in tty.toml."
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
        caption("OPEN FILE ON ⌘-CLICK"),
        labeled(
            "Command",
            text_field(
                "Default: open in the system app",
                state.settings.open_file_command.as_deref().unwrap_or(""),
                Message::OpenFileCommandChanged,
            ),
        ),
        text(
            "⌘-click a path:line[:col] in output to open it. Blank uses the system \
             opener (no line). Set a template with {file}/{line}/{col}, e.g. \
             \"code -g {file}:{line}:{col}\" or \"vim +{line} {file}\"."
        )
        .size(12)
        .color(t.muted),
    ]
    .spacing(14)
    .into()
}

/// The top-level **Shell** section: the OSC 133 shell-integration group — one master
/// toggle, with every sub-option (notifications, gutter, auto-install, the manual
/// snippet) shown only when it's on, so the whole feature reads as a single cohesive
/// unit. It lives here rather than under Appearance because it's terminal *behavior*,
/// not looks.
fn shell_section(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let si = state.settings.shell_integration();
    let mut col = column![
        section("Shell integration (OSC 133)"),
        toggle(
            "Enable shell integration (command marks)",
            si.enabled,
            Message::SetShellIntegrationEnabled(!si.enabled),
        ),
        text(
            "Marks each command's prompt + exit status so tty can notify you when one \
             finishes, jump between prompts (⌘↑/⌘↓), flag failures, and copy output. \
             Needs OSC 133 hooks in your shell (below)."
        )
        .size(12)
        .color(t.muted),
    ]
    .spacing(14);
    if si.enabled {
        col = col
            .push(toggle(
                "Notify when a command finishes while unfocused",
                si.notify,
                Message::SetNotifyOnCommandFinish(!si.notify),
            ))
            .push(stepper(
                "Only notify for commands longer than (seconds)",
                si.notify_min_seconds.to_string(),
                Message::NotifyMinSecondsStep(-5),
                Message::NotifyMinSecondsStep(5),
            ))
            .push(toggle(
                "Mark command prompts in a left gutter (failed = red)",
                si.gutter,
                Message::SetPromptGutter(!si.gutter),
            ))
            .push(toggle(
                "Auto-install hooks into new shells (zsh)",
                si.autoinstall,
                Message::SetShellIntegrationAutoinstall(!si.autoinstall),
            ))
            .push(
                text("Auto-install wires zsh up for you; otherwise add this to your ~/.zshrc:")
                    .size(12)
                    .color(t.muted),
            )
            .push(
                container(
                    text(crate::shell_integration::ZSH_SNIPPET)
                        .font(iced::Font::MONOSPACE)
                        .size(11)
                        .color(t.ink),
                )
                .padding(8)
                .width(Length::Fill)
                .style(move |_| container::Style {
                    border: Border {
                        color: t.hairline,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..container::background(t.surface)
                }),
            )
            .push(button::secondary("Copy snippet", Message::CopyShellSnippet));
    }
    col.into()
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
