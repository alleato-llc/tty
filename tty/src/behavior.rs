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
use crate::state::{MenuKind, Tab, Term, Tty, DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE};
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

/// A `Tty` with `n` pty-less single-pane tabs — bypasses `Tty::new` (which spawns a shell).
fn headless(n: usize) -> Tty {
    let tabs = (0..n)
        .map(|i| Tab::new(screen_term(&format!("sh{i}"))))
        .collect();
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
        pointer: iced::Point::ORIGIN,
        menu: None,
        renaming: None,
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
    let pane = tty.tabs[0].focus;
    let _ = update(&mut tty, Message::Select(pane, Some("hello".into())));
    assert_eq!(tty.selection.as_deref(), Some("hello"));
    let _ = update(&mut tty, Message::Select(pane, None));
    assert_eq!(tty.selection, None);
}

#[test]
fn reap_drops_exited_tabs_and_quits_on_the_last() {
    let mut tty = headless(2);
    tty.tabs[0]
        .focused()
        .unwrap()
        .alive
        .store(false, Ordering::Relaxed);
    assert!(tty.reap_dead(), "one live tab remains");
    assert_eq!(tty.tabs.len(), 1);
    tty.tabs[0]
        .focused()
        .unwrap()
        .alive
        .store(false, Ordering::Relaxed);
    assert!(!tty.reap_dead(), "no tabs left → exit");
}

#[test]
fn app_cursor_mode_follows_the_screen() {
    let tty = headless(1);
    assert!(!tty.active_app_cursor());
    // The shell enables DECCKM (application cursor keys).
    let mut parser = cathode::parser::TermParser::new();
    parser.process(
        b"\x1b[?1h",
        &mut tty.tabs[0].focused().unwrap().screen.lock(),
    );
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
fn tab_highlight_toggles_and_defaults_on() {
    let mut tty = headless(1);
    assert!(tty.settings.tab_highlight(), "on by default");
    let _ = update(&mut tty, Message::SetTabHighlight(false));
    assert_eq!(tty.settings.tab_highlight, Some(false));
    assert!(!tty.settings.tab_highlight(), "honors the explicit off");
    let _ = update(&mut tty, Message::SetTabHighlight(true));
    assert!(tty.settings.tab_highlight());
}

#[test]
fn splitting_adds_a_focused_pane_to_the_tab() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    assert_eq!(tty.tabs[0].panes.len(), 1);
    let first = tty.tabs[0].focus;
    tty.split_with(Direction::Right, screen_term("split"));
    assert_eq!(tty.tabs[0].panes.len(), 2, "a split adds a pane");
    assert_eq!(tty.tabs.len(), 1, "splitting stays within one tab");
    assert_ne!(tty.tabs[0].focus, first, "focus moves to the new pane");
}

#[test]
fn focus_dir_moves_between_neighbours() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let left = tty.tabs[0].focus;
    // Split right; focus is now the right pane. ← returns to the left, → comes back.
    tty.split_with(Direction::Right, screen_term("right"));
    let right = tty.tabs[0].focus;
    tty.focus_dir(Direction::Left);
    assert_eq!(tty.tabs[0].focus, left, "← moves to the left neighbour");
    tty.focus_dir(Direction::Right);
    assert_eq!(tty.tabs[0].focus, right, "→ moves back to the right");
    // No neighbour past the edge — focus stays put.
    tty.focus_dir(Direction::Right);
    assert_eq!(tty.tabs[0].focus, right, "no-op at the edge");
}

#[test]
fn closing_a_pane_keeps_the_tab_until_the_last_pane() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(2);
    // Split the active tab into two panes, then close one: the tab survives.
    tty.split_with(Direction::Down, screen_term("lower"));
    assert_eq!(tty.tabs[0].panes.len(), 2);
    assert!(
        tty.close_focused_pane(),
        "closing one pane keeps the app running"
    );
    assert_eq!(tty.tabs[0].panes.len(), 1, "the sibling pane remains");
    assert_eq!(tty.tabs.len(), 2, "the tab itself is untouched");
    // Closing the now-single pane closes the whole tab.
    assert!(tty.close_focused_pane(), "still one tab left");
    assert_eq!(tty.tabs.len(), 1);
    // Closing the last pane of the last tab signals exit.
    assert!(!tty.close_focused_pane(), "no panes left anywhere → exit");
}

