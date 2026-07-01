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
        main_window: Some(iced::window::Id::unique()),
        focused_window: None,
        detached: std::collections::HashMap::new(),
        detach_origin: std::collections::HashMap::new(),
        tab_drag: None,
        window_bounds: std::collections::HashMap::new(),
        last_detached_move: None,
    }
}

/// The main window id of a headless `Tty` (always set by `headless`).
fn main_win(tty: &Tty) -> iced::window::Id {
    tty.main_window.expect("headless sets a main window")
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
    let win = main_win(&tty);
    let pane = tty.tabs[0].focus;
    let _ = update(&mut tty, Message::Select(win, pane, Some("hello".into())));
    assert_eq!(tty.selection.as_deref(), Some("hello"));
    let _ = update(&mut tty, Message::Select(win, pane, None));
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
    assert!(tty.reap_dead().0, "one live tab remains");
    assert_eq!(tty.tabs.len(), 1);
    tty.tabs[0]
        .focused()
        .unwrap()
        .alive
        .store(false, Ordering::Relaxed);
    assert!(!tty.reap_dead().0, "no tabs left → exit");
}

#[test]
fn app_cursor_mode_follows_the_screen() {
    let tty = headless(1);
    let win = main_win(&tty);
    assert!(!tty.app_cursor_for(win));
    // The shell enables DECCKM (application cursor keys).
    let mut parser = cathode::parser::TermParser::new();
    parser.process(
        b"\x1b[?1h",
        &mut tty.tabs[0].focused().unwrap().screen.lock(),
    );
    assert!(tty.app_cursor_for(win));
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
    let win = main_win(&tty);
    assert_eq!(tty.tabs[0].panes.len(), 1);
    let first = tty.tabs[0].focus;
    tty.split_with(win, Direction::Right, screen_term("split"));
    assert_eq!(tty.tabs[0].panes.len(), 2, "a split adds a pane");
    assert_eq!(tty.tabs.len(), 1, "splitting stays within one tab");
    assert_ne!(tty.tabs[0].focus, first, "focus moves to the new pane");
}

#[test]
fn focus_dir_moves_between_neighbours() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let win = main_win(&tty);
    let left = tty.tabs[0].focus;
    // Split right; focus is now the right pane. ← returns to the left, → comes back.
    tty.split_with(win, Direction::Right, screen_term("right"));
    let right = tty.tabs[0].focus;
    tty.focus_dir(win, Direction::Left);
    assert_eq!(tty.tabs[0].focus, left, "← moves to the left neighbour");
    tty.focus_dir(win, Direction::Right);
    assert_eq!(tty.tabs[0].focus, right, "→ moves back to the right");
    // No neighbour past the edge — focus stays put.
    tty.focus_dir(win, Direction::Right);
    assert_eq!(tty.tabs[0].focus, right, "no-op at the edge");
}

#[test]
fn closing_a_pane_keeps_the_tab_until_the_last_pane() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(2);
    let win = main_win(&tty);
    // Split the active tab into two panes, then close one: the tab survives.
    tty.split_with(win, Direction::Down, screen_term("lower"));
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
    let win = main_win(&tty);
    // Two panes; focus the left one, then right-click the right one.
    tty.split_with(win, Direction::Right, screen_term("right"));
    let right = tty.tabs[0].focus;
    tty.focus_dir(win, Direction::Left);
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

/// Move `tab` into a detached window without opening a real OS window (tests can't),
/// returning its synthetic window id.
fn detach_manually(tty: &mut Tty, tab: Tab, origin: usize) -> iced::window::Id {
    let id = iced::window::Id::unique();
    tty.detached.insert(id, tab);
    tty.detach_origin.insert(id, origin);
    id
}

#[test]
fn detach_tab_moves_a_tab_into_its_own_window() {
    let mut tty = headless(2);
    // Detaching the first of two tabs leaves one in the strip and one detached.
    let _task = tty.detach_tab(0);
    assert_eq!(tty.tabs.len(), 1, "the detached tab left the main strip");
    assert_eq!(tty.detached.len(), 1, "and now lives in its own window");
    let (_win, origin) = tty.detach_origin.iter().next().unwrap();
    assert_eq!(*origin, 0, "its origin index is remembered for reattach");
}

#[test]
fn reattach_window_docks_the_tab_back_at_its_origin() {
    let mut tty = headless(3);
    // Detach the middle tab, then dock it back — it returns to index 1.
    tty.detach_tab(1);
    assert_eq!(tty.tabs.len(), 2);
    let win = *tty.detached.keys().next().unwrap();
    tty.reattach_window(win);
    assert_eq!(tty.tabs.len(), 3, "the tab is back in the strip");
    assert!(tty.detached.is_empty(), "and gone from the detached set");
    assert_eq!(tty.active, 1, "reattach activates it at its origin index");
}

