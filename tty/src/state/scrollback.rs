//! `impl Tty` methods for the Scrollback History panel (⌘⇧H) and the encrypted
//! archive it pages into: opening/closing, paging older/newer, the settings-panel
//! archive viewer, per-row clear/delete on both the live log and the archive, and
//! the panel's filter/expansion state. Split out of `state.rs`.

use super::{ArchivedTarget, ScrollbackTarget, Tty};
use crate::history;

impl Tty {
    /// Toggle the scrollback history panel; closing clears its filter so reopening
    /// starts fresh.
    pub fn toggle_scrollback_panel(&mut self) {
        self.show_scrollback = !self.show_scrollback;
        if !self.show_scrollback {
            self.scrollback_query.clear();
            self.scrollback_selected = None;
            self.scrollback_scroll = 0.0;
            self.scrollback_expanded.clear();
            self.scrollback_archived.clear();
            self.scrollback_archive_cursor = None;
        }
    }

    /// Page the Scrollback History panel one day older into the encrypted
    /// archive, prepending that day's entries before whatever's already
    /// paged in. A no-op if the feature is off, or if there's nothing older
    /// (`history::page_older` itself warns and returns `None` on a read
    /// failure — this never panics, it just doesn't add anything that time).
    pub fn page_scrollback_older(&mut self) {
        let Some((_, keys)) = self.history_read.as_ref() else {
            return;
        };
        let Some((date, mut entries)) = history::page_older(keys, self.scrollback_archive_cursor)
        else {
            return;
        };
        entries.append(&mut self.scrollback_archived);
        self.scrollback_archived = entries;
        self.scrollback_archive_cursor = Some(date);
    }

    /// Page the Scrollback History panel one day newer, back toward the
    /// present — the inverse of `page_scrollback_older`. Purely local: drops
    /// the oldest paged-in day's entries from `scrollback_archived` (always
    /// at the front, since entries are oldest-first) and moves the cursor to
    /// whichever day is now oldest among what's left, or `None` if that
    /// empties the list (fully back to the live view). Unlike paging older,
    /// this never touches disk — undoing a page-in only means forgetting
    /// what was already loaded, not fetching anything new.
    pub fn page_scrollback_newer(&mut self) {
        let Some(cursor) = self.scrollback_archive_cursor else {
            return;
        };
        self.scrollback_archived
            .retain(|e| history::local_date_from_epoch_ms(e.started_at_epoch_ms) != cursor);
        self.scrollback_archive_cursor = self
            .scrollback_archived
            .first()
            .map(|e| history::local_date_from_epoch_ms(e.started_at_epoch_ms));
    }

    /// Open the settings History section's read-only archive viewer, loading
    /// the most recent day if nothing is paged in yet. Callers gate this
    /// behind re-auth (see `update`'s `ToggleSettingsHistoryViewer` handler) —
    /// it shows the same protected data as the panel.
    pub fn open_settings_history_viewer(&mut self) {
        self.show_settings_history = true;
        if self.settings_history.is_empty() {
            self.page_settings_history_older();
        }
    }

    /// Close the settings archive viewer and drop everything it paged in —
    /// decrypted history doesn't linger in memory behind a closed view.
    pub fn close_settings_history_viewer(&mut self) {
        self.show_settings_history = false;
        self.settings_history.clear();
        self.settings_history_cursor = None;
        self.settings_history_selected = None;
        self.settings_history_scroll = 0.0;
        self.confirm_delete_settings_row = None;
    }

    /// Open the per-row "Delete this command?" confirmation for a viewer row.
    pub fn request_delete_settings_history_row(&mut self, target: ArchivedTarget) {
        self.close_menu();
        self.confirm_delete_settings_row = Some(target);
    }

    /// Dismiss the per-row delete confirmation without touching anything.
    pub fn cancel_delete_settings_history_row(&mut self) {
        self.confirm_delete_settings_row = None;
    }

    /// The per-row delete confirmation's "Delete" — tombstone the entry via
    /// the background writer ([`Self::delete_archived_target`], which also
    /// drops it from both surfaces' paged-in copies).
    pub fn confirm_delete_settings_history_row(&mut self) {
        if let Some(target) = self.confirm_delete_settings_row.take() {
            self.delete_archived_target(&target);
        }
    }

