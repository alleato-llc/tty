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
            if state.modifiers.control() && state.main_window == Some(win) {
                state.open_pane_menu(pane);
            } else {
                state.focus_pane(win, pane);
            }
        }
        Message::ResizeSplit(win, e) => state.resize_split(win, e.split, e.ratio),
        Message::PointerMoved(p) => state.pointer = p,
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
        Message::CopyLink(url) => {
            state.close_menu();
            return iced::clipboard::write(url);
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
        Message::ClearScrollback => state.clear_active_scrollback(),
        Message::ToggleScrollbackPanel => state.toggle_scrollback_panel(),
        Message::ScrollbackQueryChanged(q) => state.set_scrollback_query(q),
        Message::ScrollbackRowSelected(row) => state.scrollback_selected = Some(row),
        Message::ScrollbackRowActivated(row, text) => {
            state.scrollback_selected = Some(row);
            return iced::clipboard::write(text);
        }
        Message::ScrollbackToggleExpand(i) => state.toggle_scrollback_expand(i),
        Message::ScrollbackScrolled(offset) => state.scrollback_scroll = offset,
        Message::MaxScrollbackStep(delta) => state.step_max_scrollback(delta),
        Message::DefaultOutputLinesStep(delta) => state.step_default_output_lines(delta),
        Message::NewTab => {
            state.close_menu();
            state.new_tab();
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
        Message::SetTheme(name) => state.set_theme(&name),
        Message::SetFont(family) => state.set_font(&family),
        Message::FontSizeStep(delta) => state.step_font_size(delta),
        Message::Base16Changed(s) => state.base16_input = s,
        Message::ApplyBase16 => state.apply_base16(),
        Message::ResetPalette => state.reset_palette(),
        Message::EditColor(idx, color) => state.edit_color(idx, color),
        Message::Focused(f) => state.focused = f,
        Message::SetUnfocusedOpacity(o) => state.set_unfocused_opacity(o),
        Message::SetTabHighlight(on) => state.set_tab_highlight(on),

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
        }
        Message::WindowMoved(id, pos) => crate::detach_drag::on_moved(state, id, pos),
        Message::WindowResizedAt(id, size) => {
            if state.main_window == Some(id) {
                state.window_height = size.height;
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
            if let Some(task) = state.finish_tab_drag() {
                return task;
            }
        }
    }
    iced::Task::none()
}

fn handle_key(state: &mut Tty, key: Key, mods: Modifiers) -> iced::Task<Message> {
    // Escape closes the rename field / settings panel / find bar (when open) instead of
    // going to the shell.
    if matches!(key, Key::Named(iced::keyboard::key::Named::Escape)) {
        if state.renaming.is_some() {
            state.cancel_rename();
            return iced::Task::none();
        }
        if state.show_settings {
            state.show_settings = false;
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
        }
    }
    // App chords use the platform *command* modifier (⌘ on macOS) so Ctrl stays a
    // real terminal control code (Ctrl+C, Ctrl+D, …) sent to the shell.
    if mods.command() {
        if let Key::Character(s) = &key {
            match s.as_str() {
                // Tab/settings/find chrome lives only in the main window, so a detached
                // window ignores these (a no-op rather than acting on the hidden strip).
                "t" | "n" if is_main => {
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
                // ⌘⇧H opens/closes the scrollback history panel (main window only).
                s if is_main && mods.shift() && s.eq_ignore_ascii_case("h") => {
                    state.toggle_scrollback_panel();
                    return iced::Task::none();
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