#[test]
fn right_click_focuses_the_pane_and_opens_the_menu_at_the_pointer() {
    use iced::widget::pane_grid::Direction;
    use iced::Point;
    let mut tty = headless(1);
    // Two panes; focus the left one, then right-click the right one.
    tty.split_with(Direction::Right, screen_term("right"));
    let right = tty.tabs[0].focus;
    tty.focus_dir(Direction::Left);
    assert_ne!(tty.tabs[0].focus, right);
    let _ = update(&mut tty, Message::PointerMoved(Point::new(120.0, 80.0)));
    let _ = update(&mut tty, Message::PaneRightClick(right));
    assert_eq!(tty.tabs[0].focus, right, "right-click focuses its pane");
    assert_eq!(
        tty.menu,
        Some((MenuKind::Pane, Point::new(120.0, 80.0))),
        "a pane menu anchors at the last pointer position"
    );
    // Dismissing clears it.
    let _ = update(&mut tty, Message::CloseMenu);
    assert!(tty.menu.is_none());
}

#[test]
fn right_clicking_a_tab_activates_it_and_opens_the_tab_menu() {
    let mut tty = headless(2);
    tty.active = 0;
    let _ = update(&mut tty, Message::TabRightClick(1));
    assert_eq!(tty.active, 1, "right-clicking a tab activates it");
    assert_eq!(
        tty.menu.map(|(k, _)| k),
        Some(MenuKind::Tab),
        "and opens the tab menu"
    );
}

#[test]
fn ctrl_click_opens_the_menu_instead_of_activating() {
    // macOS secondary-click arrives as Left+Control, so Ctrl+click must open the menu.
    let mut tty = headless(2);
    tty.active = 0;
    tty.modifiers = Modifiers::CTRL;
    let _ = update(&mut tty, Message::ActivateTab(1));
    assert_eq!(
        tty.menu.map(|(k, _)| k),
        Some(MenuKind::Tab),
        "ctrl+click a tab opens its menu"
    );
    // A plain click still just activates.
    tty.close_menu();
    tty.modifiers = Modifiers::default();
    let _ = update(&mut tty, Message::ActivateTab(1));
    assert_eq!(tty.active, 1);
    assert!(tty.menu.is_none(), "a plain click doesn't open a menu");
}

#[test]
fn rename_tab_sets_a_custom_label_blank_reverts_escape_cancels() {
    let mut tty = headless(2);
    // Rename commits a custom label and closes the editor.
    let _ = update(&mut tty, Message::StartRename(0));
    assert!(tty.renaming.is_some(), "the rename editor is open");
    let _ = update(&mut tty, Message::RenameChanged("build".into()));
    let _ = update(&mut tty, Message::RenameSubmit);
    assert_eq!(tty.tabs[0].title.as_deref(), Some("build"));
    assert_eq!(tty.tabs[0].label(), "build");
    assert!(tty.renaming.is_none());

    // A blank name clears the override (back to the auto label).
    let _ = update(&mut tty, Message::StartRename(0));
    let _ = update(&mut tty, Message::RenameChanged("   ".into()));
    let _ = update(&mut tty, Message::RenameSubmit);
    assert_eq!(tty.tabs[0].title, None, "blank reverts to the auto label");

    // Escape cancels, leaving the name untouched.
    tty.tabs[0].title = Some("keep".into());
    let _ = update(&mut tty, Message::StartRename(0));
    let _ = update(&mut tty, Message::RenameChanged("discard".into()));
    let esc = Key::Named(iced::keyboard::key::Named::Escape);
    let _ = update(&mut tty, Message::Key(esc, Modifiers::default()));
    assert!(tty.renaming.is_none(), "escape closes the editor");
    assert_eq!(
        tty.tabs[0].title.as_deref(),
        Some("keep"),
        "the name is unchanged on cancel"
    );
}

#[test]
fn drain_lights_background_activity_and_clears_active() {
    let mut tty = headless(2);
    // Output on the inactive tab 1; active tab 0 has none.
    tty.tabs[1]
        .focused()
        .unwrap()
        .dirty
        .store(true, Ordering::Relaxed);
    let _ = tty.drain_effects();
    assert!(
        tty.tabs[1].focused().unwrap().activity,
        "background output lights a dot"
    );
    assert!(
        !tty.tabs[0].focused().unwrap().activity,
        "the active tab never carries a dot"
    );
    // Switching to it clears the dot.
    tty.activate(1);
    assert!(!tty.tabs[1].focused().unwrap().activity);
}
