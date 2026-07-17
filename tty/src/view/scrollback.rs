//! The Scrollback History panel (⌘⇧H): the active pane's buffered + on-screen
//! transcript flattened into a scrollable, filterable read-only accordion table
//! (command headers expand to their captured output). Split out of `view.rs`.

use iced::widget::{column, row, text};
use iced::{Element, Length};

use rime::theme;
use rime::widgets::{
    button, modal_sized, section, stat, table, text_field, TableColumn, TableMetrics,
};

use crate::message::Message;
use crate::state::Tty;

use super::util::format_age;

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
pub(super) fn age_from_epoch_ms(epoch_ms: u64) -> std::time::Duration {
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

pub(super) fn scrollback_panel_view<'a>(
    state: &'a Tty,
    base: Element<'a, Message>,
) -> Element<'a, Message> {
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
