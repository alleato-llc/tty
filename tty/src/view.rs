use iced::widget::{
    column, container, mouse_area, opaque, pane_grid, row, scrollable, text, Column,
};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, caption, color_field, context_menu, dialog, labeled, line_chart, modal_sized,
    rename_bar, rename_field_id, section, select, shortcut_row, slider, sparkline, stat,
    status_bar_content, stepper, table, tabs, text_field, toggle, tooltip, window_shell, LineChart,
    MenuItem, Series, SparkSeries, Sparkline, Tab, TabBarStyle, TableColumn, TableMetrics,
    TooltipPosition,
};

use crate::history::crypto::Cipher;
use crate::message::Message;
use crate::state::{Term, Tty};

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
            pane_grid(&tab.panes, move |pane, term, _maximized| {
                let is_focused = pane == focus && window_focused;
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
                )
                .find(search.clone())
                .scroll_to(scroll_to);
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
    // right. With auto-hide on it floats over the bottom edge (revealed when
    // the pointer nears it) instead of taking a row in the column, so showing
    // or hiding it never reflows the pane grid underneath.
    let autohide = state.settings.status_bar_autohide();
    if !autohide {
        root = root.push(status_bar_view(state));
    }

    let chrome = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::background(bg));

    let chrome: Element<'_, Message> = if autohide && state.status_bar_revealed() {
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
            &["Appearance", "Palette", "Keys", "History"],
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
            Message::LinkClick,
            Message::OpenLink,
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

/// The scrollback history panel (⌘⇧H): the active pane's full buffered + on-screen
/// transcript as a scrollable read-only log, with its own filter (independent of
/// ⌘F) and a couple of buffered/age stats.
/// One row of the scrollback panel's flattened accordion table: a command's header
/// (always shown) or one of its output lines (shown only while expanded).
#[derive(Clone, Copy)]
enum ScrollbackRow {
    Header(usize),
    Output(usize, usize),
}

/// Where a [`ScrollbackCommand`] came from — the live in-memory `command_log`
/// (addressed by index, for the existing `TerminalScreen`-based Clear/Delete)
/// or the encrypted archive, paged in via [`Message::ScrollbackPageOlder`]
/// (addressed by stable date+id, since there is no in-memory `CommandEntry`
/// behind it — Clear/Delete go straight to the background writer instead).
enum RowOrigin {
    Live {
        /// This command's index into the *unfiltered* `command_log` — stable
        /// across the filter (unlike this entry's own position in
        /// `filtered`), so a right-click's `ScrollbackTarget` can locate it
        /// back in `command_log` for "Clear" (see `Tty::clear_scrollback_target`).
        log_index: usize,
    },
    Archived {
        date: chrono::NaiveDate,
        id: u32,
        started_at_epoch_ms: u64,
        pane_tag: String,
    },
}

/// One filtered command entry, with just what the panel needs to render it (cloned
/// out from behind the screen's lock, or from `Tty::scrollback_archived`, rather
/// than held across the whole render).
struct ScrollbackCommand {
    origin: RowOrigin,
    command: String,
    output: Vec<String>,
    truncated: bool,
    age: std::time::Duration,
    /// Recorded on an untracked screen — session-only, never persisted;
    /// badged on the header row. Archived rows are by definition tracked.
    untracked: bool,
}

/// How long ago an archived entry's wall-clock timestamp was, for the same
/// "Xs/Xm/Xh ago" label a live row gets from its `Instant`.
fn age_from_epoch_ms(epoch_ms: u64) -> std::time::Duration {
    let wall = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(epoch_ms);
    std::time::SystemTime::now()
        .duration_since(wall)
        .unwrap_or_default()
}

/// This row's `HistoryRowTarget` for the header (whole-command) case — shared
/// by the right-click handler regardless of where the row came from.
fn command_target(c: &ScrollbackCommand) -> crate::state::HistoryRowTarget {
    match &c.origin {
        RowOrigin::Live { log_index } => {
            crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Command {
                log_index: *log_index,
                text: c.command.clone(),
            })
        }
        RowOrigin::Archived {
            date,
            id,
            started_at_epoch_ms,
            pane_tag,
        } => crate::state::HistoryRowTarget::Archived(crate::state::ArchivedTarget {
            date: *date,
            id: *id,
            started_at_epoch_ms: *started_at_epoch_ms,
            pane_tag: pane_tag.clone(),
            command: c.command.clone(),
        }),
    }
}

