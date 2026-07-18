//! The **Env view** — a popover (like the metric charts): non-modal, drag it by
//! anywhere on the card, border-resize it, and it stays open beside the terminal so
//! it tracks the shell as commands run. A masked, filterable list of the active
//! pane's environment; click a row to copy `NAME=value`. Data comes from
//! [`crate::env`]; the draggable/resizable chrome is [`rime::widgets::popover`].

use iced::widget::{column, container, mouse_area, row, scrollable, stack, text, Space};
use iced::{Border, Element, Font, Length, Padding};
use rime::theme;
use rime::widgets::{button, popover, section, stat, text_field, toggle};

use crate::message::Message;
use crate::state::Tty;

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

fn card<'a>(state: &'a Tty, w: f32, h: f32) -> Element<'a, Message> {
    let t = theme::tokens();
    let filter = state.env_filter.to_lowercase();
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
        let shown = if state.env_reveal {
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

    let body: Element<'_, Message> = if !state.settings.shell_integration().env_view {
        text(
            "The Environment view is off. Turn it on in Shell settings — it captures the \
             shell's env each prompt (and takes effect on shells started after).",
        )
        .size(12)
        .color(t.muted)
        .into()
    } else if state.env_vars.is_empty() {
        text(
            "No environment captured yet. Run any command in this pane and it'll appear \
             here (needs the shell-integration hooks).",
        )
        .size(12)
        .color(t.muted)
        .into()
    } else {
        scrollable(list).height(Length::Fill).into()
    };

    // The whole card is the drag handle (see `place_env_popover`); the reveal toggle +
    // close × keep their own hit areas.
    let title_bar = row![
        section("Environment"),
        Space::new().width(Length::Fill),
        toggle("Reveal values", state.env_reveal, Message::ToggleEnvReveal),
        button::ghost("×", Message::ToggleEnvView),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let content = column![
        title_bar,
        row![
            stat("Variables", matched.len().to_string()),
            Space::new().width(Length::Fill),
            text("Click a row to copy NAME=value")
                .size(11)
                .color(t.muted),
        ]
        .align_y(iced::Alignment::Center),
        text_field("Filter…", &state.env_filter, Message::EnvFilterChanged).size(13),
        body,
    ]
    .spacing(12);

    // Editing types into the running shell, so it's opt-in (Shell settings). When off,
    // the view stays read-only (see + copy) and no footer shows.
    let content = if state.settings.shell_integration().env_editing {
        content.push(
            column![
                text("Set in this pane — types at the shell's prompt:")
                    .size(11)
                    .color(t.muted),
                row![
                    text_field("NAME", &state.env_set_name, Message::EnvSetNameChanged).size(13),
                    text_field("value", &state.env_set_value, Message::EnvSetValueChanged).size(13),
                    button::secondary("Set", Message::EnvInjectSet),
                    button::ghost("Unset", Message::EnvInjectUnset),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(6),
        )
    } else {
        content
    };

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

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}
