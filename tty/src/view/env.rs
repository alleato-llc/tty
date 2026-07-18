//! The **Env view** — a popover (like the metric charts): non-modal, drag it by
//! anywhere on the card, border-resize it, and it stays open beside the terminal so
//! it tracks the shell as commands run. It opens **compact** — a masked list of the
//! active pane's environment plus an Add button — and expands to the full experience
//! (filter, revealed values, the source note). Click a row to copy `NAME=value`. Data
//! comes from [`crate::env`]; the draggable/resizable chrome is [`rime::widgets::popover`].

use iced::widget::{column, container, mouse_area, row, scrollable, stack, text, Space};
use iced::{Border, Element, Font, Length, Padding};
use rime::theme;
use rime::widgets::{button, modal_sized, popover, section, text_field, toggle};

use crate::message::Message;
use crate::state::{EnvSource, Tty};

/// Longest revealed value shown inline (the full value is still what a click copies).
const MAX_VALUE_CHARS: usize = 120;

/// Place the Env popover over `base` (no scrim — the terminal stays live behind it).
/// The whole card is a drag handle (press-and-drag the body to move it) exactly like
/// the metric popovers; inner controls (buttons, fields, rows, resize strips) capture
/// their own presses first. `opaque` stops a press on the card leaking to the terminal.
pub(super) fn place_env_popover<'a>(
    state: &'a Tty,
    base: Element<'a, Message>,
) -> Element<'a, Message> {
    let (x, y) = state.env_effective_pos();
    let (w, h) = state.env_size;
    let floating = popover(
        card(state, w, h),
        Message::EnvMoveStart,
        Message::EnvResizeStart,
    );
    let placed = container(floating)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Left)
        .align_y(iced::alignment::Vertical::Top)
        .padding(Padding::ZERO.left(x).top(y));
    stack![base, placed].into()
}

/// The "Set a variable" modal (opened by the Add button) — a centered dialog over a
/// dimmed terminal. Types an `export`/`unset` at the focused shell's prompt via the
/// existing inject path; only reachable when env editing is enabled.
pub(super) fn place_env_add_modal<'a>(
    state: &'a Tty,
    base: Element<'a, Message>,
) -> Element<'a, Message> {
    let t = theme::tokens();
    let content = column![
        section("Set a variable"),
        text("Types an export at the focused shell's prompt (only while it's idle).")
            .size(11)
            .color(t.muted),
        text_field("NAME", &state.env_set_name, Message::EnvSetNameChanged).size(13),
        text_field("value", &state.env_set_value, Message::EnvSetValueChanged).size(13),
        row![
            button::ghost("Unset", Message::EnvInjectUnset),
            Space::new().width(Length::Fill),
            button::ghost("Cancel", Message::CloseEnvAdd),
            button::primary("Set", Message::EnvInjectSet),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(12);
    modal_sized(base, content, Message::CloseEnvAdd, 380.0)
}

fn card<'a>(state: &'a Tty, w: f32, h: f32) -> Element<'a, Message> {
    let t = theme::tokens();
    let expanded = state.env_expanded;
    let editing = state.settings.shell_integration().env_editing;
    // Values only unmask when expanded — the compact list stays masked.
    let reveal = expanded && state.env_reveal;
    // The filter only applies in the expanded view; compact shows the whole list.
    let filter = if expanded {
        state.env_filter.to_lowercase()
    } else {
        String::new()
    };
    let matched: Vec<&crate::env::EnvVar> = state
        .env_vars
        .iter()
        .filter(|v| {
            filter.is_empty()
                || v.name.to_lowercase().contains(&filter)
                || v.value.to_lowercase().contains(&filter)
        })
        .collect();

    let mut list = column![].spacing(2);
    for v in &matched {
        let shown = if reveal {
            elide(&v.value, MAX_VALUE_CHARS)
        } else {
            "••••••••".to_string()
        };
        let line = container(
            row![
                text(v.name.clone())
                    .size(12)
                    .font(Font::MONOSPACE)
                    .color(t.ink)
                    .width(Length::FillPortion(2)),
                text(shown)
                    .size(12)
                    .font(Font::MONOSPACE)
                    .color(t.muted)
                    .width(Length::FillPortion(3)),
            ]
            .spacing(12),
        )
        .padding([3, 6])
        .width(Length::Fill);
        list = list
            .push(mouse_area(line).on_press(Message::CopyText(format!("{}={}", v.name, v.value))));
    }

    let body: Element<'_, Message> = if state.env_vars.is_empty() {
        text(
            "Couldn't read this pane's environment — it shows once the shell is running. \
             Turn on Environment view in Shell settings for live, per-prompt updates.",
        )
        .size(12)
        .color(t.muted)
        .into()
    } else {
        scrollable(list).height(Length::Fill).into()
    };

    // Title bar (also the drag handle, see `place_env_popover`): the name, then the
    // Reveal toggle (expanded only), the expand/restore control, and close.
    let mut title_bar = row![section("Environment"), Space::new().width(Length::Fill)]
        .spacing(8)
        .align_y(iced::Alignment::Center);
    if expanded {
        title_bar = title_bar.push(toggle(
            "Reveal values",
            state.env_reveal,
            Message::ToggleEnvReveal,
        ));
    }
    title_bar = title_bar
        .push(button::ghost_compact(
            if expanded { "Collapse" } else { "Expand" },
            Message::ToggleEnvExpanded,
        ))
        .push(button::ghost_compact("×", Message::ToggleEnvView));

    let mut content = column![title_bar].spacing(12);
    if expanded {
        content = content
            .push(text_field("Filter…", &state.env_filter, Message::EnvFilterChanged).size(13));
    }
    content = content.push(body);
    // The source note (live hook vs launch-time OS read) is part of the full
    // experience, so it only shows when expanded.
    if expanded {
        if let Some(note) = source_note(state.env_source, t.success, t.muted) {
            content = content.push(note);
        }
    }
    // Editing types into the running shell, so it's opt-in (Shell settings). When on,
    // an Add button opens the "Set a variable" modal; when off, the view is read-only.
    if editing {
        content = content.push(
            row![
                Space::new().width(Length::Fill),
                button::secondary("Add variable", Message::OpenEnvAdd),
            ]
            .align_y(iced::Alignment::Center),
        );
    }

    container(content)
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .padding(14)
        .style(move |_| container::Style {
            border: Border {
                color: t.hairline,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::background(t.surface)
        })
        .into()
}

/// A one-line note on where the list came from: the live shell hook (updates each
/// prompt) or a launch-time snapshot read from the OS (static until the hook is on).
/// `None` (nothing to show) renders no line — the body message covers that case.
fn source_note<'a>(
    src: EnvSource,
    live: iced::Color,
    muted: iced::Color,
) -> Option<Element<'a, Message>> {
    match src {
        EnvSource::Hook => Some(
            text("Live — the shell reports its environment each prompt.")
                .size(11)
                .color(live)
                .into(),
        ),
        EnvSource::Process => Some(
            text(
                "Launch-time snapshot from the OS. Turn on Environment view in Shell \
                 settings for live updates as you export.",
            )
            .size(11)
            .color(muted)
            .into(),
        ),
        EnvSource::None => None,
    }
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}