fn scrollback_panel_view<'a>(state: &'a Tty, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(term) = state.active_term() else {
        return base;
    };
    // Archived rows (paged in from the encrypted archive) render first —
    // oldest overall — followed by the live window, matching how
    // `TerminalScreen::seed_command_log` orders them at startup.
    let archived_commands = state.scrollback_archived.iter().map(|e| ScrollbackCommand {
        origin: RowOrigin::Archived {
            date: crate::history::local_date_from_epoch_ms(e.started_at_epoch_ms),
            id: e.id,
            started_at_epoch_ms: e.started_at_epoch_ms,
            pane_tag: e.pane_tag.clone(),
        },
        command: e.command.clone(),
        output: Vec::new(),
        truncated: false,
        age: age_from_epoch_ms(e.started_at_epoch_ms),
        untracked: false,
    });
    let live_commands = {
        let screen = term.screen.lock();
        screen
            .command_log
            .iter()
            .enumerate()
            .map(|(log_index, e)| ScrollbackCommand {
                origin: RowOrigin::Live { log_index },
                command: e.command.clone(),
                output: e.output.clone(),
                truncated: e.is_truncated(),
                age: e.started_at.elapsed(),
                untracked: e.untracked,
            })
            .collect::<Vec<_>>()
    };
    let commands: Vec<ScrollbackCommand> = archived_commands.chain(live_commands).collect();

    let query = state.scrollback_query.to_lowercase();
    let filtered: Vec<ScrollbackCommand> = commands
        .into_iter()
        .filter(|c| {
            query.is_empty()
                || c.command.to_lowercase().contains(&query)
                || c.output.iter().any(|l| l.to_lowercase().contains(&query))
        })
        .collect();
    let shown = filtered.len();

    // Flatten commands + (if expanded) their output into one row list the table
    // renders directly — the accordion effect is just which rows exist this render,
    // no variable-height-row support needed from the table widget itself.
    let mut rows = Vec::new();
    for (i, c) in filtered.iter().enumerate() {
        rows.push(ScrollbackRow::Header(i));
        if state.scrollback_expanded.contains(&i) {
            rows.extend((0..c.output.len()).map(|j| ScrollbackRow::Output(i, j)));
        }
    }
    let filtered = std::rc::Rc::new(filtered);
    let rows = std::rc::Rc::new(rows);
    let expanded = state.scrollback_expanded.clone();
    let row_count = rows.len();

    let cell_rows = rows.clone();
    let cell_filtered = filtered.clone();
    let select_rows = rows.clone();
    let activate_rows = rows.clone();
    let activate_filtered = filtered.clone();
    let right_click_rows = rows.clone();
    let right_click_filtered = filtered.clone();

    // A single "Line" column, monospace so terminal output stays aligned. A header
    // row toggles its own expand state on click; an output row selects/highlights on
    // click and copies to the clipboard on double-click (a header's own text can also
    // be double-click-copied).
    let log_table = table(
        row_count,
        vec![TableColumn::fill("Line")],
        move |row, _col| match cell_rows[row] {
            ScrollbackRow::Header(i) => {
                let c = &cell_filtered[i];
                let arrow = if expanded.contains(&i) { "▼" } else { "▶" };
                let count = if c.truncated {
                    format!("{}+ lines", c.output.len())
                } else {
                    format!("{} lines", c.output.len())
                };
                let badge = if c.untracked { " · untracked" } else { "" };
                format!(
                    "{arrow} {}  · {count} · {}{badge}",
                    c.command,
                    format_age(c.age)
                )
            }
            ScrollbackRow::Output(i, j) => format!("    {}", cell_filtered[i].output[j]),
        },
    )
    .metrics(TableMetrics {
        row_height: 18.0,
        header_height: 0.0,
    })
    .offset(state.scrollback_scroll)
    .selected(state.scrollback_selected)
    .font(iced::Font::MONOSPACE)
    .on_scroll(Message::ScrollbackScrolled)
    .on_select(move |row| match select_rows[row] {
        ScrollbackRow::Header(i) => Message::ScrollbackToggleExpand(i),
        ScrollbackRow::Output(..) => Message::ScrollbackRowSelected(row),
    })
    .on_activate(move |row| {
        // Double-click copies: a command without its captured prompt, an
        // output line verbatim.
        let text = match activate_rows[row] {
            ScrollbackRow::Header(i) => {
                cathode::commands::strip_prompt(&activate_filtered[i].command).to_string()
            }
            ScrollbackRow::Output(i, j) => activate_filtered[i].output[j].clone(),
        };
        Message::ScrollbackRowActivated(row, text)
    })
    .on_right_click(move |row| {
        let target = match right_click_rows[row] {
            ScrollbackRow::Header(i) => command_target(&right_click_filtered[i]),
            ScrollbackRow::Output(i, j) => {
                // Only a live-origin command ever has output rows to click —
                // an archived entry's `output` is always empty (never
                // persisted), so it never contributes an `Output` row.
                let c = &right_click_filtered[i];
                let log_index = match c.origin {
                    RowOrigin::Live { log_index } => log_index,
                    RowOrigin::Archived { .. } => unreachable!(
                        "an archived row's output is always empty, so it never produces an Output row"
                    ),
                };
                crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Output {
                    log_index,
                    line: j,
                    text: c.output[j].clone(),
                })
            }
        };
        Message::ScrollbackRowRightClick(row, target)
    })
    .width(Length::Fill)
    .height(Length::Fixed(380.0));

    let mut content = column![
        row![
            section("Scrollback History"),
            iced::widget::Space::new().width(Length::Fill),
            button::danger("Clear", Message::ClearScrollback),
            button::ghost("Close", Message::ToggleScrollbackPanel),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        stat("Commands", shown.to_string()),
        text_field(
            "Filter…",
            &state.scrollback_query,
            Message::ScrollbackQueryChanged
        )
        .size(13),
    ]
    .spacing(14);

    // Make the untracked promise legible where the user would look for the
    // record itself: whenever any listed row is untracked (or the whole tab
    // is), say what that means.
    let any_untracked = state.tabs.get(state.active).is_some_and(|t| t.untracked)
        || filtered.iter().any(|c| c.untracked);
    if any_untracked {
        let t = theme::tokens();
        content = content.push(
            text(
                "Rows marked untracked are session-only — they are never \
                 saved to encrypted history and vanish when their tab closes.",
            )
            .size(11)
            .color(t.muted),
        );
    }

    // Paging into the encrypted archive only makes sense (and is only
    // possible) when the feature is actually running this session.
    if state.history_writer.is_some() {
        let label = match state.scrollback_archive_cursor {
            Some(date) => format!("Archived back to {date}"),
            None => "Load earlier history from the encrypted archive".to_string(),
        };
        let mut paging_row = row![
            iced::widget::text(label).size(12),
            iced::widget::Space::new().width(Length::Fill),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if state.scrollback_archive_cursor.is_some() {
            paging_row =
                paging_row.push(button::ghost("Back to today", Message::ScrollbackPageNewer));
        }
        paging_row = paging_row.push(button::ghost(
            "Load older day",
            Message::ScrollbackPageOlder,
        ));
        content = content.push(paging_row);
    }

    content = content.push(log_table);

    modal_sized(base, content, Message::ToggleScrollbackPanel, 700.0)
}

/// A short "how long ago" label for [`scrollback_panel_view`]'s "Oldest line" stat
/// and the settings archive viewer's rows (whose entries are typically days old).
fn format_age(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Shorten a command for display inside a fixed-size dialog — a full command
/// line can be arbitrarily long. Cuts on a char boundary, appends `…`.
fn elide(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// The body of the active settings section.
fn settings_body(state: &Tty) -> Element<'_, Message> {
    match state.settings_section {
        1 => palette_section(state),
        2 => keys_section(),
        3 => history_section(state),
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
fn passphrase_prompt_view<'a>(
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
fn appearance_section(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
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

    let body = column![
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
        // Status bar: hide it until the pointer nears the bottom edge, so the
        // grid/panes get the full height and the bar floats in on demand.
        section("Status bar"),
        toggle(
            "Auto-hide until pointer nears the bottom",
            state.settings.status_bar_autohide(),
            Message::SetStatusBarAutohide(!state.settings.status_bar_autohide()),
        ),
        toggle(
            "Keep metric popovers open (pin several; click away won't close)",
            state.settings.status_bar_metrics_pinned(),
            Message::SetStatusBarMetricsPinned(!state.settings.status_bar_metrics_pinned()),
        ),
        status_bar_metrics_editor(state),
        section("Terminal"),
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
    .spacing(14);

    // The section outgrew the settings panel once the status-bar metrics editor
    // landed; without a scrollable, iced silently clips whatever falls below the
    // panel's height on a shorter window (the editor and the Terminal/Window
    // sections vanished). Mirror `history_section`.
    scrollable(body.padding(iced::Padding::ZERO.right(8)))
        .height(Length::Fill)
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

/// The status-bar footer text size — matches rime's own `status_bar` TEXT_SIZE
/// so the typography is uniform whether the bar is text-only or hosts stats.
const STATUS_BAR_TEXT_SIZE: f32 = 13.0;

/// The bottom status bar as an Element: shell name on the left, then (when
/// configured) the machine-stat cells in display order, then the grid/tab/font
/// cluster. Built on rime's `status_bar_content` (the styled strip) rather than
/// the plain-text `status_bar`, so the canvas sparklines can sit beside the
/// text. When the window is too narrow to hold every cell, metrics are shed
/// from the right (see [`visible_metric_count`]) before anything wraps.
fn status_bar_view(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let (left, right) = status_text(state);
    let right_text = || {
        text(right.clone())
            .size(STATUS_BAR_TEXT_SIZE)
            .color(t.muted)
    };

    // Three zones: shell name (left), the machine-stat cells (center), the grid
    // geometry (right). The center cluster sits between two flex spaces so it
    // centers in the gap; with no visible metrics it collapses to nothing and
    // the two spaces merge, leaving the no-stats bar pixel-identical to the old
    // text-only footer (left name, right geometry).
    let configs = state.settings.status_bar_metrics();
    let cells: Vec<(crate::settings::ResolvedMetric, MetricRender)> = configs
        .iter()
        .filter_map(|&cfg| metric_render(cfg, state).map(|r| (cfg, r)))
        .collect();
    let visible = visible_metric_count(&cells, &left, &right, state.window_width);

    let flex = || iced::widget::Space::new().width(Length::Fill);
    let mut content = row![text(left).size(STATUS_BAR_TEXT_SIZE).color(t.muted), flex()]
        .align_y(iced::Alignment::Center);

    if visible > 0 {
        // Each cell is clickable: a press opens that metric's detail popover.
        let cluster: Vec<Element<'_, Message>> = cells
            .into_iter()
            .take(visible)
            .map(|(cfg, r)| {
                let key = cfg.kind.as_setting_str().to_string();
                mouse_area(metric_cell(cfg.style, r))
                    .on_press(Message::OpenMetricDetail(key))
                    .into()
            })
            .collect();
        content = content
            .push(row(cluster).spacing(14).align_y(iced::Alignment::Center))
            .push(flex());
    }

    content = content.push(right_text());
    status_bar_content(content)
}

/// The renderable data for one metric cell, resolved from the current sample:
/// a label, one or more history series (each with its color) that share the
/// sparkline, and the sparkline's max (100 for percentages, auto-scaled for
/// rates). Most metrics have a single series; disk I/O overlays read + write.
struct MetricRender {
    label: String,
    series: Vec<(std::collections::VecDeque<f32>, iced::Color)>,
    max: f32,
}

/// Resolve one configured metric against the latest sample, or `None` when
/// there is no reading yet (so the cell is simply skipped until data lands).
/// Percentage metrics (CPU/memory) grade their color by load and scale to 100;
/// rate metrics (network/disk) use a neutral accent and auto-scale to their
/// own recent peak. Disk I/O returns two series (read + write) on one scale.
fn metric_render(cfg: crate::settings::ResolvedMetric, state: &Tty) -> Option<MetricRender> {
    use crate::metrics as mx;
    use crate::settings::MetricKind as K;

    let stats = state.metrics.latest.as_ref()?;
    let m = &state.metrics;
    let t = theme::tokens();
    let accent = t.accent;

    // Single-series metrics: (label, history, max, color).
    let single =
        |label: String, hist: &std::collections::VecDeque<f32>, max: f32, color| MetricRender {
            label,
            series: vec![(hist.clone(), color)],
            max,
        };

    let render = match cfg.kind {
        // All three CPU drill-ins share the aggregate cell; only their popover
        // body differs.
        K::Cpu | K::CpuCores | K::CpuAll => single(
            mx::cpu_label(stats),
            &m.cpu_history,
            100.0,
            load_color(stats.cpu_percent),
        ),
        K::Mem => single(
            mx::mem_label(stats),
            &m.mem_history,
            100.0,
            load_color(stats.mem_percent()),
        ),
        K::NetRx => single(
            mx::net_rx_label(stats),
            &m.net_rx_history,
            hist_max(&m.net_rx_history),
            accent,
        ),
        K::NetTx => single(
            mx::net_tx_label(stats),
            &m.net_tx_history,
            hist_max(&m.net_tx_history),
            accent,
        ),
        K::DiskR => single(
            mx::disk_r_label(stats),
            &m.disk_r_history,
            hist_max(&m.disk_r_history),
            accent,
        ),
        K::DiskW => single(
            mx::disk_w_label(stats),
            &m.disk_w_history,
            hist_max(&m.disk_w_history),
            accent,
        ),
        // rx + tx overlaid on one sparkline, on a shared scale (rx accent, tx
        // `warn`), mirroring disk I/O below.
        K::NetIo => MetricRender {
            label: mx::net_io_label(stats),
            series: vec![
                (m.net_rx_history.clone(), accent),
                (m.net_tx_history.clone(), t.warn),
            ],
            max: hist_max(&m.net_rx_history).max(hist_max(&m.net_tx_history)),
        },
        // Read + write overlaid on one sparkline, on a shared scale so their
        // relative magnitude reads true. Read is the accent; write is `warn`
        // (a second hue, not an alarm) so the two lines stay distinguishable.
        K::DiskIo => MetricRender {
            label: mx::disk_io_label(stats),
            series: vec![
                (m.disk_r_history.clone(), accent),
                (m.disk_w_history.clone(), t.warn),
            ],
            max: hist_max(&m.disk_r_history).max(hist_max(&m.disk_w_history)),
        },
    };
    Some(render)
}

/// The sparkline scale for a rate series: its recent peak, floored at 1 so an
/// all-zero history doesn't divide by zero (and reads as a flat baseline).
fn hist_max(history: &std::collections::VecDeque<f32>) -> f32 {
    history.iter().copied().fold(1.0, f32::max)
}

/// One metric's status-bar cell in its configured `style`: a sparkline of its
/// series plus the label, or (for `Number`) the label alone.
fn metric_cell<'a>(style: crate::settings::MetricStyle, r: MetricRender) -> Element<'a, Message> {
    let label = text(r.label)
        .size(STATUS_BAR_TEXT_SIZE)
        .color(theme::tokens().muted);
    match style {
        crate::settings::MetricStyle::Number => label.into(),
        crate::settings::MetricStyle::Sparkline => {
            let series: Vec<SparkSeries> = r
                .series
                .iter()
                .map(|(values, color)| {
                    SparkSeries::new(values.iter().map(|&v| v as f64).collect(), *color)
                })
                .collect();
            let spark = sparkline(
                Sparkline {
                    series,
                    max: r.max as f64,
                },
                44.0,
                14.0,
            );
            row![spark, label]
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into()
        }
    }
}

/// The drill-in popover for a single metric: a floating card with that metric's
/// full-size line chart over its retained history plus the current readout,
/// opened by clicking a status-bar sparkline and dismissed by clicking away or
/// pressing Escape. `None` only when nothing is drilled in, so the caller layers
/// nothing; a drilled-in metric whose history isn't chartable yet (a rate metric
/// with no sampler on this platform, or the first ticks after opening) still
/// shows a card, with a "collecting" note in place of a blank canvas — so
/// click-away/Escape have a target and the drill-in gives visible feedback. The
/// chart reuses [`metric_render`], so it shows exactly the series and scale the
/// clicked cell did, just larger.
fn metric_popover_card<'a>(
    state: &'a Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
) -> Element<'a, Message> {
    use crate::settings::{MetricKind as K, MetricStyle, ResolvedMetric};

    let kind = pop.kind;
    let expanded = pop.expanded;
    let pinned = state.settings.status_bar_metrics_pinned();
    let t = theme::tokens();
    let render = metric_render(
        ResolvedMetric {
            kind,
            style: MetricStyle::Sparkline,
        },
        state,
    );

    // The current size: the user's dragged override, else the compact or
    // expanded default (see `MetricPopover::effective_size`).
    let (card_w, chart_h) = pop.effective_size(state.window_width, state.window_height);

    // The line chart needs at least a couple of points to draw a segment; below
    // that (an empty or single-point history) the card shows a plain note.
    let filled = render.as_ref().map_or(0, |r| {
        r.series.iter().map(|(v, _)| v.len()).max().unwrap_or(0)
    });

    // CPU has three drill-ins: total (the aggregate line chart), per-core (the
    // cluster grid), and "all" (both). Per-core / all fall back to the aggregate
    // where the platform reports no per-core history.
    let has_cores = kind.is_cpu() && has_per_core_cpu(state);

    let body: Element<'a, Message> = if kind == K::CpuAll && has_cores {
        combined_cpu_body(state, expanded, card_w, chart_h)
    } else if kind == K::CpuCores && has_cores {
        core_grid_body(state, expanded, card_w)
    } else if let (true, Some(r)) = (filled >= 2, render) {
        // Each history series becomes a polyline over its sample index, on the
        // same shared 0..max scale the sparkline used.
        let series: Vec<Series> = r
            .series
            .iter()
            .map(|(values, color)| Series {
                points: values
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| (i as f64, v as f64))
                    .collect(),
                color: *color,
            })
            .collect();

        // CPU/memory are bounded 0..100 gauges: a fixed scale so the line reads
        // as a fraction of full capacity (39% memory sits at 39% height, not the
        // top). The open-ended rate metrics auto-scale to their own recent peak
        // and label the axis with that peak.
        let bounded = kind.is_cpu() || kind == K::Mem;
        let (y_max, y_max_label) = if bounded {
            (Some(100.0), Some("100%".to_string()))
        } else {
            let peak = r
                .series
                .iter()
                .flat_map(|(v, _)| v.iter().copied())
                .fold(1.0_f32, f32::max);
            (None, Some(crate::metrics::format_rate(peak)))
        };
        let hover_format: Option<fn(f64) -> String> =
            Some(if bounded { hover_percent } else { hover_rate });
        let chart = line_chart(
            LineChart {
                title: kind.to_string(),
                series,
                y_max: y_max.map(|m: f32| m as f64),
                y_max_label,
                hover_format,
            },
            chart_h,
        );

        let mut card = column![chart, text(r.label).size(13).color(t.ink)].spacing(8);
        // A small legend names the two overlaid lines for the combined metrics
        // (single-series metrics carry their identity in the title/label already).
        match kind {
            K::NetIo => card = card.push(legend_row(&[("Down", t.accent), ("Up", t.warn)])),
            K::DiskIo => card = card.push(legend_row(&[("Read", t.accent), ("Write", t.warn)])),
            _ => {}
        }
        card.into()
    } else {
        column![
            text(kind.to_string()).size(14).color(t.ink),
            text("No history yet — samples chart here as they're collected.")
                .size(12)
                .color(t.muted),
        ]
        .spacing(8)
        .into()
    };

    let card = container(body)
        .width(Length::Fixed(card_w))
        .padding(14)
        .style(move |_| container::Style {
            border: Border {
                color: t.hairline,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::background(t.surface)
        });
    // Border-resize strips and the top-right controls (expand, plus a close "×"
    // when popovers are pinned) overlay the card.
    iced::widget::stack![
        with_resize_edges(index, card.into()),
        popover_controls(index, expanded, pinned),
    ]
    .into()
}

/// The top-right control cluster overlaid on a popover card: the expand "+" /
/// collapse "−" toggle, plus a close "×" when popovers are pinned (in the
/// one-at-a-time mode a click away closes it, so no per-card button is needed).
fn popover_controls<'a>(index: usize, expanded: bool, pinned: bool) -> Element<'a, Message> {
    let mut controls = row![button::ghost(
        if expanded { "−" } else { "+" },
        Message::ToggleMetricDetailExpanded(index),
    )]
    .spacing(4)
    .align_y(iced::Alignment::Center);
    if pinned {
        controls = controls.push(button::ghost("×", Message::CloseMetricPopover(index)));
    }
    container(controls)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Top)
        .padding([2, 4])
        .into()
}

/// Place a popover `card` in the window: compact cards anchor just above the
/// status bar (centered), expanded ones center; a user drag offsets from there,
/// laid out absolutely via top-left padding (horizontal exact, vertical measured
/// up from the bottom). Pinned popovers past the first cascade up-and-right so a
/// stack stays legible. All clamped to stay on-screen; the headless path (window
/// size 0) falls back to the alignment anchor.
fn place_metric_popover<'a>(
    state: &Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
    card: Element<'a, Message>,
) -> Element<'a, Message> {
    use iced::alignment::{Horizontal, Vertical};

    let known = state.window_width > 1.0 && state.window_height > 1.0;
    let cascade = 28.0 * index as f32;
    let dx = pop.move_offset.0 + cascade;
    let dy = pop.move_offset.1 - cascade;
    let (card_w, _) = pop.effective_size(state.window_width, state.window_height);

    if (dx != 0.0 || dy != 0.0) && known {
        let base_gap = if pop.expanded { 60.0 } else { 44.0 };
        let left = ((state.window_width - card_w) / 2.0 + dx)
            .clamp(8.0, (state.window_width - card_w - 8.0).max(8.0));
        let bottom = (base_gap - dy).clamp(8.0, (state.window_height - 120.0).max(8.0));
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Left)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding::ZERO.left(left).bottom(bottom))
            .into()
    } else {
        let a = container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center);
        if pop.expanded {
            a.align_y(Vertical::Center).into()
        } else {
            a.align_y(Vertical::Bottom)
                .padding(iced::Padding::ZERO.bottom(44.0))
                .into()
        }
    }
}

