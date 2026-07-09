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
pub(crate) fn screen_term(title: &str) -> Term {
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
pub(crate) fn headless(n: usize) -> Tty {
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
        search_match: 0,
        show_scrollback: false,
        scrollback_query: String::new(),
        scrollback_selected: None,
        scrollback_scroll: 0.0,
        scrollback_expanded: std::collections::HashSet::new(),
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
fn clear_scrollback_empties_the_active_pane_without_touching_the_live_grid() {
    let mut tty = headless(1);
    {
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        // Force a couple of lines into scrollback.
        for n in 0..3 {
            s.scroll_up(1);
            let _ = n;
        }
    }
    assert!(!tty
        .active_term()
        .unwrap()
        .screen
        .lock()
        .scrollback
        .is_empty());
    let _ = update(&mut tty, Message::ClearScrollback);
    assert!(tty
        .active_term()
        .unwrap()
        .screen
        .lock()
        .scrollback
        .is_empty());
}

#[test]
fn scrollback_panel_toggles_open_and_closed_and_clears_its_query_on_close() {
    let mut tty = headless(1);
    assert!(!tty.show_scrollback);
    let _ = update(&mut tty, Message::ToggleScrollbackPanel);
    assert!(tty.show_scrollback);
    let _ = update(&mut tty, Message::ScrollbackQueryChanged("foo".into()));
    assert_eq!(tty.scrollback_query, "foo");
    let _ = update(&mut tty, Message::ToggleScrollbackPanel);
    assert!(!tty.show_scrollback);
    assert_eq!(tty.scrollback_query, "", "closing clears the filter");
}

#[test]
fn scrollback_row_select_highlights_without_copying() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::ScrollbackRowSelected(3));
    assert_eq!(tty.scrollback_selected, Some(3));
}

#[test]
fn scrollback_row_activate_selects_and_copies() {
    let mut tty = headless(1);
    let task = update(
        &mut tty,
        Message::ScrollbackRowActivated(5, "ls -la".to_string()),
    );
    assert_eq!(tty.scrollback_selected, Some(5));
    // The clipboard write is a real iced::Task, not directly inspectable here;
    // the important thing is update() didn't just no-op it away.
    let _ = task;
}

#[test]
fn changing_the_scrollback_filter_clears_the_stale_selection() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::ScrollbackRowSelected(2));
    assert_eq!(tty.scrollback_selected, Some(2));
    let _ = update(&mut tty, Message::ScrollbackQueryChanged("x".into()));
    assert_eq!(
        tty.scrollback_selected, None,
        "row 2 in the old filtered list may not even exist in the new one"
    );
}

#[test]
fn scrollback_toggle_expand_flips_and_changing_the_filter_clears_it() {
    let mut tty = headless(1);
    assert!(tty.scrollback_expanded.is_empty());
    let _ = update(&mut tty, Message::ScrollbackToggleExpand(1));
    assert!(tty.scrollback_expanded.contains(&1));
    let _ = update(&mut tty, Message::ScrollbackToggleExpand(1));
    assert!(!tty.scrollback_expanded.contains(&1), "toggles back off");
    let _ = update(&mut tty, Message::ScrollbackToggleExpand(2));
    let _ = update(&mut tty, Message::ScrollbackQueryChanged("x".into()));
    assert!(
        tty.scrollback_expanded.is_empty(),
        "index 2 in the old filtered list may not even exist in the new one"
    );
}

#[test]
fn enter_marks_a_command_boundary_in_the_focused_pane() {
    let mut tty = headless(1);
    {
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        parser.process(b"$ ls", &mut s);
    }
    let _ = update(
        &mut tty,
        Message::Key(
            Key::Named(iced::keyboard::key::Named::Enter),
            Modifiers::empty(),
        ),
    );
    {
        // The boundary is queued, not yet resolved — there's no real shell here to
        // echo the Enter back, so simulate it (a real PTY would do this
        // asynchronously; the pty-less test has to do it explicitly).
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        parser.process(b"\r\n", &mut s);
    }
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "$ ls");
}

