use iced::keyboard::{Key, Modifiers};

use crate::message::Message;
use crate::state::Tty;

pub fn update(state: &mut Tty, message: Message) -> iced::Task<Message> {
    match message {
        Message::Key(key, mods) => return handle_key(state, key, mods),
        Message::ModifiersChanged(mods) => state.modifiers = mods,
        Message::Resize(win, pane, cols, rows) => state.resize_pane(win, pane, cols, rows),
        Message::Select(win, pane, text) => {
            // Only the focused pane of the reporting window's tab feeds ⌘C; ignore stray
            // drags elsewhere (incl. background windows).
            if state.tab_for(win).map(|t| t.focus) == Some(pane) {
                state.selection = text;
            }
        }
        Message::PtyBytes(win, pane, bytes) => state.write_pane(win, pane, &bytes),
        // A plain click focuses the pane; Ctrl+click is macOS's secondary-click (it
        // arrives as Left+Control, not a right button), so treat it as "open the menu" —
        // but only in the main window (detached windows carry no context menu in v1).
        Message::FocusPane(win, pane) => {
            if state.pane_replace_pending.is_some() {
                // Replace-pick mode: this click chooses the pane to replace.
                state.request_pane_replace(win, pane);
            } else if state.modifiers.control() && state.main_window == Some(win) {
                state.open_pane_menu(pane);
            } else {
                state.focus_pane(win, pane);
            }
        }
        Message::ResizeSplit(win, e) => state.resize_split(win, e.split, e.ratio),
        Message::PointerMoved(p) => {
            state.pointer = p;
            // A popover drag is either a resize (an edge / corner) or a move
            // (body); resize takes precedence if both somehow started. Deltas are
            // from where the drag began, clamped to stay usable / on-screen. Only
            // the grabbed edge's axes move; the other keeps its starting value.
            // Both address a popover by its index in `metric_details`.
            let (ww, wh) = (state.window_width, state.window_height);
            if let Some((i, start, (bw, bh), edge)) = state.metric_detail_resize {
                if let Some(pop) = state.metric_details.get_mut(i) {
                    let (h_ax, v_ax) = edge.axes();
                    let w = if h_ax {
                        (bw + (p.x - start.x)).clamp(280.0, (ww - 40.0).max(280.0))
                    } else {
                        bw
                    };
                    let h = if v_ax {
                        (bh + (p.y - start.y)).clamp(110.0, (wh - 150.0).max(110.0))
                    } else {
                        bh
                    };
                    pop.size = Some((w, h));
                }
            } else if let Some((i, start, (bx, by))) = state.metric_detail_move_drag {
                if let Some(pop) = state.metric_details.get_mut(i) {
                    pop.move_offset = (bx + (p.x - start.x), by + (p.y - start.y));
                }
            }
        }
        Message::PaneRightClick(pane) => state.open_pane_menu(pane),
        Message::TabRightClick(idx) => state.open_tab_menu(idx),
        Message::LinkClick(url) => {
            state.menu = Some((crate::state::MenuKind::Link(url), state.pointer))
        }
        Message::OpenLink(url) => {
            if let Err(e) = phosphor::link::open(&url) {
                tracing::warn!("Failed to open link {url:?}: {e}");
            }
            state.close_menu();
        }
        Message::OpenFile(path, line, col) => {
            let argv = crate::settings::resolve_open_file_command(
                state.settings.open_file_command.as_deref(),
                &path,
                line,
                col,
            );
            if let Some((cmd, args)) = argv.split_first() {
                if let Err(e) = std::process::Command::new(cmd).args(args).spawn() {
                    tracing::warn!("Failed to open file {path:?} via {argv:?}: {e}");
                }
            }
        }
        Message::CopyLink(url) => {
            state.close_menu();
            return iced::clipboard::write(url);
        }
        Message::CopyLastCommandOutput => {
            state.close_menu();
            if let Some(win) = state.keyboard_window() {
                if let Some(text) = state.last_command_output(win) {
                    return iced::clipboard::write(text);
                }
            }
        }
        Message::Split(dir) => {
            // The context menu is main-window only; split the main active tab.
            if let Some(main) = state.main_window {
                state.split_focused(main, dir);
            }
            state.close_menu();
        }
        Message::ClosePane => {
            state.close_menu();
            if !state.close_focused_pane() {
                return iced::exit();
            }
        }
        Message::CloseMenu => state.close_menu(),
        Message::StartRename(idx) => {
            state.start_rename(idx);
            // Focus the rename field so the user can type immediately.
            return iced::widget::operation::focus(crate::view::rename_id());
        }
        Message::RenameChanged(text) => state.set_rename_draft(text),
        Message::RenameSubmit => state.commit_rename(),
        Message::Pasted(Some(text)) => {
            // ⌘V resolved — paste into whichever window held focus when it was pressed.
            if let Some(win) = state.keyboard_window() {
                state.paste(win, &text);
            }
        }
        Message::Pasted(None) => {}
        Message::SearchChanged(q) => {
            state.search = Some(q);
            state.search_match = 0;
        }
        Message::SearchSubmit => {
            if state.modifiers.shift() {
                state.search_match -= 1;
            } else {
                state.search_match += 1;
            }
        }
        Message::ClearScrollback => {
            state.close_menu();
            state.clear_active_scrollback();
        }
        Message::ToggleScrollbackPanel => {
            state.close_menu();
            return toggle_scrollback_gated(state);
        }
        Message::HistoryReauthResult(unlock, true) => {
            state.history_reauth_pending = false;
            state.mark_history_authenticated();
            match unlock {
                crate::message::ReauthFor::ScrollbackPanel => state.toggle_scrollback_panel(),
                crate::message::ReauthFor::SettingsHistory => state.open_settings_history_viewer(),
            }
        }
        Message::HistoryReauthResult(_, false) => state.history_reauth_pending = false,
        Message::ToggleSettingsHistoryViewer => {
            if state.show_settings_history {
                state.close_settings_history_viewer();
            } else if !state.history_reauth_pending {
                if let Some(reason) = state.history_reauth_reason() {
                    state.history_reauth_pending = true;
                    return iced::Task::perform(
                        crate::history::reauth::authenticate(reason),
                        |ok| {
                            Message::HistoryReauthResult(
                                crate::message::ReauthFor::SettingsHistory,
                                ok,
                            )
                        },
                    );
                }
                state.open_settings_history_viewer();
            }
        }
        Message::SettingsHistoryPageOlder => state.page_settings_history_older(),
        Message::SettingsHistoryScrolled(offset) => state.settings_history_scroll = offset,
        Message::SettingsHistoryRowSelected(row) => state.settings_history_selected = Some(row),
        Message::SettingsHistoryRowActivated(row, text) => {
            state.settings_history_selected = Some(row);
            return iced::clipboard::write(text);
        }
        Message::SettingsHistoryRowRightClick(row, target) => {
            state.settings_history_selected = Some(row);
            state.menu = Some((
                crate::state::MenuKind::SettingsHistoryRow(target),
                state.pointer,
            ));
        }
        Message::RequestDeleteSettingsHistoryRow(target) => {
            state.request_delete_settings_history_row(target)
        }
        Message::CancelDeleteSettingsHistoryRow => state.cancel_delete_settings_history_row(),
        Message::ConfirmDeleteSettingsHistoryRow => state.confirm_delete_settings_history_row(),
        Message::CopyText(text) => {
            state.close_menu();
            return iced::clipboard::write(text);
        }
        Message::ScrollbackQueryChanged(q) => state.set_scrollback_query(q),
        Message::ScrollbackRowSelected(row) => state.scrollback_selected = Some(row),
        Message::ScrollbackRowActivated(row, text) => {
            state.scrollback_selected = Some(row);
            return iced::clipboard::write(text);
        }
        Message::ScrollbackToggleExpand(i) => state.toggle_scrollback_expand(i),
        Message::ScrollbackRowRightClick(row, target) => {
            state.scrollback_selected = Some(row);
            state.menu = Some((crate::state::MenuKind::ScrollbackRow(target), state.pointer));
        }
        Message::CopyScrollbackTarget(target) => {
            state.close_menu();
            // Copying a *command* strips its captured shell prompt (the row
            // is the full echoed line); an output line is copied verbatim —
            // its text is data, not a prompt.
            let text = match &target {
                crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Output {
                    text,
                    ..
                }) => text.as_str(),
                other => cathode::commands::strip_prompt(other.text()),
            };
            return iced::clipboard::write(text.to_string());
        }
        Message::ClearScrollbackTarget(target) => {
            state.close_menu();
            match target {
                crate::state::HistoryRowTarget::Live(t) => state.clear_scrollback_target(&t),
                crate::state::HistoryRowTarget::Archived(t) => state.clear_archived_target(&t),
            }
        }
        Message::DeleteScrollbackTarget(target) => {
            state.close_menu();
            match target {
                crate::state::HistoryRowTarget::Live(t) => state.delete_scrollback_target(&t),
                crate::state::HistoryRowTarget::Archived(t) => state.delete_archived_target(&t),
            }
        }
        Message::ScrollbackPageOlder => state.page_scrollback_older(),
        Message::ScrollbackPageNewer => state.page_scrollback_newer(),
        Message::ScrollbackScrolled(offset) => state.scrollback_scroll = offset,
        Message::OpenFileCommandChanged(s) => state.set_open_file_command(s),
        Message::SetNotifyOnCommandFinish(on) => state.set_notify_on_command_finish(on),
        Message::NotifyMinSecondsStep(delta) => state.step_notify_min_seconds(delta),
        Message::SetShellIntegrationAutoinstall(on) => state.set_shell_integration_autoinstall(on),
        Message::CopyShellSnippet => {
            return iced::clipboard::write(crate::shell_integration::ZSH_SNIPPET.to_string());
        }
        Message::MaxScrollbackStep(delta) => state.step_max_scrollback(delta),
        Message::DefaultOutputLinesStep(delta) => state.step_default_output_lines(delta),
        Message::NewTab => {
            state.close_menu();
            state.new_tab();
        }
        Message::NewUntrackedTab => {
            state.close_menu();
            state.new_tab_with(true);
        }
        Message::CloseTab(idx) => {
            state.close_menu();
            if !state.close_tab(idx) {
                return iced::exit();
            }
        }
        // Plain click activates the tab and arms a possible tear-off detach; Ctrl+click
        // (macOS secondary-click) opens its menu.
        Message::ActivateTab(idx) => {
            if state.modifiers.control() {
                state.open_tab_menu(idx);
            } else {
                state.activate(idx);
                state.tab_drag = Some((idx, state.pointer));
            }
        }
        Message::HoverTab(i) => {
            state.hovered_tab = i;
            // If a tab press is being dragged across the strip, reorder live.
            if let Some(target) = i {
                state.reorder_dragged_tab(target);
            }
        }
        Message::Tick => {
            // Surface any OSC 52 clipboard request and light background-activity dots.
            let clip = state.drain_effects();
            // Reap tabs whose shell exited across all windows; close any detached window
            // whose tab fully died, and quit when nothing remains anywhere.
            let (any, closed) = state.reap_dead();
            let mut tasks: Vec<iced::Task<Message>> =
                closed.into_iter().map(iced::window::close).collect();
            if !any {
                return iced::exit();
            }
            if let Some(text) = clip {
                tasks.push(iced::clipboard::write(text));
            }
            return iced::Task::batch(tasks);
        }
        Message::ToggleSettings => state.toggle_settings(),
        Message::SettingsSection(i) => state.settings_section = i,
        Message::AppearanceTab(i) => state.appearance_tab = i,
        Message::SetTheme(name) => state.set_theme(&name),
        Message::SetFont(family) => state.set_font(&family),
        Message::FontSizeStep(delta) => state.step_font_size(delta),
        Message::Base16Changed(s) => state.base16_input = s,
        Message::ApplyBase16 => state.apply_base16(),
        Message::ResetPalette => state.reset_palette(),
        Message::EditColor(idx, color) => state.edit_color(idx, color),
        Message::Focused(f) => state.focused = f,
        Message::SetUnfocusedOpacity(o) => state.set_unfocused_opacity(o),
        Message::SetFocusedOpacity(o) => state.set_focused_opacity(o),
        Message::SetWindowAlwaysOnTop(on) => {
            state.set_window_always_on_top(on);
            // Apply the new level to every live window (main + detached).
            let level = state.window_level();
            return iced::Task::batch(
                state
                    .all_window_ids()
                    .into_iter()
                    .map(|id| iced::window::set_level(id, level)),
            );
        }
        Message::SetTabHighlight(on) => state.set_tab_highlight(on),
        Message::SetGraduateMetrics(on) => state.set_graduate_metrics(on),
        Message::SetHighlightFocusedPane(on) => state.set_highlight_focused_pane(on),
        Message::SetStatusBarAutohide(on) => state.set_status_bar_autohide(on),
        Message::SetStatusBarDisabled(on) => state.set_status_bar_disabled(on),
        Message::SetStatusBarMetricsPinned(on) => state.set_status_bar_metrics_pinned(on),
        Message::StatusBarScroll(dy) => {
            // Wheel down slides the window toward the later cells; up, back.
            let max = crate::view::status_bar_scroll_max(state);
            if dy < 0.0 {
                state.status_bar_scroll = (state.status_bar_scroll + 1).min(max);
            } else if dy > 0.0 {
                state.status_bar_scroll = state.status_bar_scroll.saturating_sub(1).min(max);
            }
        }
        Message::StatusMetricPress(idx) => state.press_status_metric(idx),
        Message::StatusBarEditTick => state.check_status_metric_hold(),
        Message::StatusMetricDragOver(idx) => state.drag_status_metric_over(idx),
        Message::ExitStatusBarEdit => state.exit_status_bar_edit(),
        Message::SetStatusBarEditHold(delta) => state.step_status_bar_edit_hold(delta),
        Message::SetProcSort(col) => state.set_proc_sort(col),
        Message::ProcTableScroll(offset) => state.set_proc_scroll(offset),
        Message::OpenProcDetail(pid) => {
            // Reached from the process row's context menu — dismiss it, else its
            // full-window backdrop keeps swallowing clicks (e.g. right-clicks on
            // the detail's fd rows).
            state.close_menu();
            state.open_proc_detail(pid);
            // Sample once now so the detail has data before the next tick.
            state.metrics.sample_proc_detail(pid);
        }
        Message::CloseProcDetail => state.close_proc_detail(),
        Message::ProcRowRightClick(pid, name) => {
            state.menu = Some((crate::state::MenuKind::ProcRow { pid, name }, state.pointer));
        }
        Message::FdRowRightClick(path) => {
            state.menu = Some((crate::state::MenuKind::FdRow { path }, state.pointer));
        }
        Message::CopyProcPath(pid) => {
            state.close_menu();
            if let Some(path) = crate::metrics::process_path(pid) {
                return iced::clipboard::write(path);
            }
        }
        Message::KillProcess(pid, sig) => {
            state.close_menu();
            if !crate::metrics::kill_process(pid, sig) {
                tracing::warn!("failed to signal pid {pid} with {sig}");
            }
        }
        Message::RequestForceKill(pid, name) => {
            state.close_menu();
            state.kill_confirm = Some((pid, name));
        }
        Message::ConfirmForceKill => {
            if let Some((pid, _)) = state.kill_confirm.take() {
                if !crate::metrics::kill_process(pid, crate::metrics::SIG_KILL) {
                    tracing::warn!("failed to force-kill pid {pid}");
                }
            }
        }
        Message::CancelForceKill => state.kill_confirm = None,
        Message::PromotePopoverMenu(kind) => {
            state.menu = Some((
                crate::state::MenuKind::PromotePopover { kind },
                state.pointer,
            ));
        }
        Message::PromoteMetricToPane(kind, dir) => {
            state.close_menu();
            // Drop the floating popover(s) — the view lives in the grid now.
            state.metric_details.clear();
            if let Some(main) = state.main_window {
                state.promote_metric_to_pane(main, dir, kind);
            }
        }
        Message::StartPaneReplace(kind) => {
            state.close_menu();
            state.start_pane_replace(kind);
        }
        Message::ConfirmPaneReplace => {
            if let Some((win, pane, kind)) = state.pane_replace_confirm.take() {
                state.replace_pane(win, pane, kind);
            }
        }
        Message::CancelPaneReplace => state.cancel_pane_replace(),
        Message::ToggleMaximizePane(win) => state.toggle_maximize_pane(win),
        Message::CloseMetricPane(win, pane) => {
            state.close_pane(win, pane);
        }
        Message::SetClock24h(on) => state.set_clock_24h(on),
        Message::SetClockSeconds(on) => state.set_clock_seconds(on),
        Message::SetClockDate(on) => state.set_clock_date(on),
        // The tick exists only to trigger a repaint; the clock reads the live
        // time at render.
        Message::ClockTick => {}
        Message::StatusBarMetricAdd(metric) => state.add_status_bar_metric(&metric),
        Message::StatusBarMetricRemove(idx) => state.remove_status_bar_metric(idx),
        Message::StatusBarMetricMove(idx, delta) => state.move_status_bar_metric(idx, delta),
        Message::StatusBarMetricStyle(idx, style) => state.set_status_bar_metric_style(idx, &style),
        Message::StatusBarMetricThreshold(idx, warn, delta) => {
            state.step_status_bar_metric_threshold(idx, warn, delta)
        }
        Message::SampleMetrics => {
            state.metrics.sample();
            // The process table is heavier (walks every pid), so only poll it
            // while a Processes cell is actually shown.
            if state
                .settings
                .status_bar_metrics()
                .iter()
                .any(|m| m.kind == crate::settings::MetricKind::Procs)
            {
                state.metrics.sample_processes();
                // Refresh the open per-process detail (fds + its live chart).
                if let Some(pid) = state.proc_detail_pid {
                    state.metrics.sample_proc_detail(pid);
                }
            }
        }
        Message::CloseMetricDetail => {
            // Click-away / Escape: close every open popover and drop any drag.
            state.metric_details.clear();
            state.metric_detail_resize = None;
            state.metric_detail_move_drag = None;
            state.close_proc_detail();
        }
        Message::CloseMetricPopover(i) => {
            if i < state.metric_details.len() {
                state.metric_details.remove(i);
                // Indices shift on remove; cancel any in-flight drag to be safe.
                state.metric_detail_resize = None;
                state.metric_detail_move_drag = None;
                state.close_proc_detail();
            }
        }
        Message::ToggleMetricDetailExpanded(i) => {
            if let Some(pop) = state.metric_details.get_mut(i) {
                pop.expanded = !pop.expanded;
                // Snap to the new state's default size/position; drags re-customize.
                pop.size = None;
                pop.move_offset = (0.0, 0.0);
            }
        }
        Message::MetricDetailResizeStart(i, edge) => {
            let (ww, wh) = (state.window_width, state.window_height);
            if let Some(pop) = state.metric_details.get(i) {
                state.metric_detail_resize =
                    Some((i, state.pointer, pop.effective_size(ww, wh), edge));
            }
        }
        Message::MetricDetailMoveStart(i) => {
            if let Some(pop) = state.metric_details.get(i) {
                state.metric_detail_move_drag = Some((i, state.pointer, pop.move_offset));
            }
        }
        Message::SetEncryptedHistoryEnabled(true) => state.request_enable_encrypted_history(),
        Message::SetEncryptedHistoryEnabled(false) => state.disable_encrypted_history(),
        Message::ConfirmEnableHistory => {
            // The enable dialog's "Continue", keychain source: the dialog
            // closes and the keychain start begins.
            state.cancel_passphrase_prompt();
            return state.begin_history_start(crate::message::HistoryStartOrigin::Enable);
        }
        Message::HistoryStarted(origin, outcome) => state.apply_history_started(origin, outcome),
        Message::SetHistoryKeySource(source) => state.set_history_key_source(source),
        Message::SetHistoryKdf(kdf) => state.set_history_kdf(kdf),
        Message::SetHistoryFanout(fanout) => state.set_history_fanout(fanout),
        Message::OpenHistoryUnlock => state.open_history_unlock(),
        Message::HistoryPassphraseChanged(text) => state.set_passphrase_draft(text),
        Message::HistoryPassphraseConfirmChanged(text) => state.set_passphrase_confirm(text),
        Message::SubmitHistoryPassphrase => return state.submit_passphrase(),
        Message::CancelHistoryPassphrase => state.cancel_passphrase_prompt(),
        Message::SessionStartChoice(record) => return state.choose_session_start(record),
        Message::SetHistorySessionStart(mode) => state.set_history_session_start(mode),
        Message::SetHistoryCipher(cipher) => state.set_history_cipher(cipher),
        Message::RequestResetEncryptedHistory => state.request_reset_encrypted_history(),
        Message::CancelResetEncryptedHistory => state.cancel_reset_encrypted_history(),
        Message::ConfirmResetEncryptedHistory => return state.confirm_reset_encrypted_history(),
        Message::HistoryReauthIntervalStep(delta) => {
            state.step_history_reauth_interval_minutes(delta)
        }

        // ---- multi-window: detachable tabs (ADR 0003) ----
        Message::DetachTab(idx) => {
            if let Some(task) = state.detach_tab(idx) {
                return task;
            }
        }
        Message::ReattachTab(id) => {
            // Dock the tab back, THEN close its window. Removing it from `detached` first
            // means the ensuing `WindowClosed` finds nothing to reattach (no-op).
            state.reattach_window(id);
            return iced::window::close(id);
        }
        Message::WindowFocused(id) => {
            // Any tty window gaining focus makes the app "focused" (drives the global
            // unfocused-opacity fade) and routes the keyboard to that window's tab.
            state.focused = true;
            state.focus_window(id);
            // Live-reload: if tty.toml was hand-edited in another app while we were
            // away, adopt it now (no-op when unchanged / right after our own save).
            if state.reload_settings_if_changed() {
                tracing::info!("reloaded settings from tty.toml (external change)");
            }
        }
        Message::WindowMoved(id, pos) => crate::detach_drag::on_moved(state, id, pos),
        Message::WindowResizedAt(id, size) => {
            if state.main_window == Some(id) {
                state.window_height = size.height;
                state.window_width = size.width;
            }
            crate::detach_drag::on_resized(state, id, size);
        }
        Message::WindowPosition(id, pos) => {
            if let Some(p) = pos {
                crate::detach_drag::set_position(state, id, p);
            }
        }
        Message::CheckDragReattach => {
            if let crate::detach_drag::Settle::Reattach(id) = crate::detach_drag::poll_settle(state)
            {
                state.reattach_window(id);
                return iced::window::close(id);
            }
        }
        Message::WindowClosed(id) => {
            // The daemon keeps running after its last window closes, so closing the main
            // window must explicitly exit (tearing down every detached window + its
            // shell). An OS-close of a detached window docks its tab back.
            if state.main_window == Some(id) {
                return iced::exit();
            }
            state.reattach_window(id);
        }
        Message::PointerReleased => {
            state.metric_detail_resize = None;
            state.metric_detail_move_drag = None;
            // A metric drag commits its reorder; a quick tap opens the drill-in.
            if let Some(idx) = state.release_status_metric() {
                if let Some(key) = state
                    .settings
                    .status_bar_metrics
                    .get(idx)
                    .map(|c| c.metric.clone())
                {
                    state.open_metric_detail(&key);
                }
            }
            if let Some(task) = state.finish_tab_drag() {
                return task;
            }
        }
    }
    iced::Task::none()
}