/// Whether the per-core CPU grid has enough history to draw (a baseline plus at
/// least one interval for any core).
fn has_per_core_cpu(state: &Tty) -> bool {
    state.metrics.core_history.iter().any(|h| h.len() >= 2)
}

/// Group logical core indices `0..n` by cluster type for the per-core grid:
/// Performance, then Efficiency, then any Unknown — or one ungrouped "CPU"
/// section when the platform reports no perf levels. Each group carries its
/// [`CpuKind`] so cells can color by cluster.
fn core_groups(
    levels: Option<&[prexp_core::system::CpuKind]>,
    n: usize,
) -> Vec<(&'static str, prexp_core::system::CpuKind, Vec<usize>)> {
    use prexp_core::system::CpuKind;
    let Some(ls) = levels else {
        return vec![("CPU", CpuKind::Unknown, (0..n).collect())];
    };
    let pick =
        |k: CpuKind| -> Vec<usize> { (0..n).filter(|&i| ls.get(i).copied() == Some(k)).collect() };
    let unknown: Vec<usize> = (0..n)
        .filter(|&i| {
            !matches!(
                ls.get(i),
                Some(CpuKind::Performance) | Some(CpuKind::Efficiency)
            )
        })
        .collect();
    let mut groups = Vec::new();
    let p = pick(CpuKind::Performance);
    if !p.is_empty() {
        groups.push(("Performance", CpuKind::Performance, p));
    }
    let e = pick(CpuKind::Efficiency);
    if !e.is_empty() {
        groups.push(("Efficiency", CpuKind::Efficiency, e));
    }
    if !unknown.is_empty() {
        groups.push(("CPU", CpuKind::Unknown, unknown));
    }
    if groups.is_empty() {
        groups.push(("CPU", CpuKind::Unknown, (0..n).collect()));
    }
    groups
}

