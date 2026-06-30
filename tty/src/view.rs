use iced::widget::{
    column, container, mouse_area, pane_grid, row, scrollable, text, Column, Space,
};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, color_field, context_menu, labeled, section, select, slider, status_bar, stepper, tabs,
    text_field, toggle, tooltip, MenuItem, Tab, TabBarStyle, TooltipPosition,
};

use crate::message::Message;
use crate::state::Tty;

/// The find bar's text-input id (so `⌘F` can focus it).
pub fn search_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("tty-search")
}

/// The rename field's text-input id (so "Rename tab" can focus it).
pub fn rename_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("tty-rename")
}

/// The daemon's per-window view: a detached window shows just its tab; every other
/// window is the full tabbed chrome.
pub fn root_view(state: &Tty, window: iced::window::Id) -> Element<'_, Message> {
    match state.detached.get(&window) {
        Some(tab) => detached_view(state, window, tab),
        None => main_view(state),
    }
}

/// The daemon's per-window title: a detached window takes its tab's label.
pub fn title(state: &Tty, window: iced::window::Id) -> String {
    match state.detached.get(&window) {
        Some(tab) => tab.label(),
        None => "tty".to_string(),
    }
}

/// Render the full tabbed chrome (the main window; detached windows use
/// [`detached_view`]).
fn main_view(state: &Tty) -> Element<'_, Message> {
    // Unfocused-window transparency: fade every surface + text by the same factor so
    // the whole window goes translucent uniformly (opaque while focused / by default).
    let op = state.window_opacity();
    // Open the (faded) theme palette for this render pass (RAII, drops at end).
    let _scope = theme::enter(crate::theme::fade_palette(state.theme.palette, op));
    let t = theme::tokens();
    let style = crate::theme::fade_style(state.theme.terminal, op);
    let bg = style.bg;

    let mut root = Column::new().width(Length::Fill).height(Length::Fill);

    // Tab strip — shown only when there's more than one tab (matching fed / fed-ide):
    // a lone tab carries no strip. Tab actions (rename / close / split-by-tab) live on
    // the strip's right-click menu; a single pane is still right-clickable for a split.
    // Clicking the empty area past the last tab opens a new one (also ⌘T).
    if state.tabs.len() > 1 {
        let models: Vec<Tab> = state
            .tabs
            .iter()
            .map(|tab| {
                // A user-set name, else the focused pane's program/shell title; a •
                // marks unseen activity in any of the tab's panes.
                let title = tab.label();
                let activity = tab.panes.iter().any(|(_, t)| t.activity);
                Tab::new(if activity {
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
            Message::TabRightClick, // right-click a tab → split context menu
            Message::NewTab,
            TabBarStyle {
                highlight_active: state.settings.tab_highlight(),
                text_size: 12.0,
            },
        ));
    }

    // Rename bar (from the tab menu): a focused field, prefilled with the current name.
    // Enter commits, Esc cancels.
    if let Some((_, draft)) = &state.renaming {
        let field = text_field("Tab name…", draft, Message::RenameChanged)
            .id(rename_id())
            .on_submit(Message::RenameSubmit)
            .size(13);
        root = root.push(
            container(
                row![text("Rename tab").size(12).color(t.muted), field,]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            )
            .padding([4, 6])
            .width(Length::Fill)
            .style(move |_| container::background(t.surface)),
        );
    }

    // The active tab's panes. A tab is a single pane until the user splits it; the
    // `pane_grid` lays the split tree out, drags its dividers, and reports focus clicks.
    let accent = t.accent;
    let hairline = t.hairline;
    // The pane closures emit window-tagged messages so a click/resize/selection routes to
    // the right tab (`pane_grid::Pane` ids collide across tabs). Headless renders set no
    // main window, so synthesize a throwaway id there.
    let win = state.main_window.unwrap_or_else(iced::window::Id::unique);
    let body: Element<'_, Message> = match state.tabs.get(state.active) {
        Some(tab) => {
            let focus = tab.focus;
            let window_focused = state.focused;
            let font = state.font;
            let size = state.font_size;
            let search = state.search.clone();
            // A focus border only earns its keep when there's more than one pane to tell
            // apart — a single pane shows none (no stray accent rectangle).
            let multi = tab.panes.len() > 1;
            pane_grid(&tab.panes, move |pane, term, _maximized| {
                let is_focused = pane == focus && window_focused;
                let term_widget = phosphor::terminal(
                    term.screen.clone(),
                    style,
                    font,
                    size,
                    is_focused,
                    move |c, r| Message::Resize(win, pane, c, r),
                    move |sel| Message::Select(win, pane, sel),
                    move |b| Message::PtyBytes(win, pane, b),
                )
                .find(search.clone());
                // When split, an accent border marks the focused pane so it's clear where
                // typing goes; the others get a hairline.
                let border_color = if is_focused { accent } else { hairline };
                let bordered = container(term_widget).padding(6).style(move |_| {
                    let border = if multi {
                        Border {
                            color: border_color,
                            width: 1.0,
                            radius: 0.0.into(),
                        }
                    } else {
                        Border::default()
                    };
                    container::Style {
                        border,
                        ..container::background(bg)
                    }
                });
                // Right-click anywhere in the pane opens the split context menu over it.
                pane_grid::Content::new(
                    mouse_area(bordered).on_right_press(Message::PaneRightClick(pane)),
                )
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .spacing(4)
            .on_click(move |pane| Message::FocusPane(win, pane))
            .on_resize(8, move |e| Message::ResizeSplit(win, e))
            .into()
        }
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
    let base: Element<'_, Message> = if state.show_settings {
        rime::widgets::settings(
            chrome,
            &["Appearance", "Palette", "Keys"],
            state.settings_section,
            Message::SettingsSection,
            settings_body(state),
            None,
            Message::ToggleSettings,
        )
    } else {
        chrome.into()
    };

    // The right-click context menu floats above everything, anchored at the click. A
    // tab's menu adds tab actions (new / close tab); a pane's adds "close pane". Both
    // split the active tab's focused pane.
    if let Some((kind, at)) = state.menu {
        use crate::state::MenuKind;
        use iced::widget::pane_grid::Direction;
        let mut items: Vec<MenuItem<Message>> = Vec::new();
        // A tab menu leads with tab actions; both kinds carry the four split directions.
        if kind == MenuKind::Tab {
            items.push(MenuItem::shortcut("New tab", "⌘T", Message::NewTab));
            items.push(MenuItem::action(
                "Rename tab…",
                Message::StartRename(state.active),
            ));
            items.push(MenuItem::action(
                "Detach Tab",
                Message::DetachTab(state.active),
            ));
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::shortcut(
            "Split left",
            "⌥⌘←",
            Message::Split(Direction::Left),
        ));
        items.push(MenuItem::shortcut(
            "Split right",
            "⌥⌘→",
            Message::Split(Direction::Right),
        ));
        items.push(MenuItem::shortcut(
            "Split up",
            "⌥⌘↑",
            Message::Split(Direction::Up),
        ));
        items.push(MenuItem::shortcut(
            "Split down",
            "⌥⌘↓",
            Message::Split(Direction::Down),
        ));
        items.push(MenuItem::separator());
        // …and close the right thing: the whole tab, or just the pane.
        match kind {
            MenuKind::Tab => items.push(MenuItem::shortcut(
                "Close tab",
                "⌘W",
                Message::CloseTab(state.active),
            )),
            MenuKind::Pane => {
                items.push(MenuItem::shortcut("Close pane", "⌘W", Message::ClosePane))
            }
        }
        context_menu(base, &items, at, Message::CloseMenu)
    } else {
        base
    }
}

/// A detached tab in its own window (ADR 0003): just the tab's pane tree, a slim strip
/// with a **Reattach** button, and a status bar — no tab strip / settings / find / pane
/// menu (those chrome affordances live only in the main window). Splitting still works
/// via the ⌥⌘-arrow chords, which route to the focused window.
fn detached_view<'a>(
    state: &'a Tty,
    window: iced::window::Id,
    tab: &'a crate::state::Tab,
) -> Element<'a, Message> {
    let op = state.window_opacity();
    let _scope = theme::enter(crate::theme::fade_palette(state.theme.palette, op));
    let t = theme::tokens();
    let style = crate::theme::fade_style(state.theme.terminal, op);
    let bg = style.bg;
    let accent = t.accent;
    let hairline = t.hairline;

    let focus = tab.focus;
    let window_focused = state.focused_window == Some(window);
    let font = state.font;
    let size = state.font_size;
    let multi = tab.panes.len() > 1;

    let body = pane_grid(&tab.panes, move |pane, term, _maximized| {
        let is_focused = pane == focus && window_focused;
        let term_widget = phosphor::terminal(
            term.screen.clone(),
            style,
            font,
            size,
            is_focused,
            move |c, r| Message::Resize(window, pane, c, r),
            move |sel| Message::Select(window, pane, sel),
            move |b| Message::PtyBytes(window, pane, b),
        )
        .find(None);
        let border_color = if is_focused { accent } else { hairline };
        let bordered = container(term_widget).padding(6).style(move |_| {
            let border = if multi {
                Border {
                    color: border_color,
                    width: 1.0,
                    radius: 0.0.into(),
                }
            } else {
                Border::default()
            };
            container::Style {
                border,
                ..container::background(bg)
            }
        });
        pane_grid::Content::new(bordered)
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .spacing(4)
    .on_click(move |pane| Message::FocusPane(window, pane))
    .on_resize(8, move |e| Message::ResizeSplit(window, e));

    // Slim strip: the tab name on the left, a Reattach button on the right.
    let strip = container(
        row![
            text(tab.label()).size(13).color(t.ink),
            Space::new().width(Length::Fill),
            button::ghost("Reattach", Message::ReattachTab(window)),
        ]
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4, 10])
    .style(move |_| container::background(t.surface));

    let (cols, rows) = tab
        .focused()
        .map(|term| {
            let s = term.screen.lock();
            (s.cols, s.rows)
        })
        .unwrap_or((0, 0));
    let status = format!("{cols}×{rows} · {}px", size as u32);

    container(column![
        strip,
        container(body).width(Length::Fill).height(Length::Fill),
        status_bar(&tab.label(), &status),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::background(bg))
    .into()
}

/// The body of the active settings section.
fn settings_body(state: &Tty) -> Element<'_, Message> {
    match state.settings_section {
        1 => palette_section(state),
        2 => keys_section(),
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
                ("wheel", "Scroll back through history"),
                ("⌘,", "Settings"),
            ],
        ),
        (
            "Find",
            &[("Enter", "Close the find bar"), ("Esc", "Cancel")],
        ),
    ];

    let t = theme::tokens();
    let mut body = Column::new().spacing(14);
    for (title, rows) in groups {
        let mut list = Column::new().spacing(6);
        for (chord, desc) in rows {
            list = list.push(
                row![
                    container(
                        text(*chord)
                            .size(12)
                            .color(t.ink)
                            .font(iced::Font::MONOSPACE)
                    )
                    .width(Length::Fixed(160.0)),
                    text(*desc).size(12).color(t.muted),
                ]
                .spacing(8),
            );
        }
        body = body.push(column![section(title), list].spacing(8));
    }

    scrollable(body.padding(iced::Padding::ZERO.right(8)))
        .height(Length::Fill)
        .into()
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
        // Tabs: dial how loud the active tab reads. Off swaps the accent ink for a
        // subtler normal-ink emphasis (it still beats the muted inactive tabs).
        section("Tabs"),
        toggle(
            "Highlight active tab",
            state.settings.tab_highlight(),
            Message::SetTabHighlight(!state.settings.tab_highlight()),
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
    let Some(tab) = state.tabs.get(state.active) else {
        return (String::new(), String::new());
    };
    let (cols, rows) = match tab.focused() {
        Some(term) => {
            let s = term.screen.lock();
            (s.cols, s.rows)
        }
        None => (0, 0),
    };
    let tabs = if state.tabs.len() > 1 {
        format!(" · {} tabs", state.tabs.len())
    } else {
        String::new()
    };
    let panes = if tab.panes.len() > 1 {
        format!(" · {} panes", tab.panes.len())
    } else {
        String::new()
    };
    (
        tab.label(),
        format!("{cols}×{rows}{tabs}{panes} · {}px", state.font_size as u32),
    )
}
