use iced::widget::{
    column, container, mouse_area, opaque, pane_grid, row, scrollable, text, Column,
};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, caption, color_field, context_menu, dialog, labeled, line_chart, modal_sized,
    rename_bar, rename_field_id, section, select, shortcut_row, slider, sparkline, stat,
    status_bar_content, stepper, table, tabs, text_field, toggle, tooltip, window_shell, CellAlign,
    LineChart, MenuItem, Series, SparkSeries, Sparkline, Tab, TabBarStyle, TableColumn,
    TableMetrics, TooltipPosition,
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
            MenuKind::ProcRow { pid, name } => vec![
                MenuItem::action("View Process", Message::OpenProcDetail(*pid)),
                MenuItem::separator(),
                MenuItem::action("Copy path", Message::CopyProcPath(*pid)),
                MenuItem::action("Copy PID", Message::CopyText(pid.to_string())),
                MenuItem::action("Copy name", Message::CopyText(name.clone())),
            ],
            MenuKind::FdRow { path } => {
                vec![MenuItem::action(
                    "Copy path",
                    Message::CopyText(path.clone()),
                )]
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
    column![toggle(
        "Highlight active tab",
        state.settings.tab_highlight(),
        Message::SetTabHighlight(!state.settings.tab_highlight()),
    )]
    .spacing(14)
    .into()
}