/// A core cell's sparkline color: Performance (and ungrouped) cores grade by
/// load (calm → alarm); Efficiency cores use the accent hue so the two clusters
/// read apart at a glance (their load still shows in the sparkline height + %).
fn core_color(kind: prexp_core::system::CpuKind, cur: f32) -> iced::Color {
    match kind {
        prexp_core::system::CpuKind::Efficiency => theme::tokens().accent,
        _ => load_color(cur),
    }
}

/// The aggregate CPU line chart: overall CPU% over its retained history on a
/// fixed 0..100 gauge, with the hover readout. Shared by the `Cpu` drill-in
/// (via the generic branch) and the `CpuAll` combined body.
fn aggregate_cpu_chart(state: &Tty, chart_h: f32) -> Element<'_, Message> {
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    let series = vec![Series {
        points: state
            .metrics
            .cpu_history
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as f64, v as f64))
            .collect(),
        color: load_color(agg),
    }];
    line_chart(
        LineChart {
            title: "CPU".into(),
            series,
            y_max: Some(100.0),
            y_max_label: Some("100%".to_string()),
            hover_format: Some(hover_percent),
        },
        chart_h,
    )
}

/// The per-core sparkline grid, grouped into Performance / Efficiency clusters,
/// each cell colored by its cluster (see [`core_color`]). Column count fits the
/// card width. Shared by the `CpuCores` and `CpuAll` drill-ins.
fn cpu_core_grid(state: &Tty, expanded: bool, card_w: f32) -> iced::widget::Column<'_, Message> {
    let t = theme::tokens();
    let m = &state.metrics;
    let (cell_w, cell_h) = if expanded { (96.0, 34.0) } else { (58.0, 20.0) };
    // Fit as many columns as the card width holds (accounting for padding + gap).
    let cols = (((card_w - 28.0) / (cell_w + 8.0)).floor() as usize).max(2);
    let n = m.core_history.len();
    let mut col = column![].spacing(8);
    for (name, kind, cores) in core_groups(m.perf_levels.as_deref(), n) {
        col = col.push(text(name).size(11).color(t.muted));
        let mut grid = column![].spacing(6);
        for chunk in cores.chunks(cols) {
            let mut r = row![].spacing(8).align_y(iced::Alignment::Center);
            for &ci in chunk {
                r = r.push(core_cell(m, ci, kind, cell_w, cell_h));
            }
            grid = grid.push(r);
        }
        col = col.push(grid);
    }
    col
}

