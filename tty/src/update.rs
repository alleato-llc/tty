use iced::keyboard::{Key, Modifiers};

use crate::message::Message;
use crate::state::Tty;

pub fn update(state: &mut Tty, message: Message) -> iced::Task<Message> {
    match message {
        Message::Key(key, mods) => return handle_key(state, key, mods),
        Message::ModifiersChanged(mods) => state.modifiers = mods,
        Message::Resize(cols, rows) => state.resize_active(cols, rows),
        Message::Select(text) => state.selection = text,
        Message::PtyBytes(bytes) => state.write_active(&bytes),
        Message::Pasted(Some(text)) => state.paste(&text),
        Message::Pasted(None) => {}
        Message::SearchChanged(q) => state.search = Some(q),
        Message::SearchSubmit => state.search = None,
        Message::NewTab => state.new_tab(),
        Message::CloseTab(idx) => {
            if !state.close_tab(idx) {
                return iced::exit();
            }
        }
        Message::ActivateTab(idx) => state.activate(idx),
        Message::HoverTab(i) => state.hovered_tab = i,
        Message::Tick => {
            // Surface any OSC 52 clipboard request and light background-activity dots.
            let clip = state.drain_effects();
            // Reap tabs whose shell exited (`exit`); quit when the last one goes.
            if !state.reap_dead() {
                return iced::exit();
            }
            if let Some(text) = clip {
                return iced::clipboard::write(text);
            }
        }
        Message::WindowResized(h) => state.window_height = h,
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
    }
    iced::Task::none()
}

fn handle_key(state: &mut Tty, key: Key, mods: Modifiers) -> iced::Task<Message> {
    // Escape closes the settings panel / find bar (when open) instead of going to the shell.
    if matches!(key, Key::Named(iced::keyboard::key::Named::Escape)) {
        if state.show_settings {
            state.show_settings = false;
            return iced::Task::none();
        }
        if state.search.is_some() {
            state.search = None;
            return iced::Task::none();
        }
    }
    // App chords use the platform *command* modifier (⌘ on macOS) so Ctrl stays a
    // real terminal control code (Ctrl+C, Ctrl+D, …) sent to the shell.
    if mods.command() {
        if let Key::Character(s) = &key {
            match s.as_str() {
                "t" | "n" => {
                    state.new_tab();
                    return iced::Task::none();
                }
                "w" => {
                    if !state.close_tab(state.active) {
                        return iced::exit();
                    }
                    return iced::Task::none();
                }
                // ⌘, opens/closes the settings panel.
                "," => {
                    state.toggle_settings();
                    return iced::Task::none();
                }
                // Zoom: ⌘+ / ⌘= grow, ⌘− shrink, ⌘0 reset.
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
                // ⌘V pastes the system clipboard into the active shell (read is async).
                "v" => return iced::clipboard::read().map(Message::Pasted),
                // ⌘F toggles the scrollback find bar; opening focuses its field.
                "f" => {
                    return if state.toggle_search() {
                        iced::widget::operation::focus(crate::view::search_id())
                    } else {
                        iced::Task::none()
                    };
                }
                d if d.len() == 1 && d.starts_with(|c: char| c.is_ascii_digit()) => {
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
    // Otherwise the keystroke is terminal input (arrow keys honor the app's DECCKM mode).
    if let Some(bytes) = phosphor::input::to_bytes(&key, mods, state.active_app_cursor()) {
        state.write_active(&bytes);
    }
    iced::Task::none()
}
