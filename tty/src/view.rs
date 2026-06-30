use iced::widget::{column, container, row, scrollable, text, Column};
use iced::{Element, Length};

use rime::theme;
use rime::widgets::{
    button, color_field, labeled, section, select, slider, status_bar, stepper, tabs, text_field,
    tooltip, Tab, TooltipPosition,
};

use crate::message::Message;
use crate::state::Tty;

/// The find bar's text-input id (so `⌘F` can focus it).
pub fn search_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("tty-search")
}

pub fn view(state: &Tty) -> Element<'_, Message> {
    // Unfocused-window transparency: fade every surface + text by the same factor so
    // the whole window goes translucent uniformly (opaque while focused / by default).
    let op = state.window_opacity();
    // Open the (faded) theme palette for this render pass (RAII, drops at end).
    let _scope = theme::enter(crate::theme::fade_palette(state.theme.palette, op));
    let t = theme::tokens();
    let style = crate::theme::fade_style(state.theme.terminal, op);
    let bg = style.bg;

    let mut root = Column::new().width(Length::Fill).height(Length::Fill);

    // Tab strip — shown once there's more than one terminal. Clicking the empty area
    // past the last tab opens a new one (also ⌘T).
    if state.tabs.len() > 1 {
        let models: Vec<Tab> = state
            .tabs
            .iter()
            .map(|term| {
                // Prefer the program-set title (OSC 0/2); a • marks unseen activity.
                let title = term
                    .screen
                    .lock()
                    .title
                    .clone()
                    .unwrap_or_else(|| term.title.clone());
                Tab::new(if term.activity {
                    format!("• {title}")
                } else {
                    title
                })
            })
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
        Some(term) => container(
            phosphor::terminal(
                term.screen.clone(),
                style,
                state.font,
                state.font_size,
                true,
                Message::Resize,
                Message::Select,
                Message::PtyBytes,
            )
            .find(state.search.clone()),
        )
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

    // Find bar (⌘F): a focused field whose text highlights matches in the terminal.
    if let Some(query) = &state.search {
        let field = text_field("Find in scrollback…", query, Message::SearchChanged)
            .id(search_id())
            .on_submit(Message::SearchSubmit)
            .size(13);
        root = root.push(
            container(field)
                .padding([4, 6])
                .width(Length::Fill)
                .style(move |_| container::background(t.surface)),
        );
    }

    // Status bar: shell name on the left, grid + tab count + font on the right.
    let (left, right) = status_text(state);
    root = root.push(status_bar(&left, &right));

    let chrome = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::background(bg));

    // The settings panel floats over the terminal when ⌘, is open.
    if state.show_settings {
        rime::widgets::settings(
            chrome,
            &["Appearance", "Palette"],
            state.settings_section,
            Message::SettingsSection,
            settings_body(state),
            None,
            Message::ToggleSettings,
        )
    } else {
        chrome.into()
    }
}

/// The body of the active settings section.
fn settings_body(state: &Tty) -> Element<'_, Message> {
    match state.settings_section {
        1 => palette_section(state),
        _ => appearance_section(state),
    }
}

/// Appearance: named theme, font family, font size.
fn appearance_section(state: &Tty) -> Element<'_, Message> {
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
        section("Appearance"),
        labeled("Theme", theme_pick),
        labeled("Font", font_pick),
        stepper(
            "Font size",
            format!("{}px", state.font_size as u32),
            Message::FontSizeStep(-1.0),
            Message::FontSizeStep(1.0),
        ),
        // Transparency that kicks in only when the window loses focus. Shown as a
        // 0–95% transparency amount; stored as the resulting opacity (1 − amount).
        section("Window"),
        {
            let transparency = 1.0 - state.settings.unfocused_opacity();
            let max = 1.0 - crate::settings::MIN_OPACITY;
            let control = slider(
                "Transparency On Blur",
                0.0..=max,
                transparency,
                format!("{}%", (transparency * 100.0).round() as i32),
                |t| Message::SetUnfocusedOpacity(1.0 - t),
            );
            tooltip(
                control,
                "Fades the whole window when it loses focus, so what's behind it \
                 shows through. At 0% it stays opaque. The window is always solid \
                 while focused.",
                TooltipPosition::Top,
            )
        },
    ]
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

fn status_text(state: &Tty) -> (String, String) {
    let Some(term) = state.active_term() else {
        return (String::new(), String::new());
    };
    let (cols, rows, title) = {
        let s = term.screen.lock();
        (s.cols, s.rows, s.title.clone())
    };
    let tabs = if state.tabs.len() > 1 {
        format!(" · {} tabs", state.tabs.len())
    } else {
        String::new()
    };
    (
        title.unwrap_or_else(|| term.title.clone()),
        format!("{cols}×{rows}{tabs} · {}px", state.font_size as u32),
    )
}
