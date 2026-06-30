//! Headless behavior tests (dev/test only) — the tty counterpart of fed-ide's.
//!
//! These drive the **real** `update`/state logic without spawning a shell: a tab can
//! hold a screen with no PTY behind it (`pty: None`), so we exercise tab open/close,
//! zoom clamping, selection caching, and dead-tab reaping with no GPU and no child
//! process. They share no config dir, but live in the `serial-ui` nextest group with
//! the snapshot tests for consistency. See the repo `.config/nextest.toml`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use iced::keyboard::{Key, Modifiers};
use iced::Font;

use cathode::screen::TerminalScreen;

use crate::message::Message;
use crate::state::{Term, Tty, DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::theme::Theme;
use crate::update::update;

/// A screen-only tab (no shell) for tests.
fn screen_term(title: &str) -> Term {
    Term {
        screen: Arc::new(Mutex::new(TerminalScreen::new(80, 24))),
        pty: None,
        title: title.into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
    }
}

/// A `Tty` with `n` pty-less tabs — bypasses `Tty::new` (which spawns a shell).
fn headless(n: usize) -> Tty {
    let tabs = (0..n).map(|i| screen_term(&format!("sh{i}"))).collect();
    Tty {
        tabs,
        active: 0,
        theme: Theme::default(),
        font: Font::MONOSPACE,
        font_size: DEFAULT_FONT_SIZE,
        modifiers: Modifiers::default(),
        window_height: 600.0,
        hovered_tab: None,
        selection: None,
        search: None,
        settings: Default::default(),
        show_settings: false,
        settings_section: 0,
        base16_input: String::new(),
        focused: true,
    }
}

/// The app-chord modifier `update` checks via `Modifiers::command()` — ⌘ (LOGO) on
/// macOS, Ctrl elsewhere — so these tests pass on every platform's CI.
fn cmd() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::LOGO
    } else {
        Modifiers::CTRL
    }
}

#[test]
fn closing_a_tab_keeps_at_least_one_then_signals_exit() {
    let mut tty = headless(2);
    // Closing one of two leaves one and reports "keep running".
    assert!(tty.close_tab(0));
    assert_eq!(tty.tabs.len(), 1);
    // Closing the last reports "no tabs left" (the app exits).
    assert!(!tty.close_tab(0));
}

#[test]
fn closing_clamps_the_active_index() {
    let mut tty = headless(3);
    tty.active = 2;
    tty.close_tab(2);
    assert_eq!(tty.active, 1, "active follows the removed last tab");
}

#[test]
fn zoom_is_clamped_both_ways() {
    let mut tty = headless(1);
    tty.zoom(1000.0);
    assert_eq!(tty.font_size, MAX_FONT_SIZE);
    tty.zoom(-10_000.0);
    assert_eq!(tty.font_size, MIN_FONT_SIZE);
    tty.reset_zoom();
    assert_eq!(tty.font_size, DEFAULT_FONT_SIZE);
}

#[test]
fn cmd_zoom_keys_resize_the_font() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::Key(Key::Character("+".into()), cmd()));
    assert_eq!(tty.font_size, DEFAULT_FONT_SIZE + 1.0);
    let _ = update(&mut tty, Message::Key(Key::Character("-".into()), cmd()));
    assert_eq!(tty.font_size, DEFAULT_FONT_SIZE);
    let _ = update(&mut tty, Message::Key(Key::Character("0".into()), cmd()));
    assert_eq!(tty.font_size, DEFAULT_FONT_SIZE);
}

#[test]
fn cmd_digit_activates_that_tab() {
    let mut tty = headless(3);
    let _ = update(&mut tty, Message::Key(Key::Character("3".into()), cmd()));
    assert_eq!(tty.active, 2);
    // Out-of-range digit is ignored.
    let _ = update(&mut tty, Message::Key(Key::Character("9".into()), cmd()));
    assert_eq!(tty.active, 2);
}

#[test]
fn select_then_caches_for_copy() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::Select(Some("hello".into())));
    assert_eq!(tty.selection.as_deref(), Some("hello"));
    let _ = update(&mut tty, Message::Select(None));
    assert_eq!(tty.selection, None);
}

#[test]
fn reap_drops_exited_tabs_and_quits_on_the_last() {
    let mut tty = headless(2);
    tty.tabs[0].alive.store(false, Ordering::Relaxed);
    assert!(tty.reap_dead(), "one live tab remains");
    assert_eq!(tty.tabs.len(), 1);
    tty.tabs[0].alive.store(false, Ordering::Relaxed);
    assert!(!tty.reap_dead(), "no tabs left → exit");
}

#[test]
fn app_cursor_mode_follows_the_screen() {
    let tty = headless(1);
    assert!(!tty.active_app_cursor());
    // The shell enables DECCKM (application cursor keys).
    let mut parser = cathode::parser::TermParser::new();
    parser.process(b"\x1b[?1h", &mut tty.tabs[0].screen.lock());
    assert!(tty.active_app_cursor());
}

#[test]
fn search_toggles_open_and_closed() {
    let mut tty = headless(1);
    assert!(
        tty.toggle_search(),
        "opening returns true (focus the field)"
    );
    assert_eq!(tty.search.as_deref(), Some(""));
    assert!(!tty.toggle_search(), "closing returns false");
    assert_eq!(tty.search, None);
}

#[test]
fn settings_panel_toggles_and_switches_section() {
    let mut tty = headless(1);
    assert!(!tty.show_settings);
    // ⌘, opens it.
    let _ = update(&mut tty, Message::Key(Key::Character(",".into()), cmd()));
    assert!(tty.show_settings);
    // Switch to the Palette section.
    let _ = update(&mut tty, Message::SettingsSection(1));
    assert_eq!(tty.settings_section, 1);
    // Escape closes it (without going to the shell).
    let esc = Key::Named(iced::keyboard::key::Named::Escape);
    let _ = update(&mut tty, Message::Key(esc, Modifiers::default()));
    assert!(!tty.show_settings);
}

#[test]
fn unfocused_opacity_applies_only_when_unfocused() {
    let mut tty = headless(1);
    // Off by default — opaque regardless of focus.
    assert_eq!(tty.window_opacity(), 1.0);
    let _ = update(&mut tty, Message::Focused(false));
    assert!(!tty.focused);
    assert_eq!(
        tty.window_opacity(),
        1.0,
        "off by default even when unfocused"
    );
    // Enabled: translucent only while unfocused, opaque while focused.
    tty.settings.unfocused_opacity = Some(0.6);
    assert_eq!(tty.window_opacity(), 0.6, "translucent while unfocused");
    let _ = update(&mut tty, Message::Focused(true));
    assert_eq!(tty.window_opacity(), 1.0, "always opaque while focused");
    // Never fully invisible — clamped to the minimum opacity (95% max transparency).
    tty.settings.unfocused_opacity = Some(0.0);
    tty.focused = false;
    assert_eq!(
        tty.window_opacity(),
        crate::settings::MIN_OPACITY,
        "clamped away from invisible"
    );
}

#[test]
fn drain_lights_background_activity_and_clears_active() {
    let mut tty = headless(2);
    // Output on the inactive tab 1; active tab 0 has none.
    tty.tabs[1].dirty.store(true, Ordering::Relaxed);
    let _ = tty.drain_effects();
    assert!(tty.tabs[1].activity, "background output lights a dot");
    assert!(!tty.tabs[0].activity, "the active tab never carries a dot");
    // Switching to it clears the dot.
    tty.activate(1);
    assert!(!tty.tabs[1].activity);
}