    /// Page the settings archive viewer one day older — the viewer's own
    /// counterpart of [`Self::page_scrollback_older`], with its own cursor so
    /// the panel and the viewer never fight over one.
    pub fn page_settings_history_older(&mut self) {
        let Some((_, keys)) = self.history_read.as_ref() else {
            return;
        };
        let Some((date, mut entries)) = history::page_older(keys, self.settings_history_cursor)
        else {
            return;
        };
        entries.append(&mut self.settings_history);
        self.settings_history = entries;
        self.settings_history_cursor = Some(date);
    }

    /// Blank an archived row's command text in place, straight to the
    /// background writer (there is no in-memory `CommandEntry` for a paged-in
    /// entry to mutate) — the archive counterpart of
    /// [`Self::clear_scrollback_target`]. Also updates both surfaces'
    /// paged-in copies (the panel's and the settings viewer's) so they
    /// reflect it without waiting for a re-page.
    pub fn clear_archived_target(&mut self, target: &ArchivedTarget) {
        let Some(writer) = self.history_writer.as_ref() else {
            return;
        };
        writer.send(cathode::history::HistoryEvent::Upsert(
            cathode::history::PersistedCommandEntry {
                id: target.id,
                command: String::new(),
                started_at_epoch_ms: target.started_at_epoch_ms,
                pane_tag: target.pane_tag.clone(),
            },
        ));
        for list in [&mut self.scrollback_archived, &mut self.settings_history] {
            if let Some(entry) = list.iter_mut().find(|e| e.id == target.id) {
                entry.command.clear();
            }
        }
    }

    /// Permanently remove an archived row, straight to the background writer
    /// — the archive counterpart of [`Self::delete_scrollback_target`]. Also
    /// drops it from both surfaces' paged-in copies so the panel and the
    /// settings viewer reflect it immediately.
    pub fn delete_archived_target(&mut self, target: &ArchivedTarget) {
        let Some(writer) = self.history_writer.as_ref() else {
            return;
        };
        writer.send(cathode::history::HistoryEvent::Tombstone {
            id: target.id,
            started_at_epoch_ms: target.started_at_epoch_ms,
        });
        self.scrollback_archived.retain(|e| e.id != target.id);
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
        self.settings_history.retain(|e| e.id != target.id);
        self.settings_history_selected = None;
    }

    /// Update the scrollback panel's filter — a new query invalidates the row
    /// selection and any expanded commands (both index into the filtered list,
    /// which just changed).
    pub fn set_scrollback_query(&mut self, query: String) {
        self.scrollback_query = query;
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
    }

    /// Toggle whether a command (its index into the *filtered* command list) shows
    /// its output.
    pub fn toggle_scrollback_expand(&mut self, index: usize) {
        if !self.scrollback_expanded.remove(&index) {
            self.scrollback_expanded.insert(index);
        }
    }

    /// Drop the active pane's buffered scrollback (the live on-screen grid is
    /// untouched — this is "clear history," not the shell's own `clear`).
    pub fn clear_active_scrollback(&mut self) {
        if let Some(term) = self.active_term() {
            term.screen.lock().clear_scrollback();
        }
    }

    /// Empty a single Scrollback History row's value in place (the row stays, its
    /// text goes blank) — the active pane's per-row "Clear" menu item, as opposed
    /// to [`Self::clear_active_scrollback`]'s wholesale wipe.
    pub fn clear_scrollback_target(&mut self, target: &ScrollbackTarget) {
        let Some(term) = self.active_term() else {
            return;
        };
        let mut screen = term.screen.lock();
        match *target {
            ScrollbackTarget::Command { log_index, .. } => screen.clear_command_output(log_index),
            ScrollbackTarget::Output {
                log_index, line, ..
            } => screen.clear_command_output_line(log_index, line),
        }
    }

    /// Permanently remove a Scrollback History command entry (its header row and
    /// all captured output) — the active pane's "Delete" menu item, unlike
    /// [`Self::clear_scrollback_target`]'s "blank the value, keep the row". Only
    /// applies to a `Command` target (no-op on an `Output` line — there's nothing
    /// sensible to "delete" for a single captured line, just clear it). Deleting
    /// shifts every later command's index, so the panel's selection/expand state
    /// (both indices into the row list that just changed) resets, mirroring
    /// [`Self::set_scrollback_query`]'s reasoning.
    pub fn delete_scrollback_target(&mut self, target: &ScrollbackTarget) {
        let ScrollbackTarget::Command { log_index, .. } = *target else {
            return;
        };
        let Some(term) = self.active_term() else {
            return;
        };
        term.screen.lock().remove_command(log_index);
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
    }
}