/// Appearance → Status bar: off switch, auto-hide, popover pinning, and the
/// machine-stats cell editor. When the bar is off the rest is moot, so only the
/// off switch (and a note) shows.
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
            "The status bar is off. Turn it back on to configure auto-hide and machine-stat cells.",
        ));
    } else {
        col = col
            .push(toggle(
                "Auto-hide until pointer nears the bottom",
                state.settings.status_bar_autohide(),
                Message::SetStatusBarAutohide(!state.settings.status_bar_autohide()),
            ))
            .push(toggle(
                "Keep metric popovers open (pin several; click away won't close)",
                state.settings.status_bar_metrics_pinned(),
                Message::SetStatusBarMetricsPinned(!state.settings.status_bar_metrics_pinned()),
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
    let editing = state.status_bar_edit;
    // Each cell carries its raw-config index (drag-reorder mutates the stored list
    // by index; the resolved list may drop unknown entries, so we can't assume the
    // display position is the config index).
    let cells: Vec<(usize, crate::settings::ResolvedMetric, MetricRender)> = state
        .settings
        .status_bar_metrics_indexed()
        .into_iter()
        .filter_map(|(i, cfg)| metric_render(cfg, state).map(|r| (i, cfg, r)))
        .collect();
    let total = cells.len();
    let visible = visible_metric_count(&cells, &left, &right, state.window_width);
    // When the bar can't hold every cell, a scroll over it slides this window
    // through the full list; clamp the stored offset to the valid range.
    let max_start = total.saturating_sub(visible);
    let start = state.status_bar_scroll.min(max_start);

    let flex = || iced::widget::Space::new().width(Length::Fill);
    let mut content = row![text(left).size(STATUS_BAR_TEXT_SIZE).color(t.muted), flex()]
        .align_y(iced::Alignment::Center);

    if visible > 0 {
        // A muted chevron on each side when there are more cells off that edge, so
        // the user knows to scroll.
        let chevron = |show: bool, glyph: &str| -> Element<'_, Message> {
            if show {
                text(glyph.to_string())
                    .size(STATUS_BAR_TEXT_SIZE)
                    .color(t.muted)
                    .into()
            } else {
                iced::widget::Space::new().width(Length::Fixed(0.0)).into()
            }
        };
        // What's being dragged, and where a drop would land — for the outline /
        // lift / insertion-bar affordances. A cell mid press-hold shows *no*
        // border: the outline appears only once the hold engages (edit mode), so a
        // quick tap never flashes one.
        let dragging = state.status_metric_drag;
        let drop = state.status_metric_drop;
        // The window shows `visible` cells from `start`. A cell arms a press on
        // press-down (a quick release opens its drill-in; a hold enters edit mode
        // and starts dragging it); in edit mode it also reports drag-overs. An
        // accent insertion bar shows where a dragged cell would drop.
        let mut cluster: Vec<Element<'_, Message>> = Vec::new();
        for (raw_i, cfg, r) in cells.into_iter().skip(start).take(visible) {
            // The drop insertion bar goes just before the target cell (only while
            // actually dragging to a *different* slot).
            if editing && dragging.is_some() && dragging != Some(raw_i) && drop == Some(raw_i) {
                cluster.push(
                    container(
                        iced::widget::Space::new()
                            .width(Length::Fixed(2.0))
                            .height(Length::Fixed(16.0)),
                    )
                    .style(move |_| container::background(t.accent))
                    .into(),
                );
            }
            let cell = metric_cell(cfg.style, r);
            // In edit mode every cell outlines; the one being dragged also fills so
            // it reads as "lifted".
            let is_dragged = dragging == Some(raw_i);
            let el: Element<'_, Message> = if editing {
                container(cell)
                    .padding([1, 4])
                    .style(move |_| container::Style {
                        border: Border {
                            color: t.accent,
                            width: 1.0,
                            radius: 5.0.into(),
                        },
                        background: is_dragged.then(|| {
                            iced::Color {
                                a: 0.18,
                                ..t.accent
                            }
                            .into()
                        }),
                        ..Default::default()
                    })
                    .into()
            } else {
                cell
            };
            let mut area = mouse_area(el).on_press(Message::StatusMetricPress(raw_i));
            if editing {
                area = area
                    .on_enter(Message::StatusMetricDragOver(raw_i))
                    .interaction(iced::mouse::Interaction::Grab);
            }
            cluster.push(area.into());
        }
        let cluster = row![
            chevron(start > 0, "‹"),
            row(cluster).spacing(14).align_y(iced::Alignment::Center),
            chevron(start + visible < total, "›"),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        // Wheel over the cluster slides the window (only meaningful when cells are
        // shed, i.e. `max_start > 0`).
        content = content
            .push(mouse_area(cluster).on_scroll(|d| Message::StatusBarScroll(scroll_delta_y(d))))
            .push(flex());
    }

    // In edit mode the right end shows a hint instead of the geometry.
    if editing {
        content = content.push(
            text("drag to reorder · Esc to finish")
                .size(STATUS_BAR_TEXT_SIZE)
                .color(t.accent),
        );
    } else {
        content = content.push(right_text());
    }

    // In edit mode a press on empty bar space (the flex gaps / hint — cells and
    // the scroll region consume their own press) exits. Otherwise the bar is
    // inert to background presses.
    let area = status_bar_content(content);
    if editing {
        mouse_area(area).on_press(Message::ExitStatusBarEdit).into()
    } else {
        area
    }
}

/// The vertical component of a wheel `ScrollDelta`, for panning the status bar.
fn scroll_delta_y(delta: iced::mouse::ScrollDelta) -> f32 {
    match delta {
        iced::mouse::ScrollDelta::Lines { y, .. } => y,
        iced::mouse::ScrollDelta::Pixels { y, .. } => y,
    }
}

/// The furthest the status-bar metric window can scroll: the number of
/// renderable cells minus how many currently fit (0 when everything fits). Shared
/// by the view (to clamp the window) and `update` (to clamp the scroll offset).
pub fn status_bar_scroll_max(state: &Tty) -> usize {
    let (left, right) = status_text(state);
    let cells: Vec<(usize, crate::settings::ResolvedMetric, MetricRender)> = state
        .settings
        .status_bar_metrics_indexed()
        .into_iter()
        .filter_map(|(i, cfg)| metric_render(cfg, state).map(|r| (i, cfg, r)))
        .collect();
    let visible = visible_metric_count(&cells, &left, &right, state.window_width);
    cells.len().saturating_sub(visible)
}

/// The renderable data for one metric cell, resolved from the current sample:
/// a label, one or more history series (each with its color) that share the
/// sparkline, and the sparkline's max (100 for percentages, auto-scaled for
/// rates). Most metrics have a single series; disk I/O overlays read + write.
struct MetricRender {
    label: String,
    series: Vec<(std::collections::VecDeque<f32>, iced::Color)>,
    max: f32,
    /// Set to a caution/alarm color when a graded cell is past its threshold, so
    /// `metric_cell` recolors the label (not just the sparkline). `None` = calm.
    alert: Option<iced::Color>,
}

/// Resolve one configured metric against the latest sample, or `None` when
/// there is no reading yet (so the cell is simply skipped until data lands).
/// Percentage metrics (CPU/memory) grade their color by load and scale to 100;
/// rate metrics (network/disk) use a neutral accent and auto-scale to their
/// own recent peak. Disk I/O returns two series (read + write) on one scale.
fn metric_render(cfg: crate::settings::ResolvedMetric, state: &Tty) -> Option<MetricRender> {
    use crate::metrics as mx;
    use crate::settings::MetricKind as K;

    // The clock is the wall time, independent of the sampler — render it without
    // waiting for a machine-stats reading.
    if cfg.kind == K::Clock {
        return Some(MetricRender {
            label: clock_label(state),
            series: vec![],
            max: 1.0,
            alert: None,
        });
    }

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
            alert: None,
        };

    // For a graded kind (CPU/mem/battery), grade the current value against this
    // cell's configured thresholds → the series color + an over-threshold alert.
    let graded: Option<(iced::Color, Option<iced::Color>)> =
        cfg.kind.default_thresholds().map(|(_, _, inverted)| {
            let value = match cfg.kind {
                K::Battery => m.battery.map_or(0.0, |b| b.percent as f32),
                K::Mem => stats.mem_percent(),
                _ => stats.cpu_percent,
            };
            let g = grade(value, cfg.warn as f32, cfg.alarm as f32, inverted);
            (grade_color(g), (g != Grade::Calm).then(|| grade_color(g)))
        });
    let graded_color = graded.map_or(accent, |(c, _)| c);

    let mut render = match cfg.kind {
        // All three CPU drill-ins share the aggregate cell; only their popover
        // body differs.
        K::Cpu | K::CpuCores | K::CpuAll => {
            single(mx::cpu_label(stats), &m.cpu_history, 100.0, graded_color)
        }
        K::Mem => single(mx::mem_label(stats), &m.mem_history, 100.0, graded_color),
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
            alert: None,
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
            alert: None,
        },
        // Uptimes are text, not sparklines: no series (so `metric_cell` renders
        // the label alone). Skip the cell until a reading exists.
        K::Uptime => MetricRender {
            label: mx::uptime_abbrev(m.system_uptime_secs?),
            series: vec![],
            max: 1.0,
            alert: None,
        },
        K::Session => MetricRender {
            label: mx::uptime_abbrev(m.session_uptime_secs?),
            series: vec![],
            max: 1.0,
            alert: None,
        },
        // Load: a sparkline of the 1-minute load, auto-scaled to its recent peak
        // like the rate cells. Skip until a reading exists.
        K::Load => {
            let load = m.load_avg?;
            MetricRender {
                label: mx::load_label(load[0]),
                series: vec![(m.load1_history.clone(), accent)],
                max: hist_max(&m.load1_history),
                alert: None,
            }
        }
        // Battery: a fixed 0..100% gauge, colored by charge against its threshold
        // (low = alarm). Skip on a machine with no battery.
        K::Battery => {
            let b = m.battery?;
            MetricRender {
                label: mx::battery_label(&b),
                series: vec![(m.battery_history.clone(), graded_color)],
                max: 100.0,
                alert: None,
            }
        }
        // Processes: a text cell showing the busiest process (by CPU%); the
        // drill-in is the scrollable, sortable table. Skip until sampled.
        K::Procs => {
            let top = m.processes.iter().max_by(|a, b| {
                a.cpu_percent
                    .partial_cmp(&b.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
            MetricRender {
                label: format!(
                    "↑ {} {}%",
                    truncate_name(&top.name, 16),
                    top.cpu_percent.round() as i32
                ),
                series: vec![],
                max: 1.0,
                alert: None,
            }
        }
        K::Clock => unreachable!("clock is handled before the stats read"),
    };
    // A graded cell past its warn/alarm threshold carries the alert color so the
    // label recolors, not just the sparkline.
    render.alert = graded.and_then(|(_, alert)| alert);
    Some(render)
}

/// The clock cell's label: the current local time, formatted per the user's clock
/// options. Reads the live clock, so it is never pixel-snapshotted (the pure
/// formatter `metrics::format_clock` is unit-tested instead).
fn clock_label(state: &Tty) -> String {
    crate::metrics::format_clock(
        chrono::Local::now().naive_local(),
        state.settings.clock_format(),
    )
}

/// The sparkline scale for a rate series: its recent peak, floored at 1 so an
/// all-zero history doesn't divide by zero (and reads as a flat baseline).
fn hist_max(history: &std::collections::VecDeque<f32>) -> f32 {
    history.iter().copied().fold(1.0, f32::max)
}

/// One metric's status-bar cell in its configured `style`: a sparkline of its
/// series plus the label, or (for `Number`) the label alone.
fn metric_cell<'a>(style: crate::settings::MetricStyle, r: MetricRender) -> Element<'a, Message> {
    // A graded cell past its threshold recolors its label to the alert color so
    // it reads at a glance; otherwise the muted default.
    let label = text(r.label)
        .size(STATUS_BAR_TEXT_SIZE)
        .color(r.alert.unwrap_or(theme::tokens().muted));
    // Text-only when there is no plottable data (a text metric like uptime, or a
    // rate metric whose history hasn't landed yet), regardless of style.
    let has_data = r.series.iter().any(|(v, _)| !v.is_empty());
    match style {
        _ if !has_data => label.into(),
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
    // Render off the configured cell (so a graded popover picks up the user's
    // thresholds); fall back to defaults if somehow not in the list.
    let resolved = state
        .settings
        .status_bar_metrics()
        .into_iter()
        .find(|m| m.kind == kind)
        .unwrap_or_else(|| {
            let (warn, alarm) = kind
                .default_thresholds()
                .map_or((0.0, 0.0), |(w, a, _)| (w, a));
            ResolvedMetric {
                kind,
                style: MetricStyle::Sparkline,
                warn,
                alarm,
            }
        });
    let render = metric_render(resolved, state);

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

    let body: Element<'a, Message> = if kind == K::Procs {
        procs_body(state, card_w, chart_h)
    } else if kind == K::Clock {
        clock_body(state)
    } else if kind.is_uptime() {
        uptime_body(state, kind)
    } else if kind == K::CpuAll && has_cores {
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

        // The y axis: CPU/memory are bounded 0..100 gauges (a fixed scale, so the
        // line reads as a fraction of full capacity); load auto-scales to its peak
        // with a plain numeric label; the rate metrics auto-scale and label the
        // axis with the peak throughput.
        let bounded = kind.is_cpu() || kind == K::Mem || kind == K::Battery;
        let peak = || {
            r.series
                .iter()
                .flat_map(|(v, _)| v.iter().copied())
                .fold(1.0_f32, f32::max)
        };
        let y_max: Option<f64> = bounded.then_some(100.0);
        let y_max_label: Option<String> = Some(if bounded {
            "100%".to_string()
        } else if kind == K::Load {
            format!("{:.1}", peak())
        } else {
            crate::metrics::format_rate(peak())
        });
        let hover: fn(f64) -> String = if bounded {
            hover_percent
        } else if kind == K::Load {
            hover_load
        } else {
            hover_rate
        };
        let chart = line_chart(
            LineChart {
                title: kind.to_string(),
                series,
                y_max,
                y_max_label,
                hover_format: Some(hover),
            },
            chart_h,
        );

        let mut card = column![chart, text(r.label).size(13).color(t.ink)].spacing(8);
        // A small legend names the two overlaid lines for the combined metrics
        // (single-series metrics carry their identity in the title/label already);
        // load shows its full 1/5/15-minute triple.
        match kind {
            K::NetIo => card = card.push(legend_row(&[("Down", t.accent), ("Up", t.warn)])),
            K::DiskIo => card = card.push(legend_row(&[("Read", t.accent), ("Write", t.warn)])),
            K::Load => {
                let triple =
                    crate::metrics::load_triple(state.metrics.load_avg.unwrap_or_default());
                card = card.push(text(triple).size(12).color(t.muted));
            }
            K::Battery => {
                if let Some(b) = state.metrics.battery {
                    card = card.push(
                        text(crate::metrics::battery_detail(&b))
                            .size(12)
                            .color(t.muted),
                    );
                }
            }
            // Memory drills in with a swap line beneath the RAM readout (the bar
            // cell stays RAM-only).
            K::Mem => {
                if let Some(stats) = state.metrics.latest.as_ref() {
                    card = card.push(
                        text(crate::metrics::swap_label(stats))
                            .size(12)
                            .color(t.muted),
                    );
                }
            }
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

/// The uptime drill-in's body: the full duration breakdown (the "full view" to
/// the cell's abbreviated one), under the metric name and over a note saying what
/// it counts from.
fn uptime_body(state: &Tty, kind: crate::settings::MetricKind) -> Element<'_, Message> {
    use crate::settings::MetricKind as K;
    let t = theme::tokens();
    let secs = if kind == K::Uptime {
        state.metrics.system_uptime_secs
    } else {
        state.metrics.session_uptime_secs
    };
    let (full, note) = match secs {
        Some(s) => (
            crate::metrics::uptime_full(s),
            if kind == K::Uptime {
                "since the system booted"
            } else {
                "since this terminal opened"
            },
        ),
        None => ("Unavailable".to_string(), "no reading yet"),
    };
    column![
        text(kind.to_string()).size(14).color(t.ink),
        text(full).size(20).color(t.ink),
        text(note).size(12).color(t.muted),
    ]
    .spacing(8)
    .into()
}

/// The clock drill-in's body: the current time (honoring the 12/24-hour and
/// seconds options) over the full weekday + date. Live, so never snapshotted.
fn clock_body(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let now = chrono::Local::now();
    let fmt = state.settings.clock_format();
    // The drill-in always shows seconds and the date (the "full" view), but keeps
    // the 12/24-hour choice.
    let time = crate::metrics::format_clock(
        now.naive_local(),
        crate::metrics::ClockFormat {
            hour24: fmt.hour24,
            seconds: true,
            date: false,
        },
    );
    column![
        text("Clock").size(14).color(t.ink),
        text(time).size(20).color(t.ink),
        text(now.format("%A, %B %-d, %Y").to_string())
            .size(12)
            .color(t.muted),
    ]
    .spacing(8)
    .into()
}

/// Clip a process name to `max` chars for the compact cell (an `…` when cut).
fn truncate_name(name: &str, max: usize) -> String {
    if name.chars().count() > max {
        format!(
            "{}…",
            name.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        name.to_string()
    }
}

/// The Processes drill-in. Normally a clickable header row (re-sort by clicking a
/// column) over a virtualized, scrollable `rime` table of every process, ordered
/// by the active sort; the bar cell shows only the busiest process, this is the
/// list. Double- or right-clicking a row drills into that one process's detail
/// (fds + a live chart), rendered by [`proc_detail_body`] instead.
fn procs_body(state: &Tty, card_w: f32, chart_h: f32) -> Element<'_, Message> {
    use crate::state::ProcSortColumn as Col;
    let t = theme::tokens();

    // A per-process detail takes over the whole card when one is open.
    if state.proc_detail_pid.is_some() {
        return proc_detail_body(state, chart_h);
    }

    let procs = &state.metrics.processes;
    if procs.is_empty() {
        return column![
            text("Processes").size(14).color(t.ink),
            text("Collecting…").size(12).color(t.muted),
        ]
        .spacing(8)
        .into();
    }

    let (sort_col, desc) = state.proc_sort;
    // Row order: indices into `procs` sorted by the active column/direction.
    let mut order: Vec<usize> = (0..procs.len()).collect();
    order.sort_by(|&a, &b| {
        let (pa, pb) = (&procs[a], &procs[b]);
        let cmp = match sort_col {
            Col::Name => pa.name.to_lowercase().cmp(&pb.name.to_lowercase()),
            Col::Cpu => pa
                .cpu_percent
                .partial_cmp(&pb.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Mem => pa.memory_bytes.cmp(&pb.memory_bytes),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // Column widths (must match the header row and the table below); `CELL_PAD`
    // in the table is 8px, so the header cells pad to match.
    const NUM_W: f32 = 64.0;
    // The name column is the fill remainder; truncate names to what fits so a long
    // one (e.g. `com.apple.WebKit.WebContent`) clips with an ellipsis rather than
    // spilling into the numbers. ~7px per glyph at the table's 13px, minus padding.
    let name_px = (card_w - 28.0 - 2.0 * NUM_W - 16.0).max(40.0);
    let name_budget = (name_px / 7.0) as usize;

    let arrow = move |c: Col| -> &'static str {
        if sort_col == c {
            if desc {
                " ▾"
            } else {
                " ▴"
            }
        } else {
            ""
        }
    };
    let header_cell = |label: &str, col: Col, width: Length, right: bool| -> Element<'_, Message> {
        let color = if sort_col == col { t.ink } else { t.muted };
        let txt = text(format!("{label}{}", arrow(col))).size(11).color(color);
        let mut c = container(txt).width(width).padding([0, 8]);
        if right {
            c = c.align_x(iced::alignment::Horizontal::Right);
        }
        mouse_area(c)
            .on_press(Message::SetProcSort(col))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    };
    let header = row![
        header_cell("PROCESS", Col::Name, Length::Fill, false),
        header_cell("CPU", Col::Cpu, Length::Fixed(NUM_W), true),
        header_cell("MEM", Col::Mem, Length::Fixed(NUM_W), true),
    ]
    .align_y(iced::Alignment::Center);

    // Sorted-order metadata for the callbacks: (pid, name) so a right-clicked row
    // opens its context menu, and the CPU% per row so a hog can be graded a color.
    let row_meta: Vec<(i32, String)> = order
        .iter()
        .map(|&i| (procs[i].pid, procs[i].name.clone()))
        .collect();
    let cpu_by_row: Vec<f32> = order.iter().map(|&i| procs[i].cpu_percent).collect();
    // Grade the CPU% cell by the same cutoffs as the CPU status-bar cell, so a
    // busy process reads amber (>=60%) / red (>=85%) at a glance.
    let (warn, alarm, _) = crate::settings::MetricKind::Cpu
        .default_thresholds()
        .unwrap_or((60.0, 85.0, false));
    let (warn, alarm) = (warn as f32, alarm as f32);

    // The virtualized body. The cell closure owns the sorted order and borrows the
    // process list.
    let rows = order.len();
    let cell = move |row: usize, col: usize| -> String {
        let p = &procs[order[row]];
        match col {
            0 => truncate_name(&p.name, name_budget),
            1 => format!("{}%", p.cpu_percent.round() as i32),
            _ => crate::metrics::format_bytes(p.memory_bytes),
        }
    };
    let cell_color = move |row: usize, col: usize| -> Option<iced::Color> {
        if col != 1 {
            return None;
        }
        match grade(cpu_by_row[row], warn, alarm, false) {
            Grade::Calm => None,
            g => Some(grade_color(g)),
        }
    };
    let columns = vec![
        TableColumn::fill("").align(CellAlign::Left),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
    ];
    let body = table(rows, columns, cell)
        .cell_color(cell_color)
        .metrics(TableMetrics {
            row_height: 22.0,
            header_height: 0.0,
        })
        .offset(state.proc_table_scroll)
        .on_scroll(Message::ProcTableScroll)
        // Right-click opens the row's context menu (View fds / Copy path / PID /
        // name); it is the way into the per-process detail.
        .on_right_click(move |row| {
            let (pid, name) = &row_meta[row];
            Message::ProcRowRightClick(*pid, name.clone())
        });

    column![
        text(format!("Processes — {}", rows)).size(14).color(t.ink),
        header,
        container(body)
            .height(Length::Fixed(chart_h))
            .width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// One process's detail, shown in place of the process list when a row is
/// double- or right-clicked: a "‹ Back" control, a live CPU% chart (its history
/// is fresh for this process — we do not retain a series for every process), the
/// current memory / thread count, and the scrollable list of open file
/// descriptors. Refreshed each sample by [`crate::metrics::Metrics::sample_proc_detail`].
fn proc_detail_body(state: &Tty, chart_h: f32) -> Element<'_, Message> {
    use prexp_core::models::ResourceKind;
    let t = theme::tokens();
    let Some(d) = state.metrics.proc_detail.as_ref() else {
        // The process exited between opening and rendering; offer the way back.
        return column![
            back_row("‹ Back"),
            text("That process is no longer running.")
                .size(12)
                .color(t.muted),
        ]
        .spacing(8)
        .into();
    };

    // A "‹ Back" control, then the process identity.
    let head = column![
        back_row(&format!("‹ {}", truncate_name(&d.name, 28))),
        text(format!("pid {} · {} threads", d.pid, d.thread_count))
            .size(11)
            .color(t.muted),
    ]
    .spacing(2);

    // The live CPU chart. It needs a couple of points to draw a segment; until
    // then (just opened) it shows a note.
    let cpu_now = d.cpu_history.back().copied().unwrap_or(0.0);
    let chart: Element<'_, Message> = if d.cpu_history.len() >= 2 {
        let series = vec![Series {
            points: d
                .cpu_history
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v as f64))
                .collect(),
            color: t.accent,
        }];
        let peak = d.cpu_history.iter().copied().fold(1.0_f32, f32::max);
        line_chart(
            LineChart {
                title: "CPU".to_string(),
                series,
                y_max: None,
                y_max_label: Some(format!("{}%", peak.round() as i32)),
                hover_format: Some(hover_percent),
            },
            (chart_h * 0.42).max(48.0),
        )
    } else {
        container(text("Collecting CPU…").size(12).color(t.muted))
            .height(Length::Fixed((chart_h * 0.42).max(48.0)))
            .into()
    };

    let stats = text(format!(
        "CPU {}%   ·   Mem {}",
        cpu_now.round() as i32,
        crate::metrics::format_bytes(d.memory_bytes),
    ))
    .size(13)
    .color(t.ink);

    // The file descriptors: a summary count line, then a scrollable list. When the
    // OS denied fd access we say so instead of showing an empty list.
    let files = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::File)
        .count();
    let sockets = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::Socket)
        .count();
    let fd_head = text(format!(
        "Open files — {} ({} files, {} sockets)",
        d.resources.len(),
        files,
        sockets,
    ))
    .size(12)
    .color(t.muted);

    let fd_list: Element<'_, Message> = if !d.accessible {
        text("fd access denied by the OS for this process.")
            .size(12)
            .color(t.muted)
            .into()
    } else if d.resources.is_empty() {
        text("No open file descriptors.")
            .size(12)
            .color(t.muted)
            .into()
    } else {
        let mut list = column![].spacing(1);
        for r in &d.resources {
            let kind = resource_kind_label(&r.kind);
            let path = r.path.as_deref().unwrap_or("—");
            let line = text(format!("{:>3}  {:<6} {}", r.descriptor, kind, path))
                .size(11)
                .color(t.muted)
                .font(iced::Font::MONOSPACE);
            // Right-click a descriptor with a path to copy it.
            list = list.push(match &r.path {
                Some(p) => mouse_area(line)
                    .on_right_press(Message::FdRowRightClick(p.clone()))
                    .into(),
                None => Element::from(line),
            });
        }
        scrollable(list).height(Length::Fill).into()
    };

    column![
        head,
        chart,
        stats,
        fd_head,
        container(fd_list).height(Length::Fill).width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// A clickable "‹ Back" row that returns the Processes drill-in to the list.
fn back_row(label: &str) -> Element<'static, Message> {
    mouse_area(text(label.to_string()).size(14).color(theme::tokens().ink))
        .on_press(Message::CloseProcDetail)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// A short label for a file-descriptor kind, for the process detail's fd list.
fn resource_kind_label(kind: &prexp_core::models::ResourceKind) -> &'static str {
    use prexp_core::models::ResourceKind as R;
    match kind {
        R::File => "file",
        R::Socket => "sock",
        R::Pipe => "pipe",
        R::Device => "dev",
        R::Kqueue => "kq",
        R::Unknown => "?",
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
fn hover_load(v: f64) -> String {
    format!("{v:.2}")
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
    cells: &[(usize, crate::settings::ResolvedMetric, MetricRender)],
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
    for (_, cfg, r) in cells {
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
        rows.push(caption("ADD A METRIC"));
        // Wrap into rows of four so the full metric list (now ten, with the three
        // CPU variants) doesn't overflow the panel width.
        let mut wrapped = column![].spacing(8);
        let mut current = row![].spacing(8);
        let mut n = 0;
        for btn in add_buttons {
            current = current.push(btn);
            n += 1;
            if n == 4 {
                wrapped = wrapped.push(current);
                current = row![].spacing(8);
                n = 0;
            }
        }
        if n > 0 {
            wrapped = wrapped.push(current);
        }
        rows.push(wrapped.into());
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

/// A graded cell's alert level against its warn/alarm thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grade {
    Calm,
    Warn,
    Alarm,
}

/// Grade `value` against `warn`/`alarm` cutoffs. Normal metrics alarm when the
/// value climbs *past* the cutoffs (CPU, memory); `inverted` metrics alarm when
/// it *falls below* them (battery — low charge is the concern).
fn grade(value: f32, warn: f32, alarm: f32, inverted: bool) -> Grade {
    if inverted {
        if value <= alarm {
            Grade::Alarm
        } else if value <= warn {
            Grade::Warn
        } else {
            Grade::Calm
        }
    } else if value >= alarm {
        Grade::Alarm
    } else if value >= warn {
        Grade::Warn
    } else {
        Grade::Calm
    }
}

/// The theme color for a [`Grade`]: calm / caution / alarm.
fn grade_color(g: Grade) -> iced::Color {
    let t = theme::tokens();
    match g {
        Grade::Alarm => t.danger,
        Grade::Warn => t.warn,
        Grade::Calm => t.success,
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

    fn cell(style: MetricStyle) -> (usize, ResolvedMetric, MetricRender) {
        (
            0,
            ResolvedMetric {
                kind: MetricKind::Cpu,
                style,
                warn: 60.0,
                alarm: 85.0,
            },
            MetricRender {
                label: "CPU".to_string(),
                series: vec![(Default::default(), iced::Color::WHITE)],
                max: 100.0,
                alert: None,
            },
        )
    }

    #[test]
    fn grade_thresholds_normal_and_inverted() {
        // Normal (higher = worse), CPU-style warn 60 / alarm 85.
        assert_eq!(grade(50.0, 60.0, 85.0, false), Grade::Calm);
        assert_eq!(grade(70.0, 60.0, 85.0, false), Grade::Warn);
        assert_eq!(grade(90.0, 60.0, 85.0, false), Grade::Alarm);
        // Inverted (lower = worse), battery-style warn 40 / alarm 20.
        assert_eq!(grade(80.0, 40.0, 20.0, true), Grade::Calm);
        assert_eq!(grade(30.0, 40.0, 20.0, true), Grade::Warn);
        assert_eq!(grade(15.0, 40.0, 20.0, true), Grade::Alarm);
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
