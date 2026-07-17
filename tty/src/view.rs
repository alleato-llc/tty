use iced::widget::{container, mouse_area, opaque, pane_grid, row, text, Column};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, context_menu, dialog, rename_bar, rename_field_id, tabs, text_field, window_shell,
    MenuItem, Tab, TabBarStyle,
};

use crate::message::Message;
use crate::state::{Term, Tty};

mod metrics;
mod popover;
mod procs;
mod scrollback;
mod settings;
mod settings_history;
mod status_bar;
mod util;
use popover::{metric_pane_content, metric_popover_card, place_metric_popover};
use scrollback::{age_from_epoch_ms, scrollback_panel_view};
use settings::settings_body;
use settings_history::passphrase_prompt_view;
pub use status_bar::status_bar_scroll_max;
use status_bar::{status_bar_metrics_editor, status_bar_view};
use util::*;

/// The `⌘F` matches in `term`'s buffer for `query` (empty when there's no active
/// query) — shared by the find bar's "N of M" label and the per-pane `scroll_to`.
fn current_matches(term: &Term, query: &str) -> Vec<(usize, usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let screen = term.screen.lock();
    phosphor::find_matches(&screen, screen.cols, query)
}

/// The absolute line of the currently-selected match (`search_match`, wrapped modulo
/// the live match count), for `.scroll_to`.
fn current_match_line(term: &Term, query: &str, search_match: i64) -> Option<usize> {
    let matches = current_matches(term, query);
    if matches.is_empty() {
        return None;
    }
    let idx = search_match.rem_euclid(matches.len() as i64) as usize;
    Some(matches[idx].0)
}

/// The find bar's text-input id (so `⌘F` can focus it).
pub fn search_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("tty-search")
}

/// The rename field's text-input id (so "Rename tab" can focus it) — the shared rime
/// rename bar owns the field, so we delegate to its id.
pub fn rename_id() -> iced::advanced::widget::Id {
    rename_field_id()
}

/// The daemon's per-window view: a detached window shows just its tab; every other
/// window is the full tabbed chrome.
pub fn root_view(state: &Tty, window: iced::window::Id) -> Element<'_, Message> {
    // Set the macOS Dock icon on the first render (post-launch, main thread — so it
    // sticks; a call in `main` before the event loop is reset by winit). Once-guarded.
    crate::app_icon::ensure_dock_icon(crate::APP_ICON);

    match state.detached.get(&window) {
        Some(tab) => detached_view(state, window, tab),
        None => main_view(state),
    }
}