/// The `CpuAll` drill-in body: the aggregate line chart, the current readout,
/// then the per-core grid — the two CPU views stacked.
fn combined_cpu_body(
    state: &Tty,
    expanded: bool,
    card_w: f32,
    chart_h: f32,
) -> Element<'_, Message> {
    let t = theme::tokens();
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    column![
        aggregate_cpu_chart(state, chart_h),
        text(format!("CPU {}%", agg.round() as u32))
            .size(13)
            .color(t.ink),
    ]
    .spacing(8)
    .push(cpu_core_grid(state, expanded, card_w))
    .into()
}

/// The `CpuCores` drill-in body: the per-core grid alone, under a compact
/// aggregate readout (no line chart — that is the separate `Cpu` drill-in).
fn core_grid_body(state: &Tty, expanded: bool, card_w: f32) -> Element<'_, Message> {
    let t = theme::tokens();
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    column![text(format!("CPU cores — {}%", agg.round() as u32))
        .size(13)
        .color(t.ink),]
    .spacing(8)
    .push(cpu_core_grid(state, expanded, card_w))
    .into()
}

/// One core's cell: a small sparkline of its recent load (colored by cluster via
/// [`core_color`]) over its current percentage.
fn core_cell<'a>(
    m: &crate::metrics::Metrics,
    ci: usize,
    kind: prexp_core::system::CpuKind,
    w: f32,
    h: f32,
) -> Element<'a, Message> {
    let t = theme::tokens();
    let hist = &m.core_history[ci];
    let cur = hist.back().copied().unwrap_or(0.0);
    let spark = sparkline(
        Sparkline::single(
            hist.iter().map(|&v| v as f64).collect(),
            100.0,
            core_color(kind, cur),
        ),
        w,
        h,
    );
    column![
        spark,
        text(format!("{}%", cur.round() as u32))
            .size(10)
            .color(t.muted),
    ]
    .spacing(2)
    .align_x(iced::Alignment::Center)
    .into()
}