#[test]
fn os_close_of_a_detached_window_reattaches_but_cmd_w_last_pane_does_not() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(2);
    let win = tty.main_window.unwrap();
    // Build a two-pane tab in the main strip, then move it into a detached window.
    tty.split_with(win, Direction::Right, screen_term("b"));
    let tab = tty.tabs.remove(0);
    tty.active = 0;
    let dwin = detach_manually(&mut tty, tab, 0);

    // ⌘W closes one pane in place (no window close).
    assert!(
        tty.close_detached_focused_pane(dwin).is_none(),
        "first ⌘W closes a pane, window stays"
    );
    // ⌘W on the now-only pane closes the WINDOW and drops the tab (no reattach).
    assert_eq!(
        tty.close_detached_focused_pane(dwin),
        Some(dwin),
        "last pane closes the window"
    );
    assert!(
        !tty.detached.contains_key(&dwin),
        "the tab was removed before the window closes, so a WindowClosed can't reattach it"
    );
    let before = tty.tabs.len();
    let _ = update(&mut tty, Message::WindowClosed(dwin));
    assert_eq!(tty.tabs.len(), before, "⌘W-closed window does not reattach");

    // By contrast, an OS-close of a still-detached window reattaches its tab.
    let dwin2 = detach_manually(&mut tty, Tab::new(screen_term("c")), 0);
    let before = tty.tabs.len();
    let _ = update(&mut tty, Message::WindowClosed(dwin2));
    assert_eq!(tty.tabs.len(), before + 1, "OS-close reattaches the tab");
    assert!(tty.detached.is_empty());
}

#[test]
fn reap_closes_a_detached_window_whose_shell_died() {
    let mut tty = headless(1);
    let dead = Tab::new(screen_term("dead"));
    dead.focused()
        .unwrap()
        .alive
        .store(false, Ordering::Relaxed);
    let dwin = detach_manually(&mut tty, dead, 0);
    let (any, closed) = tty.reap_dead();
    assert!(any, "a live main tab remains");
    assert_eq!(
        closed,
        vec![dwin],
        "the dead detached window is reported for close"
    );
    assert!(tty.detached.is_empty(), "and reaped from the detached set");
}

#[test]
fn tab_tear_off_detaches_only_past_the_threshold() {
    use crate::state::TAB_TEAR_THRESHOLD;
    // A short drag is a click — no detach.
    let mut tty = headless(2);
    tty.tab_drag = Some((0, iced::Point::new(20.0, 0.0)));
    tty.pointer = iced::Point::new(22.0, 10.0); // dy = 10 < threshold
    assert!(
        tty.finish_tab_drag().is_none(),
        "a short drag is just a click"
    );
    assert_eq!(tty.detached.len(), 0);

    // A drag past the threshold tears the tab off.
    tty.tab_drag = Some((0, iced::Point::new(20.0, 0.0)));
    tty.pointer = iced::Point::new(22.0, TAB_TEAR_THRESHOLD + 10.0);
    let _task = tty.finish_tab_drag().expect("a long drag detaches");
    assert_eq!(
        tty.detached.len(),
        1,
        "the tab tore off into its own window"
    );
}

/// Dragging a tab across the strip live-reorders it (browser-style): the pressed tab
/// follows the pointer to the slot it's dragged over and stays active.
#[test]
fn dragging_a_tab_reorders_it() {
    let mut tty = headless(3); // sh0, sh1, sh2
                               // Press the first tab (arming the drag), then drag it over the last slot.
    tty.tab_drag = Some((0, iced::Point::ORIGIN));
    tty.reorder_dragged_tab(2);

    let labels: Vec<String> = tty.tabs.iter().map(|t| t.label()).collect();
    assert_eq!(
        labels,
        vec!["sh1", "sh2", "sh0"],
        "the dragged tab moved to the end"
    );
    assert_eq!(tty.active, 2, "and stays active under the cursor");
    assert_eq!(
        tty.tab_drag,
        Some((2, iced::Point::ORIGIN)),
        "the drag re-anchored to the new slot for further crossings"
    );
}

#[test]
fn drag_dock_reattaches_only_when_dropped_on_the_main_band() {
    use std::time::{Duration, Instant};
    let mut tty = headless(1);
    let main = tty.main_window.unwrap();
    let dwin = detach_manually(&mut tty, Tab::new(screen_term("d")), 0);
    // Place the main window at the origin, 900×600.
    tty.window_bounds.insert(
        main,
        iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        },
    );
    let settled = Instant::now() - Duration::from_secs(1); // already past SETTLE

    // Dropped with its top inside the main window's top band → reattach.
    tty.window_bounds.insert(
        dwin,
        iced::Rectangle {
            x: 100.0,
            y: 10.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((dwin, settled));
    assert!(
        matches!(
            crate::detach_drag::poll_settle(&mut tty),
            crate::detach_drag::Settle::Reattach(w) if w == dwin
        ),
        "dropping on the tab-bar band docks back"
    );

    // Dropped well below the band → just repositioned.
    tty.window_bounds.insert(
        dwin,
        iced::Rectangle {
            x: 100.0,
            y: 400.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((dwin, settled));
    assert!(
        matches!(
            crate::detach_drag::poll_settle(&mut tty),
            crate::detach_drag::Settle::Repositioned
        ),
        "dropping elsewhere just moves the window"
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