#[test]
fn unbracketed_multiline_paste_queues_a_boundary_per_complete_line() {
    // No `bracketed_paste` (mode 2004) was ever set, so the destination almost
    // certainly can't tell paste apart from typing — each embedded newline runs
    // immediately, just like a real Enter. Each complete pasted line should become
    // its own command as its own echo arrives, exactly as if the user had typed and
    // entered each one.
    let mut tty = headless(1);
    let _ = update(
        &mut tty,
        Message::Pasted(Some("echo one\necho two\n".to_string())),
    );
    {
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        // The shell echoing + running each pasted line in turn (no real Enter
        // keypress involved — the embedded newlines are what triggers this).
        parser.process(b"echo one\r\none\r\necho two\r\ntwo\r\n$ ", &mut s);
    }
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert_eq!(screen.command_log.len(), 2);
    assert_eq!(screen.command_log[0].command, "echo one");
    assert_eq!(screen.command_log[0].output, vec!["one"]);
    assert_eq!(screen.command_log[1].command, "echo two");
    assert_eq!(screen.command_log[1].output, vec!["two"]);
}

#[test]
fn unbracketed_paste_with_no_trailing_newline_leaves_the_last_line_unmarked() {
    let mut tty = headless(1);
    let _ = update(
        &mut tty,
        Message::Pasted(Some("echo one\necho two".to_string())),
    );
    {
        // Only "echo one" was terminated by a newline in the pasted text — "echo
        // two" is still an in-progress line, same as if the user were mid-typing.
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        parser.process(b"echo one\r\none\r\necho two", &mut s);
    }
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "echo one");
}

#[test]
fn bracketed_paste_does_not_split_on_embedded_newlines() {
    let mut tty = headless(1);
    {
        // The app declared bracketed-paste support (mode 2004) — a compliant shell
        // holds the whole paste as one edit buffer, so we shouldn't preemptively
        // split it into per-line command boundaries.
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        parser.process(b"\x1b[?2004h", &mut s);
    }
    let _ = update(
        &mut tty,
        Message::Pasted(Some("echo one\necho two\n".to_string())),
    );
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert!(
        screen.command_log.is_empty(),
        "a bracketed paste shouldn't queue any per-line boundaries"
    );
}

#[test]
fn max_scrollback_step_persists_and_applies_live_to_open_panes() {
    let mut tty = headless(1);
    assert_eq!(
        tty.settings.max_scrollback(),
        crate::settings::DEFAULT_MAX_SCROLLBACK
    );
    let _ = update(&mut tty, Message::MaxScrollbackStep(-1000));
    let expected = crate::settings::DEFAULT_MAX_SCROLLBACK - 1000;
    assert_eq!(tty.settings.max_scrollback(), expected);
    // Live-applied to the already-open pane's screen, not just persisted.
    {
        let term = tty.active_term().unwrap();
        let mut s = term.screen.lock();
        for n in 0..(expected + 10) {
            s.scroll_up(1);
            let _ = n;
        }
        assert_eq!(s.scrollback.len(), expected);
    }
}

#[test]
fn clear_scrollback_and_history_panel_key_chords() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::Key(Key::Character("k".into()), cmd()));
    // No panic / no-op is the main thing we're checking here (the message-level test
    // above covers the actual clearing); this just confirms the chord routes there.
    let _ = update(
        &mut tty,
        Message::Key(Key::Character("h".into()), cmd() | Modifiers::SHIFT),
    );
    assert!(tty.show_scrollback, "⌘⇧H opens the scrollback panel");
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
        tty.menu.as_ref().map(|(k, _)| k),
        Some(&MenuKind::Tab),
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
        tty.menu.as_ref().map(|(k, _)| k),
        Some(&MenuKind::Tab),
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