/// One invisible resize strip for [`with_resize_edges`]: a transparent hit area
/// of the given size whose press starts a drag-resize from `edge` (tracked
/// through `PointerMoved`, ended by `PointerReleased`), showing the matching
/// resize cursor on hover. It carries no paint of its own; the card's own border
/// is the visible edge the user grabs.
fn resize_strip<'a>(
    index: usize,
    edge: crate::state::ResizeEdge,
    width: Length,
    height: Length,
    cursor: iced::mouse::Interaction,
) -> Element<'a, Message> {
    mouse_area(iced::widget::Space::new().width(width).height(height))
        .on_press(Message::MetricDetailResizeStart(index, edge))
        .interaction(cursor)
        .into()
}

/// Overlay thin invisible resize strips along the card's right and bottom edges
/// and its bottom-right corner, so the popover resizes by dragging its borders
/// (each with the matching resize cursor) rather than a separate grip. The strips
/// stack over the card; `iced`'s `stack` sizes to its first child, so they span
/// exactly the card, not the window. `index` is the popover the drag addresses.
fn with_resize_edges(index: usize, card: Element<'_, Message>) -> Element<'_, Message> {
    use crate::state::ResizeEdge;
    use iced::alignment::{Horizontal::Right, Vertical::Bottom};
    use iced::mouse::Interaction;

    const EDGE: f32 = 8.0;
    const CORNER: f32 = 16.0;

    let right = container(resize_strip(
        index,
        ResizeEdge::Right,
        Length::Fixed(EDGE),
        Length::Fill,
        Interaction::ResizingHorizontally,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right);
    let bottom = container(resize_strip(
        index,
        ResizeEdge::Bottom,
        Length::Fill,
        Length::Fixed(EDGE),
        Interaction::ResizingVertically,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Bottom);
    let corner = container(resize_strip(
        index,
        ResizeEdge::Corner,
        Length::Fixed(CORNER),
        Length::Fixed(CORNER),
        Interaction::ResizingDiagonallyDown,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right)
    .align_y(Bottom);

    iced::widget::stack![card, right, bottom, corner].into()
}

/// Hover-readout formatters for [`metric_popover_card`]'s chart, as plain `fn`
/// pointers `rime`'s `LineChart` can hold: a percentage for CPU/memory, a
/// throughput rate for the network/disk series.
fn hover_percent(v: f64) -> String {
    format!("{}%", v.round() as u32)
}
fn hover_rate(v: f64) -> String {
    crate::metrics::format_rate(v as f32)
}

/// A row of colored-dot + label legend entries for a multi-series drill-in.
fn legend_row<'a>(items: &[(&str, iced::Color)]) -> Element<'a, Message> {
    let t = theme::tokens();
    let mut r = row![].spacing(14).align_y(iced::Alignment::Center);
    for &(label, color) in items {
        let dot = container(
            iced::widget::Space::new()
                .width(Length::Fixed(9.0))
                .height(Length::Fixed(9.0)),
        )
        .style(move |_| container::Style {
            border: Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..container::background(color)
        });
        r = r.push(
            row![dot, text(label.to_string()).size(11).color(t.muted)]
                .spacing(6)
                .align_y(iced::Alignment::Center),
        );
    }
    r.into()
}

/// How many of the leading metric cells fit before the bar would overflow, so
/// the rightmost are shed first. The width of each piece is estimated (iced does
/// not expose measured text extents at view-build time), erring toward showing
/// all: an unknown window width (`<= 0`, before the first resize) sheds nothing.
fn visible_metric_count(
    cells: &[(crate::settings::ResolvedMetric, MetricRender)],
    left: &str,
    right: &str,
    window_width: f32,
) -> usize {
    if window_width <= 0.0 {
        return cells.len();
    }
    // Rough per-glyph advance for the 13px UI font, plus the strip padding
    // ([7,14]) and the left/right end labels that always show.
    const CHAR_W: f32 = 7.0;
    const PADDING: f32 = 28.0;
    const GAP: f32 = 14.0;
    let reserved = PADDING + (left.chars().count() + right.chars().count()) as f32 * CHAR_W + GAP;
    let mut used = 0.0;
    let mut n = 0;
    for (cfg, r) in cells {
        let label_w = r.label.chars().count() as f32 * CHAR_W;
        let cell_w = match cfg.style {
            // sparkline (44) + inner gap (6) + label
            crate::settings::MetricStyle::Sparkline => 44.0 + 6.0 + label_w,
            crate::settings::MetricStyle::Number => label_w,
        } + GAP;
        if used + cell_w > (window_width - reserved).max(0.0) {
            break;
        }
        used += cell_w;
        n += 1;
    }
    n
}

/// The Status-bar settings section's machine-stats editor: the ordered list of
/// configured metrics (each with a style dropdown, reorder, and remove) plus a
/// row of Add buttons for the metrics not yet shown.
fn status_bar_metrics_editor(state: &Tty) -> Element<'_, Message> {
    use crate::settings::{MetricKind, MetricStyle};

    // The rows walk the raw stored list so a row's index matches the index the
    // reorder/remove/style messages carry; the Add buttons consult the resolved
    // list for which kinds are already present.
    let raw = &state.settings.status_bar_metrics;
    let present = state.settings.status_bar_metrics();
    let mut rows: Vec<Element<'_, Message>> = vec![caption(
        "Machine stats, in display order. The bar sheds from the right when space is tight.",
    )];

    if raw.is_empty() {
        rows.push(caption("No stats shown. Add a metric below."));
    }

    for (i, cfg) in raw.iter().enumerate() {
        // Show the metric's friendly name, falling back to the raw key for an
        // entry this build does not recognize (so it can still be removed).
        let name = MetricKind::from_setting_str(&cfg.metric)
            .map(|k| k.to_string())
            .unwrap_or_else(|| cfg.metric.clone());
        let style = MetricStyle::from_setting_str(&cfg.style);
        let style_pick = select(
            MetricStyle::ALL.to_vec(),
            Some(style),
            move |s: MetricStyle| Message::StatusBarMetricStyle(i, s.as_setting_str().to_string()),
        );
        let cell = row(vec![
            text(name)
                .size(STATUS_BAR_TEXT_SIZE)
                .width(Length::Fixed(72.0))
                .into(),
            style_pick.into(),
            button::ghost("Up", Message::StatusBarMetricMove(i, -1)).into(),
            button::ghost("Down", Message::StatusBarMetricMove(i, 1)).into(),
            button::ghost("Remove", Message::StatusBarMetricRemove(i)).into(),
        ])
        .spacing(8)
        .align_y(iced::Alignment::Center);
        rows.push(cell.into());
    }

    let add_buttons: Vec<Element<'_, Message>> = MetricKind::ALL
        .iter()
        .filter(|k| !present.iter().any(|c| c.kind == **k))
        .map(|k| {
            button::secondary(
                &format!("Add {k}"),
                Message::StatusBarMetricAdd(k.as_setting_str().to_string()),
            )
            .into()
        })
        .collect();
    if !add_buttons.is_empty() {
        rows.push(row(add_buttons).spacing(8).into());
    }

    column(rows).spacing(8).into()
}

/// Grade a 0..=100 load into the theme's calm / caution / alarm colors.
fn load_color(percent: f32) -> iced::Color {
    let t = theme::tokens();
    if percent >= 85.0 {
        t.danger
    } else if percent >= 60.0 {
        t.warn
    } else {
        t.success
    }
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
    // Narrate the async history start so the user knows *why* an OS keychain
    // dialog might be appearing right now — and say plainly when a locked
    // (passphrase) archive or an untracked tab means nothing is being
    // recorded.
    let history = if state.history_starting {
        " · unlocking history key…"
    } else if state.history_locked {
        " · history locked — not recording"
    } else if tab.untracked {
        " · untracked — not recording"
    } else {
        ""
    };
    (
        tab.label(),
        format!(
            "{cols}×{rows}{tabs}{panes} · {}px{history}",
            state.font_size as u32
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{MetricKind, MetricStyle, ResolvedMetric};

    fn cell(style: MetricStyle) -> (ResolvedMetric, MetricRender) {
        (
            ResolvedMetric {
                kind: MetricKind::Cpu,
                style,
            },
            MetricRender {
                label: "CPU".to_string(),
                series: vec![(Default::default(), iced::Color::WHITE)],
                max: 100.0,
            },
        )
    }

    #[test]
    fn shedding_shows_all_when_width_is_unknown_or_ample() {
        let cells = [cell(MetricStyle::Sparkline), cell(MetricStyle::Sparkline)];
        // Unknown width (pre-first-resize) sheds nothing.
        assert_eq!(visible_metric_count(&cells, "z", "z", 0.0), 2);
        // A wide window fits both.
        assert_eq!(visible_metric_count(&cells, "z", "z", 5000.0), 2);
    }

    #[test]
    fn shedding_drops_rightmost_cells_as_width_shrinks() {
        let cells = [cell(MetricStyle::Sparkline), cell(MetricStyle::Sparkline)];
        // reserved = 28 + (1+1)*7 + 14 = 56; each sparkline cell = 44+6+21 +14 = 85.
        assert_eq!(visible_metric_count(&cells, "z", "z", 141.0), 1);
        assert_eq!(visible_metric_count(&cells, "z", "z", 66.0), 0);
        // Monotonic: never more visible in a narrower window.
        let wide = visible_metric_count(&cells, "z", "z", 5000.0);
        let narrow = visible_metric_count(&cells, "z", "z", 141.0);
        assert!(narrow <= wide);
    }
}
