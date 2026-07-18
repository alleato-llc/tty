//! The **Env view** panel: a masked, filterable list of the active pane's environment
//! variables, click a row to copy `NAME=value`. Data comes from [`crate::env`] (the
//! per-session capture file the shell-integration hook writes).

use iced::widget::{column, container, mouse_area, row, scrollable, text, Space};
use iced::{Element, Font, Length};
use rime::theme;
use rime::widgets::{button, modal_sized, section, stat, text_field, toggle};

use crate::message::Message;
use crate::state::Tty;

/// Longest revealed value shown inline (the full value is still what a click copies).
const MAX_VALUE_CHARS: usize = 120;

pub(super) fn env_panel_view<'a>(
    state: &'a Tty,
    base: Element<'a, Message>,
) -> Element<'a, Message> {
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
        // Click a row to copy the whole `NAME=value` (the real value, even when masked).
        list = list
            .push(mouse_area(line).on_press(Message::CopyText(format!("{}={}", v.name, v.value))));
    }

    let body: Element<'_, Message> = if state.env_vars.is_empty() {
        text(
            "No environment captured yet. This needs shell integration (Shell settings); \
             once it's on, run any command and it'll appear here.",
        )
        .size(12)
        .color(t.muted)
        .into()
    } else {
        scrollable(list).height(Length::Fixed(360.0)).into()
    };

    let content = column![
        row![
            section("Environment"),
            Space::new().width(Length::Fill),
            toggle("Reveal values", state.env_reveal, Message::ToggleEnvReveal),
            button::ghost("Close", Message::ToggleEnvView),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        stat("Variables", matched.len().to_string()),
        text_field("Filter…", &state.env_filter, Message::EnvFilterChanged).size(13),
        text("Click a variable to copy NAME=value.")
            .size(11)
            .color(t.muted),
        body,
    ]
    .spacing(12);

    modal_sized(base, content, Message::ToggleEnvView, 720.0)
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}