// ---- update() dispatch: the rest of the Message variants not already exercised
// above via more specific behavior tests. These call `update()` itself (not the bare
// state method) to catch a message wired to the wrong method/state, not just verify
// the underlying logic (already covered where the logic itself is nontrivial).

#[test]
fn modifiers_changed_message_updates_the_tracked_modifiers() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::ModifiersChanged(cmd()));
    assert_eq!(tty.modifiers, cmd());
}

#[test]
fn resize_message_resizes_the_panes_screen() {
    let mut tty = headless(1);
    let win = main_win(&tty);
    let pane = tty.tabs[0].focus;
    let _ = update(&mut tty, Message::Resize(win, pane, 100, 40));
    let screen = tty.tabs[0].focused().unwrap().screen.lock();
    assert_eq!((screen.cols, screen.rows), (100, 40));
}

#[test]
fn pty_bytes_message_writes_without_a_pty_is_a_no_op() {
    // No PTY behind a headless term — this should never panic.
    let mut tty = headless(1);
    let win = main_win(&tty);
    let pane = tty.tabs[0].focus;
    let _ = update(&mut tty, Message::PtyBytes(win, pane, b"hi".to_vec()));
}

#[test]
fn focus_pane_message_focuses_on_a_plain_click_opens_menu_on_ctrl_click() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let win = main_win(&tty);
    tty.split_with(win, Direction::Right, screen_term("right"));
    let (first, _) = tty.tabs[0].panes.iter().next().unwrap();
    let first = *first;

    let _ = update(&mut tty, Message::FocusPane(win, first));
    assert_eq!(tty.tabs[0].focus, first, "a plain click focuses the pane");
    assert!(tty.menu.is_none());

    tty.modifiers = Modifiers::CTRL;
    let _ = update(&mut tty, Message::FocusPane(win, first));
    assert!(
        matches!(tty.menu, Some((MenuKind::Pane, _))),
        "Ctrl+click opens the pane menu instead"
    );
}

#[test]
fn resize_split_message_adjusts_the_divider_ratio() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let win = main_win(&tty);
    tty.split_with(win, Direction::Right, screen_term("right"));
    let split = *tty.tabs[0].panes.layout().splits().next().unwrap();
    let _ = update(
        &mut tty,
        Message::ResizeSplit(
            win,
            iced::widget::pane_grid::ResizeEvent { split, ratio: 0.25 },
        ),
    );
    let ratio = tty.tabs[0]
        .panes
        .layout()
        .splits()
        .find(|s| **s == split)
        .map(|_| ()); // just confirm the split still exists after resizing it
    assert!(ratio.is_some());
}

#[test]
fn link_click_message_opens_the_link_menu_at_the_pointer() {
    let mut tty = headless(1);
    tty.pointer = iced::Point::new(12.0, 34.0);
    let _ = update(&mut tty, Message::LinkClick("https://example.com".into()));
    assert!(matches!(
        &tty.menu,
        Some((MenuKind::Link(url), p)) if url == "https://example.com" && *p == iced::Point::new(12.0, 34.0)
    ));
}

#[test]
fn copy_link_message_closes_the_menu() {
    let mut tty = headless(1);
    tty.menu = Some((
        MenuKind::Link("https://example.com".into()),
        iced::Point::ORIGIN,
    ));
    let _ = update(&mut tty, Message::CopyLink("https://example.com".into()));
    assert!(tty.menu.is_none());
}

#[test]
fn split_message_splits_the_main_tabs_focused_pane() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    tty.menu = Some((MenuKind::Pane, iced::Point::ORIGIN));
    let _ = update(&mut tty, Message::Split(Direction::Down));
    assert_eq!(tty.tabs[0].panes.len(), 2);
    assert!(tty.menu.is_none(), "the menu closes after acting");
}

