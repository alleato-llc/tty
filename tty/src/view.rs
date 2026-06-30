use iced::widget::{container, text, Column};
use iced::{Element, Length};

use rime::theme;
use rime::widgets::{status_bar, tabs, Tab};

use crate::message::Message;
use crate::state::Tty;

pub fn view(state: &Tty) -> Element<'_, Message> {
    // Open the active theme's palette for this render pass (RAII, drops at end).
    let _scope = theme::enter(state.theme.palette);
    let t = theme::tokens();
    let style = state.theme.terminal;
    let bg = style.bg;

    let mut root = Column::new().width(Length::Fill).height(Length::Fill);

    // Tab strip — shown once there's more than one terminal. Clicking the empty area
    // past the last tab opens a new one (also ⌘T).
    if state.tabs.len() > 1 {
        let models: Vec<Tab> = state
            .tabs
            .iter()
            .map(|term| Tab::new(term.title.clone()))
            .collect();
        root = root.push(tabs(
            models,
            state.active,
            state.hovered_tab,
            Message::ActivateTab,
            Message::CloseTab,
            Message::HoverTab,
            |_| Message::Tick, // no per-tab context menu yet
            Message::NewTab,
        ));
    }

    // The active terminal.
    let body: Element<'_, Message> = match state.active_term() {
        Some(term) => container(phosphor::terminal(
            term.screen.clone(),
            style,
            state.font,
            state.font_size,
            true,
            Message::Resize,
            Message::Select,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(6)
        .style(move |_| container::background(bg))
        .into(),
        None => container(text("no terminal").color(t.muted))
            .padding(6)
            .into(),
    };
    root = root.push(body);

    // Status bar: shell name on the left, grid + tab count + font on the right.
    let (left, right) = status_text(state);
    root = root.push(status_bar(&left, &right));

    container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::background(bg))
        .into()
}

fn status_text(state: &Tty) -> (String, String) {
    let Some(term) = state.active_term() else {
        return (String::new(), String::new());
    };
    let (cols, rows) = {
        let s = term.screen.lock();
        (s.cols, s.rows)
    };
    let tabs = if state.tabs.len() > 1 {
        format!(" · {} tabs", state.tabs.len())
    } else {
        String::new()
    };
    (
        term.title.clone(),
        format!("{cols}×{rows}{tabs} · {}px", state.font_size as u32),
    )
}