/// Open/close the Scrollback History panel through the re-auth gate — the one
/// shared entry point for every open path (the ⌘⇧H chord and the pane menu's
/// "View Scrollback History"), so no path can skip the Touch ID/password
/// check. Closing never prompts; opening prompts only when a check is due
/// (see `Tty::history_reauth_reason`), deferring the actual open to the
/// `HistoryReauthResult(true)` handler.
fn toggle_scrollback_gated(state: &mut Tty) -> iced::Task<Message> {
    if !state.show_scrollback {
        if state.history_reauth_pending {
            // A prompt is already up — pressing the chord again while it's
            // showing must not stack a second one.
            return iced::Task::none();
        }
        if let Some(reason) = state.history_reauth_reason() {
            state.history_reauth_pending = true;
            return iced::Task::perform(crate::history::reauth::authenticate(reason), |ok| {
                Message::HistoryReauthResult(crate::message::ReauthFor::ScrollbackPanel, ok)
            });
        }
    }
    state.toggle_scrollback_panel();
    iced::Task::none()
}

fn handle_key(state: &mut Tty, key: Key, mods: Modifiers) -> iced::Task<Message> {
    // Escape closes the rename field / settings panel / find bar (when open) instead of
    // going to the shell.
    if matches!(key, Key::Named(iced::keyboard::key::Named::Escape)) {
        if state.kill_confirm.is_some() {
            state.kill_confirm = None;
            return iced::Task::none();
        }
        if state.pane_replace_pending.is_some() || state.pane_replace_confirm.is_some() {
            // Leave "replace a pane" pick mode / dismiss its confirm.
            state.cancel_pane_replace();
            return iced::Task::none();
        }
        if state.status_bar_edit {
            state.exit_status_bar_edit();
            return iced::Task::none();
        }
        if state.proc_detail_pid.is_some() {
            // A per-process detail is open: Escape steps back to the process list
            // (the popover itself stays open), matching the "‹ Back" control.
            state.close_proc_detail();
            return iced::Task::none();
        }
        if !state.metric_details.is_empty() {
            // Escape closes every open popover at once (in both modes).
            state.metric_details.clear();
            state.metric_detail_resize = None;
            state.metric_detail_move_drag = None;
            return iced::Task::none();
        }
        if state.renaming.is_some() {
            state.cancel_rename();
            return iced::Task::none();
        }
        if state.show_settings {
            // Through toggle_settings, so the archive viewer's paged-in
            // entries are dropped too.
            state.toggle_settings();
            return iced::Task::none();
        }
        if state.search.is_some() {
            state.search = None;
            return iced::Task::none();
        }
    }
    // Chords and typing act on the focused window's tab (the main strip, or a detached
    // window). `keyboard_window` falls back to the main window.
    let Some(win) = state.keyboard_window() else {
        return iced::Task::none();
    };
    let is_main = state.main_window == Some(win);

    // Pane chords: ⌥⌘ + arrow splits the focused pane toward that direction; ⌃⌘ + arrow
    // moves focus to the neighbour. Checked before the PTY fallthrough so the arrows
    // don't also reach the shell.
    if mods.command() {
        if let Key::Named(named) = &key {
            if let Some(dir) = arrow_direction(*named) {
                if mods.alt() {
                    state.split_focused(win, dir);
                    return iced::Task::none();
                }
                if mods.control() {
                    state.focus_dir(win, dir);
                    return iced::Task::none();
                }
            }
            // Plain ⌘↑ / ⌘↓ jumps to the previous / next command prompt (OSC 133).
            use iced::keyboard::key::Named;
            if !mods.alt() && !mods.control() {
                match named {
                    Named::ArrowUp => {
                        state.jump_to_prompt(win, true);
                        return iced::Task::none();
                    }
                    Named::ArrowDown => {
                        state.jump_to_prompt(win, false);
                        return iced::Task::none();
                    }
                    _ => {}
                }
            }
        }
    }
    // App chords use the platform *command* modifier (⌘ on macOS) so Ctrl stays a
    // real terminal control code (Ctrl+C, Ctrl+D, …) sent to the shell.
    if mods.command() {
        if let Key::Character(s) = &key {
            match s.as_str() {
                // ⌘⇧T: a new *untracked* tab — commands in it are never
                // written to encrypted history. Must precede the plain
                // new-tab arm (some platforms deliver a shifted chord as a
                // lowercase character + SHIFT).
                s if is_main && mods.shift() && s.eq_ignore_ascii_case("t") => {
                    state.new_tab_with(true);
                    return iced::Task::none();
                }
                // Tab/settings/find chrome lives only in the main window, so a detached
                // window ignores these (a no-op rather than acting on the hidden strip).
                "t" | "n" if is_main && !mods.shift() => {
                    state.new_tab();
                    return iced::Task::none();
                }
                "w" => {
                    // ⌘W closes the focused pane. In the main window the last pane closes
                    // the tab → quits; in a detached window the last pane closes the
                    // window (without reattaching).
                    if is_main {
                        if !state.close_focused_pane() {
                            return iced::exit();
                        }
                    } else if let Some(close) = state.close_detached_focused_pane(win) {
                        return iced::window::close(close);
                    }
                    return iced::Task::none();
                }
                // ⌘, opens/closes the settings panel (main window only).
                "," if is_main => {
                    state.toggle_settings();
                    return iced::Task::none();
                }
                // Zoom: ⌘+ / ⌘= grow, ⌘− shrink, ⌘0 reset. Global (one font size).
                "+" | "=" => {
                    state.zoom(1.0);
                    return iced::Task::none();
                }
                "-" => {
                    state.zoom(-1.0);
                    return iced::Task::none();
                }
                "0" => {
                    state.reset_zoom();
                    return iced::Task::none();
                }
                // ⌘C copies the selection (Ctrl+C stays SIGINT to the shell). Always
                // consumed so it never types a literal "c".
                "c" => {
                    return match &state.selection {
                        Some(text) => iced::clipboard::write(text.clone()),
                        None => iced::Task::none(),
                    };
                }
                // ⌘V pastes the system clipboard into the focused shell (read is async).
                "v" => return iced::clipboard::read().map(Message::Pasted),
                // ⌘F toggles the scrollback find bar (main window only); opening focuses
                // its field.
                "f" if is_main => {
                    return if state.toggle_search() {
                        iced::widget::operation::focus(crate::view::search_id())
                    } else {
                        iced::Task::none()
                    };
                }
                // ⌘K clears the active pane's scrollback (main window only, matching
                // ⌘F/⌘,'s scope) — the de facto macOS terminal convention.
                "k" if is_main => {
                    state.clear_active_scrollback();
                    return iced::Task::none();
                }
                // ⌘⇧H opens/closes the scrollback history panel (main window only) —
                // through the same re-auth gate as the menu path.
                s if is_main && mods.shift() && s.eq_ignore_ascii_case("h") => {
                    return toggle_scrollback_gated(state);
                }
                // ⌘⇧O copies the most recent command's output (OSC 133) to the clipboard.
                s if mods.shift() && s.eq_ignore_ascii_case("o") => {
                    return update(state, Message::CopyLastCommandOutput);
                }
                d if is_main && d.len() == 1 && d.starts_with(|c: char| c.is_ascii_digit()) => {
                    let n = d.parse::<usize>().unwrap_or(0);
                    if (1..=state.tabs.len()).contains(&n) {
                        state.activate(n - 1);
                    }
                    return iced::Task::none();
                }
                _ => {}
            }
        }
    }
    // A plain Enter submits a command — mark the boundary before it's sent, so the
    // scrollback history panel can separate this command from its output (a no-op on
    // the alt screen — see `TerminalScreen::mark_command_boundary`).
    if matches!(key, Key::Named(iced::keyboard::key::Named::Enter)) {
        state.mark_command_boundary(win);
    }
    // Otherwise the keystroke is terminal input (arrow keys honor the app's DECCKM mode).
    if let Some(bytes) = phosphor::input::to_bytes(&key, mods, state.app_cursor_for(win)) {
        // Typing at the shell returns focus to the live bottom, so prompt-jump restarts
        // from the newest prompt next time.
        state.clear_prompt_jump();
        state.write_focused(win, &bytes);
    }
    iced::Task::none()
}

/// Map an arrow key to a `pane_grid` direction (for the split / focus chords).
fn arrow_direction(
    named: iced::keyboard::key::Named,
) -> Option<iced::widget::pane_grid::Direction> {
    use iced::keyboard::key::Named;
    use iced::widget::pane_grid::Direction;
    Some(match named {
        Named::ArrowLeft => Direction::Left,
        Named::ArrowRight => Direction::Right,
        Named::ArrowUp => Direction::Up,
        Named::ArrowDown => Direction::Down,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