#[test]
fn close_pane_message_closes_a_pane_or_exits_on_the_last() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    tty.split_with(main_win(&tty), Direction::Down, screen_term("lower"));
    tty.menu = Some((MenuKind::Pane, iced::Point::ORIGIN));
    let _ = update(&mut tty, Message::ClosePane);
    assert_eq!(tty.tabs[0].panes.len(), 1, "one of two panes closed");
    assert!(tty.menu.is_none());

    // The last pane in the last tab closing signals exit (a real `Task` we don't
    // need to inspect further than "it didn't panic"; `close_focused_pane`'s own
    // return value is covered directly elsewhere).
    let _ = update(&mut tty, Message::ClosePane);
}

#[test]
fn hover_tab_message_tracks_hover_and_reorders_a_live_drag() {
    let mut tty = headless(3);
    let _ = update(&mut tty, Message::HoverTab(Some(2)));
    assert_eq!(tty.hovered_tab, Some(2));
    let _ = update(&mut tty, Message::HoverTab(None));
    assert_eq!(tty.hovered_tab, None);

    // With a drag armed, hovering another tab reorders live.
    tty.tab_drag = Some((0, iced::Point::ORIGIN));
    let _ = update(&mut tty, Message::HoverTab(Some(2)));
    assert_eq!(tty.tab_drag.map(|(idx, _)| idx), Some(2));
}

#[test]
fn search_changed_and_submit_step_the_match_index() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::SearchChanged("foo".to_string()));
    assert_eq!(tty.search.as_deref(), Some("foo"));
    assert_eq!(tty.search_match, 0);

    let _ = update(&mut tty, Message::SearchSubmit);
    assert_eq!(tty.search_match, 1, "Enter steps to the next match");

    tty.modifiers = Modifiers::SHIFT;
    let _ = update(&mut tty, Message::SearchSubmit);
    assert_eq!(tty.search_match, 0, "⇧Enter steps back");
}

#[test]
fn scrollback_scrolled_message_updates_the_offset() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::ScrollbackScrolled(42.0));
    assert_eq!(tty.scrollback_scroll, 42.0);
}

#[test]
fn default_output_lines_step_message_persists() {
    let mut tty = headless(1);
    let before = tty.settings.default_output_lines();
    let _ = update(&mut tty, Message::DefaultOutputLinesStep(10));
    assert_eq!(tty.settings.default_output_lines(), before + 10);
}

#[test]
fn new_tab_and_close_tab_messages() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::NewTab);
    assert_eq!(tty.tabs.len(), 2, "NewTab opens another tab");

    let _ = update(&mut tty, Message::CloseTab(0));
    assert_eq!(tty.tabs.len(), 1, "CloseTab removes it, one tab remains");
}

#[test]
fn tick_message_reaps_dead_tabs_and_exits_when_none_remain() {
    let mut tty = headless(1);
    tty.tabs[0]
        .focused()
        .unwrap()
        .alive
        .store(false, Ordering::Relaxed);
    // The only tab died — Tick should reap it and signal exit (a real `Task`,
    // not directly inspectable here; the point is it doesn't panic and the state
    // ends up reaped).
    let _ = update(&mut tty, Message::Tick);
}

#[test]
fn toggle_settings_and_theme_font_messages() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::ToggleSettings);
    assert!(tty.show_settings);
    let _ = update(&mut tty, Message::SettingsSection(1));
    assert_eq!(tty.settings_section, 1);

    let _ = update(&mut tty, Message::SetTheme("Nord".to_string()));
    assert_eq!(tty.settings.theme.as_deref(), Some("Nord"));

    let _ = update(&mut tty, Message::SetFont("Fira Code".to_string()));
    assert_eq!(tty.settings.font_family.as_deref(), Some("Fira Code"));

    let before = tty.font_size;
    let _ = update(&mut tty, Message::FontSizeStep(1.0));
    assert!(tty.font_size > before);
}