/// The daemon's per-window title: a detached window takes its tab's label.
/// An untracked tab says so in the title, so the promise stays visible even
/// with the tab strip hidden (a single tab) or detached.
pub fn title(state: &Tty, window: iced::window::Id) -> String {
    match state.detached.get(&window) {
        Some(tab) if tab.untracked => format!("{} — Untracked", tab.label()),
        Some(tab) => tab.label(),
        None => {
            if state.tabs.get(state.active).is_some_and(|t| t.untracked) {
                "tty — Untracked".to_string()
            } else {
                "tty".to_string()
            }
        }
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
                // marks unseen activity in any of the tab's panes, and a ○ marks an
                // untracked tab (commands never saved to encrypted history).
                let mut title = tab.label();
                if tab.untracked {
                    title = format!("○ {title}");
                }
                let activity = tab.terms().any(|t| t.activity);
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

    // Rename bar (from the tab menu): the shared rime bar, a focused field prefilled
    // with the current name. Enter commits, Esc cancels.
    if let Some((_, draft)) = &state.renaming {
        root = root.push(rename_bar(
            "Rename tab",
            "Tab name…",
            draft,
            Message::RenameChanged,
            Message::RenameSubmit,
        ));
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
            let search_match = state.search_match;
            // A focus border only earns its keep when there's more than one pane to tell
            // apart — a single pane shows none (no stray accent rectangle).
            let multi = tab.panes.len() > 1;
            let highlight = state.settings.highlight_focused_pane();
            pane_grid(&tab.panes, move |pane, content, maximized| {
                let is_focused = pane == focus && window_focused;
                let term = match content {
                    crate::state::Pane::Term(term) => term,
                    // A graduated metric view (CPU chart, process table, …).
                    crate::state::Pane::Metric(kind) => {
                        return metric_pane_content(
                            state, *kind, win, pane, is_focused, maximized, multi, highlight,
                            accent, hairline, bg,
                        );
                    }
                };
                let scroll_to = search
                    .as_deref()
                    .and_then(|q| current_match_line(term, q, search_match));
                let term_widget = phosphor::terminal(
                    term.screen.clone(),
                    style,
                    font,
                    size,
                    is_focused,
                    move |c, r| Message::Resize(win, pane, c, r),
                    move |sel| Message::Select(win, pane, sel),
                    move |b| Message::PtyBytes(win, pane, b),
                    Message::LinkClick,
                    Message::OpenLink,
                    Message::OpenFile,
                )
                .find(search.clone())
                .scroll_to(scroll_to);
                // When split, an accent border marks the focused pane so it's clear where
                // typing goes (unless the highlight is off); the others get a hairline.
                let border_color = if is_focused && highlight {
                    accent
                } else {
                    hairline
                };
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
    // "Replace a pane" pick mode dims the grid and shows an instruction; the scrim
    // has no handlers, so clicks fall through it to the panes (which route to the
    // replace via `FocusPane`).
    let body = if let Some(kind) = state.pane_replace_pending {
        let hint = container(
            text(format!(
                "Click a pane to replace it with {kind}    ·    Esc to cancel"
            ))
            .size(13)
            .color(t.ink),
        )
        .padding([8, 14])
        .style(move |_| container::Style {
            border: Border {
                color: t.accent,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::background(t.surface)
        });
        let scrim = container(hint)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Top)
            .padding(24)
            .style(|_| {
                container::background(iced::Color {
                    a: 0.45,
                    ..iced::Color::BLACK
                })
            });
        iced::widget::stack![body, scrim].into()
    } else {
        body
    };
    root = root.push(body);

    // Find bar (⌘F): a focused field whose text highlights every match in the whole
    // buffer; Enter/⇧Enter step through them (an "N of M" count reads out which).
    if let Some(query) = &state.search {
        let field = text_field("Find in scrollback…", query, Message::SearchChanged)
            .id(search_id())
            .on_submit(Message::SearchSubmit)
            .size(13);
        let count = state
            .tabs
            .get(state.active)
            .and_then(|t| t.focused())
            .map(|term| current_matches(term, query).len())
            .filter(|&n| n > 0)
            .map(|total| {
                let current = (state.search_match.rem_euclid(total as i64)) as usize + 1;
                format!("{current} of {total}")
            })
            .unwrap_or_else(|| "No matches".to_string());
        root = root.push(
            container(
                row![field, text(count).size(12).color(t.muted)]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
            )
            .padding([4, 6])
            .width(Length::Fill)
            .style(move |_| container::background(t.surface)),
        );
    }

    // Status bar: shell name on the left, machine stats + grid/tab/font on the
    // right. Off entirely gives the terminal the full height. With auto-hide on
    // it floats over the bottom edge (revealed when the pointer nears it) instead
    // of taking a row in the column, so showing or hiding it never reflows the
    // pane grid underneath.
    let disabled = state.settings.status_bar_disabled();
    let autohide = state.settings.status_bar_autohide();
    if !disabled && !autohide {
        root = root.push(status_bar_view(state));
    }

    let chrome = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::background(bg));

    let chrome: Element<'_, Message> = if !disabled && autohide && state.status_bar_revealed() {
        iced::widget::stack![
            chrome,
            container(status_bar_view(state))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::alignment::Vertical::Bottom),
        ]
        .into()
    } else {
        chrome.into()
    };

    // The metric drill-in popovers float over the status bar. Each card starts a
    // drag-move when its body is pressed (`opaque` stops that press from falling
    // through); its edges/controls take their own press first. In the default
    // one-at-a-time mode a transparent full-window layer beneath the single card
    // closes it on a click away. When pinned, several cards stack (cascaded so
    // they don't fully overlap) with no click-away closer — each is dismissed by
    // its own "×" or Escape.
    let mut chrome = chrome;
    let pinned = state.settings.status_bar_metrics_pinned();
    for (i, pop) in state.metric_details.iter().enumerate() {
        let card = metric_popover_card(state, pop, i);
        let draggable = opaque(mouse_area(card).on_press(Message::MetricDetailMoveStart(i)));
        let anchored = place_metric_popover(state, pop, i, draggable);
        chrome = if pinned {
            iced::widget::stack![chrome, anchored].into()
        } else {
            iced::widget::stack![
                chrome,
                opaque(mouse_area(anchored).on_press(Message::CloseMetricDetail)),
            ]
            .into()
        };
    }

    // The settings panel floats over the terminal when ⌘, is open.
    let mut base: Element<'_, Message> = if state.show_settings {
        rime::widgets::settings(
            chrome,
            &["Appearance", "Palette", "Keys", "Metrics", "History"],
            state.settings_section,
            Message::SettingsSection,
            settings_body(state),
            None,
            Message::ToggleSettings,
        )
    } else {
        chrome
    };

    // The scrollback history panel floats over the chrome/settings when open — applied
    // *before* the context menu below, so a menu opened from within the panel (a row's
    // copy/clear) layers on top of it instead of being buried underneath.
    if state.show_scrollback {
        base = scrollback_panel_view(state, base);
    }

    // The right-click context menu floats above everything, anchored at the click. A
    // tab's menu adds tab actions (new / close tab); a pane's adds "close pane"; a
    // link's is just open/copy; a scrollback row's is copy/clear.
    if let Some((kind, at)) = &state.menu {
        use crate::state::MenuKind;
        use iced::widget::pane_grid::Direction;
        let at = *at;
        // Both Tab and Pane carry the four split directions; only their leading/
        // trailing items differ (tab actions + "close tab" vs. just "close pane").
        let split_items = || {
            vec![
                MenuItem::shortcut("Split left", "⌥⌘←", Message::Split(Direction::Left)),
                MenuItem::shortcut("Split right", "⌥⌘→", Message::Split(Direction::Right)),
                MenuItem::shortcut("Split up", "⌥⌘↑", Message::Split(Direction::Up)),
                MenuItem::shortcut("Split down", "⌥⌘↓", Message::Split(Direction::Down)),
            ]
        };
        let items: Vec<MenuItem<Message>> = match kind {
            MenuKind::Link(url) => vec![
                MenuItem::action("Open Link", Message::OpenLink(url.clone())),
                MenuItem::action("Copy Link", Message::CopyLink(url.clone())),
            ],
            MenuKind::Tab => {
                let mut items = vec![
                    MenuItem::shortcut("New tab", "⌘T", Message::NewTab),
                    MenuItem::shortcut("New Untracked Tab", "⌘⇧T", Message::NewUntrackedTab),
                    MenuItem::action("Rename tab…", Message::StartRename(state.active)),
                    MenuItem::action("Detach Tab", Message::DetachTab(state.active)),
                    MenuItem::separator(),
                ];
                items.extend(split_items());
                items.push(MenuItem::separator());
                items.push(MenuItem::shortcut(
                    "Close tab",
                    "⌘W",
                    Message::CloseTab(state.active),
                ));
                items
            }
            MenuKind::Pane => {
                let mut items = split_items();
                items.push(MenuItem::separator());
                items.push(MenuItem::shortcut(
                    "Clear Scrollback",
                    "⌘K",
                    Message::ClearScrollback,
                ));
                items.push(MenuItem::shortcut(
                    "View Scrollback History",
                    "⌘⇧H",
                    Message::ToggleScrollbackPanel,
                ));
                items.push(MenuItem::separator());
                items.push(MenuItem::shortcut("Close pane", "⌘W", Message::ClosePane));
                items
            }
            MenuKind::ScrollbackRow(target) => {
                let mut items = vec![
                    MenuItem::action("Copy", Message::CopyScrollbackTarget(target.clone())),
                    MenuItem::action("Clear", Message::ClearScrollbackTarget(target.clone())),
                ];
                // "Delete" (remove the whole entry) only applies to a command's
                // header row — there's no sensible "delete" for a single live
                // output line, just "Clear" (blank it). An archived row is
                // always command-level (output is never persisted), so it
                // always offers Delete.
                let offers_delete = match target {
                    crate::state::HistoryRowTarget::Live(
                        crate::state::ScrollbackTarget::Command { .. },
                    ) => true,
                    crate::state::HistoryRowTarget::Live(
                        crate::state::ScrollbackTarget::Output { .. },
                    ) => false,
                    crate::state::HistoryRowTarget::Archived(_) => true,
                };
                if offers_delete {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::action(
                        "Delete",
                        Message::DeleteScrollbackTarget(target.clone()),
                    ));
                }
                items
            }
            MenuKind::SettingsHistoryRow(target) => vec![
                MenuItem::action(
                    "Copy",
                    Message::CopyText(cathode::commands::strip_prompt(&target.command).to_string()),
                ),
                MenuItem::separator(),
                MenuItem::action(
                    "Delete…",
                    Message::RequestDeleteSettingsHistoryRow(target.clone()),
                ),
            ],
            MenuKind::ProcRow { pid, name } => vec![
                MenuItem::action("View Process", Message::OpenProcDetail(*pid)),
                MenuItem::separator(),
                MenuItem::action("Copy path", Message::CopyProcPath(*pid)),
                MenuItem::action("Copy PID", Message::CopyText(pid.to_string())),
                MenuItem::action("Copy name", Message::CopyText(name.clone())),
                MenuItem::separator(),
                // "Quit" is a polite SIGTERM (the process can clean up); "Force
                // Quit" is an uncatchable SIGKILL, so it confirms first.
                MenuItem::action("Quit", Message::KillProcess(*pid, crate::metrics::SIG_TERM)),
                MenuItem::action("Force Quit…", Message::RequestForceKill(*pid, name.clone())),
            ],
            MenuKind::FdRow { path } => {
                vec![MenuItem::action(
                    "Copy path",
                    Message::CopyText(path.clone()),
                )]
            }
            MenuKind::PromotePopover { kind } => {
                use iced::widget::pane_grid::Direction;
                let kind = *kind;
                vec![
                    MenuItem::action(
                        "Move to pane · Left",
                        Message::PromoteMetricToPane(kind, Direction::Left),
                    ),
                    MenuItem::action(
                        "Move to pane · Right",
                        Message::PromoteMetricToPane(kind, Direction::Right),
                    ),
                    MenuItem::action(
                        "Move to pane · Up",
                        Message::PromoteMetricToPane(kind, Direction::Up),
                    ),
                    MenuItem::action(
                        "Move to pane · Down",
                        Message::PromoteMetricToPane(kind, Direction::Down),
                    ),
                    MenuItem::separator(),
                    MenuItem::action("Replace a pane…", Message::StartPaneReplace(kind)),
                ]
            }
        };
        base = context_menu(base, &items, at, Message::CloseMenu);
    }

    // The archive browser's per-row delete confirmation — layered over the
    // settings panel (and under the whole-archive reset below, though the two
    // can't be armed at once: the reset button isn't reachable while the
    // browser is open).
    if let Some(target) = &state.confirm_delete_settings_row {
        let message = format!(
            "Permanently remove \"{}\" from the encrypted archive. This can't be undone.",
            elide(&target.command, 80),
        );
        base = dialog(
            base,
            "Delete this command from history?",
            &message,
            vec![
                button::ghost("Cancel", Message::CancelDeleteSettingsHistoryRow).into(),
                button::danger("Delete", Message::ConfirmDeleteSettingsHistoryRow).into(),
            ],
            Message::CancelDeleteSettingsHistoryRow,
        );
    }

    // The enable/unlock dialog floats over everything the settings panel
    // shows — including the startup unlock case, where it's the first thing
    // the user sees with the terminal ready behind it.
    if let Some(prompt) = &state.passphrase_prompt {
        base = passphrase_prompt_view(state, prompt, base);
    }

    // The startup chooser (`history_session_start == "ask"`): the first
    // thing an ask-configured launch shows. A backdrop click counts as
    // "Stay untracked" — the fail-closed answer.
    if state.show_session_start_prompt {
        base = dialog(
            base,
            "Record this session's commands?",
            "Encrypted history is on. Record this session to the encrypted \
             archive, or stay untracked — then nothing typed this session is \
             saved. (Configure this in Settings → History.)",
            vec![
                button::ghost("Stay untracked", Message::SessionStartChoice(false)).into(),
                button::primary("Record", Message::SessionStartChoice(true)).into(),
            ],
            Message::SessionStartChoice(false),
        );
    }

    // The reset-history confirmation sits above everything else — it's the
    // one destructive action in the app, so nothing (a menu, the settings
    // panel underneath it) should be clickable through it.
    if state.confirm_reset_history {
        base = dialog(
            base,
            "Reset encrypted history?",
            "This permanently deletes every persisted command history file — \
             every day's archive and the index. This can't be undone.",
            vec![
                button::ghost("Cancel", Message::CancelResetEncryptedHistory).into(),
                button::danger("Delete", Message::ConfirmResetEncryptedHistory).into(),
            ],
            Message::CancelResetEncryptedHistory,
        );
    }

    // Confirm before replacing a live shell pane with a metric view.
    if let Some((_, _, kind)) = state.pane_replace_confirm {
        base = dialog(
            base,
            "Replace this pane?",
            &format!(
                "This closes the terminal in this pane (ending its shell if still \
                 running) and replaces it with the {kind} view. Its scrollback is lost."
            ),
            vec![
                button::ghost("Cancel", Message::CancelPaneReplace).into(),
                button::danger("End & replace", Message::ConfirmPaneReplace).into(),
            ],
            Message::CancelPaneReplace,
        );
    }

    // Confirm a force-quit (SIGKILL) of a process from the Processes drill-in.
    if let Some((pid, name)) = &state.kill_confirm {
        base = dialog(
            base,
            "Force quit this process?",
            &format!(
                "Force quit {name} (pid {pid})? SIGKILL is immediate and uncatchable \
                 — the process can't save or clean up, and unsaved work is lost."
            ),
            vec![
                button::ghost("Cancel", Message::CancelForceKill).into(),
                button::danger("Force Quit", Message::ConfirmForceKill).into(),
            ],
            Message::CancelForceKill,
        );
    }

    base
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
    let highlight = state.settings.highlight_focused_pane();

    let body = pane_grid(&tab.panes, move |pane, content, maximized| {
        let is_focused = pane == focus && window_focused;
        let term = match content {
            crate::state::Pane::Term(term) => term,
            crate::state::Pane::Metric(kind) => {
                return metric_pane_content(
                    state, *kind, window, pane, is_focused, maximized, multi, highlight, accent,
                    hairline, bg,
                );
            }
        };
        let term_widget = phosphor::terminal(
            term.screen.clone(),
            style,
            font,
            size,
            is_focused,
            move |c, r| Message::Resize(window, pane, c, r),
            move |sel| Message::Select(window, pane, sel),
            move |b| Message::PtyBytes(window, pane, b),
            Message::LinkClick,
            Message::OpenLink,
            Message::OpenFile,
        )
        .find(None);
        let border_color = if is_focused && highlight {
            accent
        } else {
            hairline
        };
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

    let (cols, rows) = tab
        .focused()
        .map(|term| {
            let s = term.screen.lock();
            (s.cols, s.rows)
        })
        .unwrap_or((0, 0));
    let label = if tab.untracked {
        format!("○ {} — Untracked", tab.label())
    } else {
        tab.label()
    };
    let status = format!("{cols}×{rows} · {}px", size as u32);

    // The shared rime detached-window chrome: a title strip (name + Reattach) over the
    // pane tree (kept on the terminal background) over a status bar. The body carries the
    // terminal bg so the shell's window background never shows through the pane gaps.
    window_shell(
        &label,
        vec![button::ghost("Reattach", Message::ReattachTab(window)).into()],
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::background(bg)),
        &label,
        &status,
    )
}