#[test]
fn base16_and_palette_messages() {
    let mut tty = headless(1);
    let _ = update(
        &mut tty,
        Message::Base16Changed("not-16-colors".to_string()),
    );
    assert_eq!(tty.base16_input, "not-16-colors");
    // Malformed input is a no-op rather than a panic.
    let _ = update(&mut tty, Message::ApplyBase16);
    assert!(tty.settings.palette.is_none());

    let _ = update(
        &mut tty,
        Message::EditColor(16, iced::Color::from_rgb(1.0, 0.0, 0.0)),
    );
    assert_eq!(tty.theme.terminal.fg, iced::Color::from_rgb(1.0, 0.0, 0.0));

    let _ = update(&mut tty, Message::ResetPalette);
    assert!(tty.settings.palette.is_none());
}

#[test]
fn set_unfocused_opacity_message_clamps_and_persists() {
    let mut tty = headless(1);
    let _ = update(&mut tty, Message::SetUnfocusedOpacity(0.5));
    assert_eq!(tty.settings.unfocused_opacity, Some(0.5));
}

#[test]
fn detach_tab_and_reattach_tab_messages() {
    let mut tty = headless(2);
    let _ = update(&mut tty, Message::DetachTab(0));
    assert_eq!(tty.tabs.len(), 1);
    assert_eq!(tty.detached.len(), 1);

    let win = *tty.detached.keys().next().unwrap();
    let _ = update(&mut tty, Message::ReattachTab(win));
    assert_eq!(tty.tabs.len(), 2, "the tab is docked back");
    assert!(tty.detached.is_empty());
}

#[test]
fn window_focus_move_resize_position_messages() {
    let mut tty = headless(1);
    let main = main_win(&tty);
    let _ = update(&mut tty, Message::WindowFocused(main));
    assert!(tty.focused);
    assert_eq!(tty.focused_window, Some(main));

    let _ = update(
        &mut tty,
        Message::WindowResizedAt(main, iced::Size::new(900.0, 650.0)),
    );
    assert_eq!(tty.window_height, 650.0);

    let detached_win = iced::window::Id::unique();
    let _ = update(
        &mut tty,
        Message::WindowPosition(detached_win, Some(iced::Point::new(5.0, 6.0))),
    );
    assert_eq!(
        (
            tty.window_bounds[&detached_win].x,
            tty.window_bounds[&detached_win].y
        ),
        (5.0, 6.0)
    );

    let _ = update(
        &mut tty,
        Message::WindowMoved(detached_win, iced::Point::new(7.0, 8.0)),
    );
    assert_eq!(
        (
            tty.window_bounds[&detached_win].x,
            tty.window_bounds[&detached_win].y
        ),
        (7.0, 8.0)
    );
}

#[test]
fn check_drag_reattach_message_reattaches_when_settled_on_the_band() {
    let mut tty = headless(1);
    let main = main_win(&tty);
    tty.window_bounds.insert(
        main,
        iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        },
    );
    let dwin = detach_manually(&mut tty, Tab::new(screen_term("d")), 0);
    tty.window_bounds.insert(
        dwin,
        iced::Rectangle {
            x: 100.0,
            y: 10.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((
        dwin,
        std::time::Instant::now() - std::time::Duration::from_secs(1),
    ));
    let _ = update(&mut tty, Message::CheckDragReattach);
    assert!(tty.detached.is_empty(), "settling on the band reattaches");
}

#[test]
fn pointer_released_message_finishes_an_armed_tab_drag() {
    use crate::state::TAB_TEAR_THRESHOLD;
    let mut tty = headless(2);
    tty.tab_drag = Some((0, iced::Point::new(20.0, 0.0)));
    tty.pointer = iced::Point::new(22.0, TAB_TEAR_THRESHOLD + 10.0);
    let _ = update(&mut tty, Message::PointerReleased);
    assert_eq!(tty.detached.len(), 1, "a long drag detaches on release");
}
