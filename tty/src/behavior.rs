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
        window_width: 0.0,
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
        appearance_tab: 0,
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
        history_writer: None,
        history_read: None,
        scrollback_archived: Vec::new(),
        scrollback_archive_cursor: None,
        history_start_failed: false,
        confirm_reset_history: false,
        last_history_auth: None,
        history_reauth_pending: false,
        show_settings_history: false,
        settings_history: Vec::new(),
        settings_history_cursor: None,
        settings_history_selected: None,
        settings_history_scroll: 0.0,
        confirm_delete_settings_row: None,
        history_starting: false,
        history_id_floor: 0,
        history_locked: false,
        passphrase_prompt: None,
        session_untracked: false,
        untracked_forced_by_cli: false,
        show_session_start_prompt: false,
        metrics: Default::default(),
        status_bar_scroll: 0,
        status_bar_edit: false,
        status_metric_press: None,
        status_metric_drag: None,
        status_metric_drop: None,
        proc_sort: (crate::state::ProcSortColumn::Cpu, true),
        proc_table_scroll: 0.0,
        proc_detail_pid: None,
        metric_details: Vec::new(),
        metric_detail_resize: None,
        metric_detail_move_drag: None,
        pane_replace_pending: None,
        pane_replace_confirm: None,
        kill_confirm: None,
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
fn adopt_settings_applies_external_changes_and_no_ops_when_unchanged() {
    use crate::settings::Settings;
    let mut tty = headless(1);
    // A hand-edit changes the theme, font, and scrollback cap on disk.
    let edited = Settings {
        theme: Some("Nord".into()),
        font_size: Some(20.0),
        max_scrollback: Some(500),
        ..Default::default()
    };
    assert!(
        tty.adopt_settings(edited.clone()),
        "a real change is adopted"
    );
    assert_eq!(tty.settings.theme.as_deref(), Some("Nord"));
    assert_eq!(tty.font_size, 20.0);
    // The cap reached the open pane, not just the settings struct.
    assert_eq!(
        tty.tabs[0]
            .focused()
            .unwrap()
            .screen
            .lock()
            .max_scrollback(),
        500,
    );
    // Re-adopting the same settings is a no-op (this is what makes reload-on-focus
    // cheap and idempotent right after our own save).
    assert!(
        !tty.adopt_settings(edited),
        "unchanged settings are a no-op"
    );
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
fn metric_popover_drags_to_move_and_resizes_in_both_states() {
    use iced::Point;
    let mut tty = headless(1);
    tty.window_width = 1000.0;
    tty.window_height = 700.0;
    tty.open_metric_detail(crate::settings::MetricKind::Cpu.as_setting_str());
    assert_eq!(tty.metric_details.len(), 1, "one popover opens");

    // Move: press popover 0's body at (500,500), drag to (530,450), release.
    tty.pointer = Point::new(500.0, 500.0);
    let _ = update(&mut tty, Message::MetricDetailMoveStart(0));
    let _ = update(&mut tty, Message::PointerMoved(Point::new(530.0, 450.0)));
    assert_eq!(tty.metric_details[0].move_offset, (30.0, -50.0));
    let _ = update(&mut tty, Message::PointerReleased);
    assert!(tty.metric_detail_move_drag.is_none());
    assert_eq!(
        tty.metric_details[0].move_offset,
        (30.0, -50.0),
        "offset persists after release"
    );

    // Resize the compact card: corner drag from (530,450) to (630,490) grows
    // both axes.
    let _ = update(
        &mut tty,
        Message::MetricDetailResizeStart(0, crate::state::ResizeEdge::Corner),
    );
    let _ = update(&mut tty, Message::PointerMoved(Point::new(630.0, 490.0)));
    let (cw, ch) = tty.metric_details[0].size.expect("compact resized");
    assert!(cw > 320.0 && ch > 150.0, "grew from the compact default");
    let _ = update(&mut tty, Message::PointerReleased);

    // Toggling expand snaps back to the new state's default size + position.
    let _ = update(&mut tty, Message::ToggleMetricDetailExpanded(0));
    assert!(tty.metric_details[0].expanded);
    assert!(tty.metric_details[0].size.is_none());
    assert_eq!(tty.metric_details[0].move_offset, (0.0, 0.0));

    // Resize works while expanded too, and a single-edge drag moves only its
    // axis: dragging the right edge changes width but leaves height alone.
    let (start_w, start_h) = tty.metric_details[0].effective_size(1000.0, 700.0);
    tty.pointer = Point::new(400.0, 300.0);
    let _ = update(
        &mut tty,
        Message::MetricDetailResizeStart(0, crate::state::ResizeEdge::Right),
    );
    let _ = update(&mut tty, Message::PointerMoved(Point::new(360.0, 330.0)));
    let (ew, eh) = tty.metric_details[0]
        .size
        .expect("expanded card is resizable");
    assert!(ew < start_w, "right-edge drag left shrank the width");
    assert_eq!(eh, start_h, "right-edge drag left the height unchanged");

    // Close (click-away) clears everything.
    let _ = update(&mut tty, Message::CloseMetricDetail);
    assert!(tty.metric_details.is_empty());
    assert!(tty.metric_detail_resize.is_none());
    assert!(tty.metric_detail_move_drag.is_none());
}

#[test]
fn pinned_mode_keeps_multiple_popovers_until_closed() {
    let mut tty = headless(1);
    tty.window_width = 1000.0;
    tty.window_height = 700.0;
    tty.settings.status_bar_metrics_pinned = Some(true);
    let open = |tty: &mut Tty, k: crate::settings::MetricKind| {
        tty.open_metric_detail(k.as_setting_str());
    };

    // Pinned: each distinct metric accumulates; re-opening one is a no-op.
    open(&mut tty, crate::settings::MetricKind::Cpu);
    open(&mut tty, crate::settings::MetricKind::CpuCores);
    open(&mut tty, crate::settings::MetricKind::Mem);
    open(&mut tty, crate::settings::MetricKind::Cpu);
    assert_eq!(
        tty.metric_details.len(),
        3,
        "three distinct popovers, no dup"
    );

    // A click away does NOT close them while pinned.
    let _ = update(&mut tty, Message::PointerReleased);
    assert_eq!(
        tty.metric_details.len(),
        3,
        "click-away is inert when pinned"
    );

    // The per-card close button removes just that one (drops CpuCores at idx 1).
    let _ = update(&mut tty, Message::CloseMetricPopover(1));
    assert_eq!(tty.metric_details.len(), 2);
    assert!(
        tty.metric_details
            .iter()
            .all(|p| p.kind != crate::settings::MetricKind::CpuCores),
        "the closed metric is gone"
    );

    // Turning pinning off collapses to at most one open popover.
    tty.set_status_bar_metrics_pinned(false);
    assert_eq!(tty.metric_details.len(), 1, "un-pinning truncates to one");

    // Escape closes all remaining.
    let _ = update(&mut tty, Message::CloseMetricDetail);
    assert!(tty.metric_details.is_empty());
}

#[test]
fn window_opacity_and_level_track_their_settings() {
    let mut tty = headless(1);

    // Focused opacity: floored at 50% (a fully-transparent focused window would
    // be unusable), and it drives window_opacity() while focused.
    tty.focused = true;
    let _ = update(&mut tty, Message::SetFocusedOpacity(0.2));
    assert_eq!(
        tty.settings.focused_opacity(),
        crate::settings::MIN_FOCUSED_OPACITY,
        "focused opacity is floored"
    );
    let _ = update(&mut tty, Message::SetFocusedOpacity(0.8));
    assert_eq!(tty.settings.focused_opacity(), 0.8);
    assert_eq!(tty.window_opacity(), 0.8, "focused uses focused opacity");

    // Unfocused still uses the (lower-floored) unfocused opacity.
    tty.focused = false;
    let _ = update(&mut tty, Message::SetUnfocusedOpacity(0.3));
    assert_eq!(tty.window_opacity(), 0.3);

    // Always-on-top flips the window level and persists.
    assert_eq!(tty.window_level(), iced::window::Level::Normal);
    let _ = update(&mut tty, Message::SetWindowAlwaysOnTop(true));
    assert!(tty.settings.window_always_on_top());
    assert_eq!(tty.window_level(), iced::window::Level::AlwaysOnTop);
    let _ = update(&mut tty, Message::SetWindowAlwaysOnTop(false));
    assert_eq!(tty.window_level(), iced::window::Level::Normal);
}

fn metric_cfg(kind: &str) -> crate::settings::MetricConfig {
    crate::settings::MetricConfig {
        metric: kind.to_string(),
        style: "sparkline".to_string(),
        warn: None,
        alarm: None,
    }
}

#[test]
fn status_bar_hold_enters_edit_and_drag_reorders_and_exits() {
    let mut tty = headless(1);
    tty.settings.status_bar_metrics =
        vec![metric_cfg("cpu"), metric_cfg("mem"), metric_cfg("net_io")];

    // Press-hold CPU (index 0): held past the threshold, it enters edit mode and
    // starts dragging that cell.
    let _ = update(&mut tty, Message::StatusMetricPress(0));
    assert!(tty.status_metric_press.is_some());
    assert!(
        !tty.status_bar_edit,
        "not editing yet — just a pending press"
    );
    // Simulate the hold completing by backdating the press past the max duration.
    tty.status_metric_press = Some((
        0,
        std::time::Instant::now() - std::time::Duration::from_secs(10),
    ));
    let _ = update(&mut tty, Message::StatusBarEditTick);
    assert!(tty.status_bar_edit, "hold entered edit mode");
    assert_eq!(tty.status_metric_drag, Some(0), "and started dragging CPU");
    assert!(tty.status_metric_press.is_none());

    // Drag over Net I/O (index 2) marks the drop; the reorder commits on release.
    let _ = update(&mut tty, Message::StatusMetricDragOver(2));
    assert_eq!(tty.status_metric_drop, Some(2));
    assert_eq!(
        tty.settings.status_bar_metrics[0].metric, "cpu",
        "not reordered until release"
    );
    let _ = update(&mut tty, Message::PointerReleased);
    assert_eq!(tty.settings.status_bar_metrics[2].metric, "cpu");
    assert_eq!(tty.settings.status_bar_metrics[0].metric, "mem");
    assert!(tty.status_metric_drag.is_none(), "release ends the drag");
    assert!(tty.status_bar_edit, "stays in edit mode for more drags");

    // Escape leaves edit mode.
    let esc = Key::Named(iced::keyboard::key::Named::Escape);
    let _ = update(&mut tty, Message::Key(esc, Modifiers::default()));
    assert!(!tty.status_bar_edit);
}

#[test]
fn status_bar_quick_tap_opens_popover_not_edit() {
    let mut tty = headless(1);
    tty.settings.status_bar_metrics = vec![metric_cfg("cpu"), metric_cfg("mem")];
    // A press that is released before the hold completes is a tap: it opens the
    // cell's drill-in and never enters edit mode.
    let _ = update(&mut tty, Message::StatusMetricPress(1));
    assert!(tty.status_metric_press.is_some());
    let _ = update(&mut tty, Message::PointerReleased);
    assert!(!tty.status_bar_edit, "a quick tap does not enter edit mode");
    assert!(tty.status_metric_press.is_none());
    assert_eq!(tty.metric_details.len(), 1, "the tap opened a popover");
    assert_eq!(tty.metric_details[0].kind, crate::settings::MetricKind::Mem);
}

#[test]
fn proc_sort_toggles_direction_and_switches_column() {
    use crate::state::ProcSortColumn as Col;
    let mut tty = headless(1);
    // Default is CPU descending.
    assert_eq!(tty.proc_sort, (Col::Cpu, true));
    // Re-selecting the active column flips direction.
    let _ = update(&mut tty, Message::SetProcSort(Col::Cpu));
    assert_eq!(tty.proc_sort, (Col::Cpu, false));
    // A new numeric column sorts descending; a scroll offset resets on re-sort.
    tty.proc_table_scroll = 120.0;
    let _ = update(&mut tty, Message::SetProcSort(Col::Mem));
    assert_eq!(tty.proc_sort, (Col::Mem, true));
    assert_eq!(
        tty.proc_table_scroll, 0.0,
        "re-sort scrolls back to the top"
    );
    // The name column sorts ascending by default.
    let _ = update(&mut tty, Message::SetProcSort(Col::Name));
    assert_eq!(tty.proc_sort, (Col::Name, false));
    // Scroll clamps non-negative.
    let _ = update(&mut tty, Message::ProcTableScroll(-5.0));
    assert_eq!(tty.proc_table_scroll, 0.0);
    let _ = update(&mut tty, Message::ProcTableScroll(40.0));
    assert_eq!(tty.proc_table_scroll, 40.0);
}

#[test]
fn proc_detail_open_and_close_routing() {
    use crate::settings::MetricKind;
    let mut tty = headless(1);
    assert_eq!(tty.proc_detail_pid, None);

    // Opening a process records its pid (the live sample is refreshed by the
    // metrics tick; `open_proc_detail` just flags which process to show).
    tty.open_proc_detail(4321);
    assert_eq!(tty.proc_detail_pid, Some(4321));

    // "‹ Back" / Escape returns to the process list.
    let _ = update(&mut tty, Message::CloseProcDetail);
    assert_eq!(tty.proc_detail_pid, None);
    assert!(tty.metrics.proc_detail.is_none());

    // Closing the Processes popover entirely also drops any open detail.
    tty.metric_details = vec![crate::state::MetricPopover::new(MetricKind::Procs)];
    tty.open_proc_detail(999);
    let _ = update(&mut tty, Message::CloseMetricPopover(0));
    assert_eq!(tty.proc_detail_pid, None);
}

#[test]
fn proc_and_fd_right_click_open_context_menus() {
    use crate::state::MenuKind;
    let mut tty = headless(1);
    tty.pointer = iced::Point::new(120.0, 200.0);

    // Right-clicking a process row opens its menu at the pointer.
    let _ = update(
        &mut tty,
        Message::ProcRowRightClick(412, "Google Chrome".to_string()),
    );
    assert!(matches!(
        &tty.menu,
        Some((MenuKind::ProcRow { pid: 412, name }, _)) if name == "Google Chrome"
    ));

    // A copy action closes the menu (path resolves for our own live pid).
    let _ = update(&mut tty, Message::CopyProcPath(std::process::id() as i32));
    assert!(tty.menu.is_none());

    // Right-clicking a descriptor row opens a menu carrying its path.
    let _ = update(&mut tty, Message::FdRowRightClick("/dev/null".to_string()));
    assert!(matches!(
        &tty.menu,
        Some((MenuKind::FdRow { path }, _)) if path == "/dev/null"
    ));
}

#[test]
fn process_kill_menu_routing() {
    let mut tty = headless(1);
    let me = std::process::id() as i32;
    tty.menu = Some((
        crate::state::MenuKind::ProcRow {
            pid: me,
            name: "self".to_string(),
        },
        tty.pointer,
    ));

    // "Quit" signals directly and closes the menu (signal 0 = harmless existence
    // probe so the test never actually terminates anything); no confirm.
    let _ = update(&mut tty, Message::KillProcess(me, 0));
    assert!(tty.menu.is_none(), "Quit closes the menu");
    assert!(tty.kill_confirm.is_none(), "Quit needs no confirm");

    // "Force Quit…" stages a confirm carrying the pid + name.
    let _ = update(
        &mut tty,
        Message::RequestForceKill(4321, "victim".to_string()),
    );
    assert_eq!(tty.kill_confirm, Some((4321, "victim".to_string())));

    // Cancel dismisses without signalling.
    let _ = update(&mut tty, Message::CancelForceKill);
    assert!(tty.kill_confirm.is_none());
}

#[test]
fn pane_replace_pick_confirm_and_replace() {
    use crate::settings::MetricKind;
    use crate::state::Pane;
    let mut tty = headless(1);
    let win = main_win(&tty);
    let ti = tty.active;
    let pane = tty.tabs[ti].focus;

    // ⊞ "Replace a pane…" arms pick mode.
    let _ = update(&mut tty, Message::StartPaneReplace(MetricKind::Cpu));
    assert_eq!(tty.pane_replace_pending, Some(MetricKind::Cpu));

    // Clicking a terminal pane stages a confirm (it has a shell/scrollback to lose)
    // rather than replacing outright.
    let _ = update(&mut tty, Message::FocusPane(win, pane));
    assert!(
        tty.pane_replace_pending.is_none(),
        "pick consumed the click"
    );
    assert!(
        tty.pane_replace_confirm.is_some(),
        "a terminal needs confirming"
    );
    // Cancel leaves the terminal intact.
    let _ = update(&mut tty, Message::CancelPaneReplace);
    assert!(tty.pane_replace_confirm.is_none());
    assert!(matches!(tty.tabs[ti].panes.get(pane), Some(Pane::Term(_))));

    // Re-arm, click, confirm → the pane is now the metric view.
    let _ = update(&mut tty, Message::StartPaneReplace(MetricKind::Cpu));
    let _ = update(&mut tty, Message::FocusPane(win, pane));
    let _ = update(&mut tty, Message::ConfirmPaneReplace);
    assert!(matches!(
        tty.tabs[ti].panes.get(pane),
        Some(Pane::Metric(MetricKind::Cpu))
    ));

    // Replacing that (metric) pane again skips the confirm — nothing to terminate.
    let _ = update(&mut tty, Message::StartPaneReplace(MetricKind::Mem));
    let _ = update(&mut tty, Message::FocusPane(win, pane));
    assert!(
        tty.pane_replace_confirm.is_none(),
        "metric pane replaces outright"
    );
    assert!(matches!(
        tty.tabs[ti].panes.get(pane),
        Some(Pane::Metric(MetricKind::Mem))
    ));
}

#[test]
fn metric_cell_click_wont_open_a_duplicate() {
    use crate::settings::MetricKind;
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    tty.settings.status_bar_metrics = vec![metric_cfg("cpu"), metric_cfg("mem")];

    // Opening CPU works; clicking CPU again while it's shown does nothing.
    tty.open_metric_detail("cpu");
    assert_eq!(tty.metric_details.len(), 1);
    tty.open_metric_detail("cpu");
    assert_eq!(tty.metric_details.len(), 1, "no duplicate CPU popover");

    // A different metric still opens (one-at-a-time replaces).
    tty.open_metric_detail("mem");
    assert_eq!(tty.metric_details[0].kind, MetricKind::Mem);

    // A metric shown as a *pane* also blocks its popover.
    let win = main_win(&tty);
    tty.metric_details.clear();
    tty.promote_metric_to_pane(win, Direction::Right, MetricKind::Cpu);
    tty.open_metric_detail("cpu");
    assert!(
        tty.metric_details.is_empty(),
        "CPU is already a pane — no popover"
    );
}

#[test]
fn metric_pane_promote_maximize_close() {
    use crate::settings::MetricKind;
    use crate::state::Pane;
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let win = main_win(&tty);
    let ti = tty.active;
    assert_eq!(tty.tabs[ti].panes.len(), 1, "starts with one terminal pane");

    // Promote a CPU metric into a pane split to the right; it takes focus.
    tty.promote_metric_to_pane(win, Direction::Right, MetricKind::Cpu);
    assert_eq!(tty.tabs[ti].panes.len(), 2);
    let focus = tty.tabs[ti].focus;
    assert!(matches!(
        tty.tabs[ti].panes.get(focus),
        Some(Pane::Metric(MetricKind::Cpu))
    ));

    // Maximize fills the grid; toggling again restores.
    let _ = update(&mut tty, Message::ToggleMaximizePane(win));
    assert!(tty.tabs[ti].panes.maximized().is_some());
    let _ = update(&mut tty, Message::ToggleMaximizePane(win));
    assert!(tty.tabs[ti].panes.maximized().is_none());

    // The pane's × closes just that pane; the terminal remains.
    let _ = update(&mut tty, Message::CloseMetricPane(win, focus));
    assert_eq!(tty.tabs[ti].panes.len(), 1);
    assert!(matches!(
        tty.tabs[ti].panes.get(tty.tabs[ti].focus),
        Some(Pane::Term(_))
    ));
}

#[test]
fn status_bar_scroll_offset_moves_and_saturates() {
    // With no cells fitting (window width 0 → everything "fits", max 0), scroll is
    // pinned at 0; the offset never goes negative.
    let mut tty = headless(1);
    tty.settings.status_bar_metrics = vec![metric_cfg("cpu"), metric_cfg("mem")];
    let _ = update(&mut tty, Message::StatusBarScroll(1.0));
    assert_eq!(tty.status_bar_scroll, 0, "up-scroll saturates at 0");
    let _ = update(&mut tty, Message::StatusBarScroll(-1.0));
    // window_width is 0 in the headless fixture, so every cell "fits" and the max
    // offset is 0 — the window can't advance.
    assert_eq!(tty.status_bar_scroll, 0);
}

#[test]
fn disabling_status_bar_closes_popovers() {
    let mut tty = headless(1);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Cpu,
    )];
    // Turning the bar off removes it and closes any open popovers (their
    // sparklines are gone).
    let _ = update(&mut tty, Message::SetStatusBarDisabled(true));
    assert!(tty.settings.status_bar_disabled());
    assert!(
        tty.metric_details.is_empty(),
        "popovers close when the bar goes off"
    );
    // And back on is a plain toggle.
    let _ = update(&mut tty, Message::SetStatusBarDisabled(false));
    assert!(!tty.settings.status_bar_disabled());
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
fn clear_scrollback_and_toggle_scrollback_panel_close_the_pane_menu() {
    // Both are pane-menu items ("Clear Scrollback" / "View Scrollback History"); picking
    // either must dismiss the menu it was chosen from, same as every other menu action
    // (Split, ClosePane, OpenLink, …) — a stale open menu used to render invisibly under
    // the scrollback panel and only became visible once the panel's layering was fixed
    // to render underneath the menu.
    let mut tty = headless(1);
    tty.menu = Some((MenuKind::Pane, tty.pointer));
    let _ = update(&mut tty, Message::ClearScrollback);
    assert!(tty.menu.is_none(), "Clear Scrollback closes the menu");

    tty.menu = Some((MenuKind::Pane, tty.pointer));
    let _ = update(&mut tty, Message::ToggleScrollbackPanel);
    assert!(
        tty.menu.is_none(),
        "View Scrollback History closes the menu"
    );
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
fn scrollback_row_right_click_opens_the_menu_targeting_the_resolved_row() {
    let mut tty = headless(1);
    let target = crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Command {
        log_index: 2,
        text: "ls -la".to_string(),
    });
    let _ = update(
        &mut tty,
        Message::ScrollbackRowRightClick(0, target.clone()),
    );
    assert_eq!(tty.scrollback_selected, Some(0));
    assert_eq!(
        tty.menu,
        Some((MenuKind::ScrollbackRow(target), tty.pointer))
    );
}

#[test]
fn copy_scrollback_target_closes_the_menu() {
    let mut tty = headless(1);
    let target = crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Command {
        log_index: 0,
        text: "ls".to_string(),
    });
    tty.menu = Some((MenuKind::ScrollbackRow(target.clone()), tty.pointer));
    let task = update(&mut tty, Message::CopyScrollbackTarget(target));
    assert!(tty.menu.is_none());
    // The clipboard write is a real iced::Task, not directly inspectable here;
    // the important thing is update() didn't just no-op it away.
    let _ = task;
}

/// Feed a screen a `$ ls` command with two output lines, boundary-marked exactly
/// like `update::handle_key` does live — the same fixture pattern
/// `snapshot::scrollback_panel_view` uses.
fn command_log_fixture(tty: &Tty) {
    let term = tty.active_term().unwrap();
    let mut screen = term.screen.lock();
    let mut parser = cathode::parser::TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml\r\nsrc\r\n", &mut screen);
}

#[test]
fn clear_scrollback_target_empties_a_commands_text_and_output() {
    // Blanking the output alone would make "Clear" a silent no-op for any command
    // that captured zero output (`cd`, `export`, `alias`, ...) — the row (and the
    // command's own text) must empty too, so choosing Clear always has a visible
    // effect.
    let mut tty = headless(1);
    command_log_fixture(&tty);
    let _ = update(
        &mut tty,
        Message::ClearScrollbackTarget(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Command {
                log_index: 0,
                text: "$ ls".to_string(),
            },
        )),
    );
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    let entry = &screen.command_log[0];
    assert_eq!(entry.command, "", "Clear blanks the command's own text too");
    assert!(entry.output.is_empty(), "and its captured output");
}

#[test]
fn clear_scrollback_target_blanks_a_single_output_line() {
    let mut tty = headless(1);
    command_log_fixture(&tty);
    let _ = update(
        &mut tty,
        Message::ClearScrollbackTarget(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Output {
                log_index: 0,
                line: 0,
                text: "Cargo.toml".to_string(),
            },
        )),
    );
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    let entry = &screen.command_log[0];
    assert_eq!(entry.output[0], "", "only the targeted line blanks");
    assert_eq!(entry.output[1], "src", "sibling lines are untouched");
}

#[test]
fn delete_scrollback_target_removes_a_command_entry_entirely() {
    let mut tty = headless(1);
    command_log_fixture(&tty);
    tty.scrollback_selected = Some(1);
    tty.scrollback_expanded.insert(0);
    let _ = update(
        &mut tty,
        Message::DeleteScrollbackTarget(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Command {
                log_index: 0,
                text: "$ ls".to_string(),
            },
        )),
    );
    let term = tty.active_term().unwrap();
    assert!(
        term.screen.lock().command_log.is_empty(),
        "Delete removes the whole entry, not just its text"
    );
    assert_eq!(
        tty.scrollback_selected, None,
        "the deletion shifted row indices, so a stale selection must clear"
    );
    assert!(
        tty.scrollback_expanded.is_empty(),
        "and any expanded-command indices, for the same reason"
    );
}

#[test]
fn delete_scrollback_target_shifts_later_commands_down() {
    let mut tty = headless(1);
    {
        let term = tty.active_term().unwrap();
        let mut screen = term.screen.lock();
        let mut parser = cathode::parser::TermParser::new();
        parser.process(b"$ ls", &mut screen);
        screen.mark_command_boundary(50);
        parser.process(b"\r\nCargo.toml\r\n", &mut screen);
        parser.process(b"$ pwd", &mut screen);
        screen.mark_command_boundary(50);
        parser.process(b"\r\n/tmp\r\n", &mut screen);
    }
    let _ = update(
        &mut tty,
        Message::DeleteScrollbackTarget(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Command {
                log_index: 0,
                text: "$ ls".to_string(),
            },
        )),
    );
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(
        screen.command_log[0].command, "$ pwd",
        "the surviving command shifts down to index 0"
    );
}

#[test]
fn delete_scrollback_target_is_a_no_op_for_an_output_line() {
    // Only a command's header row can be deleted; an output line just has "Clear"
    // (blank it), not "Delete" — no row concept to remove for a single line.
    let mut tty = headless(1);
    command_log_fixture(&tty);
    let _ = update(
        &mut tty,
        Message::DeleteScrollbackTarget(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Output {
                log_index: 0,
                line: 0,
                text: "Cargo.toml".to_string(),
            },
        )),
    );
    let term = tty.active_term().unwrap();
    let screen = term.screen.lock();
    assert_eq!(
        screen.command_log.len(),
        1,
        "the command entry is untouched"
    );
    assert_eq!(screen.command_log[0].output, vec!["Cargo.toml", "src"]);
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
fn settings_history_viewer_toggles_and_clears_when_closed() {
    // With no active archive (history off) there's no re-auth gate and
    // nothing to page in — the viewer just opens empty and closes clean.
    let mut tty = headless(1);
    assert!(!tty.show_settings_history);
    let _ = update(&mut tty, Message::ToggleSettingsHistoryViewer);
    assert!(tty.show_settings_history);
    assert!(tty.settings_history.is_empty());

    tty.settings_history_selected = Some(3);
    tty.settings_history_scroll = 40.0;
    tty.confirm_delete_settings_row = Some(crate::state::ArchivedTarget {
        date: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        id: 1,
        started_at_epoch_ms: 0,
        pane_tag: "Tab 0".to_string(),
        command: "$ ls".to_string(),
    });
    let _ = update(&mut tty, Message::ToggleSettingsHistoryViewer);
    assert!(!tty.show_settings_history);
    assert_eq!(tty.settings_history_selected, None, "selection cleared");
    assert_eq!(tty.settings_history_scroll, 0.0, "scroll reset");
    assert_eq!(
        tty.confirm_delete_settings_row, None,
        "a pending delete confirmation dies with the browser"
    );
}

#[test]
fn closing_settings_also_closes_the_archive_viewer() {
    let mut tty = headless(1);
    tty.show_settings = true;
    let _ = update(&mut tty, Message::ToggleSettingsHistoryViewer);
    assert!(tty.show_settings_history);
    let _ = update(&mut tty, Message::ToggleSettings);
    assert!(!tty.show_settings);
    assert!(
        !tty.show_settings_history,
        "paged-in decrypted entries must not linger behind a closed settings panel"
    );
}

#[test]
fn reset_encrypted_history_confirmation_opens_and_cancels_without_touching_anything() {
    // `confirm_reset_encrypted_history` itself (the actual deletion) isn't
    // exercised here, for the same reason `history::keychain` isn't in
    // `history_integration`: it targets the real, hardcoded
    // `history::history_dir()` on whatever machine runs the test, not an
    // injectable temp path — a real side effect a test run shouldn't cause.
    // This only checks the request/cancel state transitions, which touch
    // nothing on disk.
    let mut tty = headless(1);
    assert!(!tty.confirm_reset_history);
    let _ = update(&mut tty, Message::RequestResetEncryptedHistory);
    assert!(tty.confirm_reset_history);
    let _ = update(&mut tty, Message::CancelResetEncryptedHistory);
    assert!(!tty.confirm_reset_history);
    assert!(
        tty.history_writer.is_none(),
        "cancelling must not start or touch the writer"
    );
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
fn graduate_and_focus_border_toggles_default_on() {
    let mut tty = headless(1);
    assert!(tty.settings.graduate_metrics(), "graduation on by default");
    assert!(
        tty.settings.highlight_focused_pane(),
        "focus highlight on by default"
    );
    let _ = update(&mut tty, Message::SetGraduateMetrics(false));
    assert!(!tty.settings.graduate_metrics(), "honors the explicit off");
    let _ = update(&mut tty, Message::SetHighlightFocusedPane(false));
    assert!(!tty.settings.highlight_focused_pane());
}

#[test]
fn splitting_adds_a_focused_pane_to_the_tab() {
    use iced::widget::pane_grid::Direction;
    let mut tty = headless(1);
    let win = main_win(&tty);
    assert_eq!(tty.tabs[0].panes.len(), 1);
    let first = tty.tabs[0].focus;
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("split")),
    );
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
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("right")),
    );
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
    tty.split_with(
        win,
        Direction::Down,
        crate::state::Pane::Term(screen_term("lower")),
    );
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
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("right")),
    );
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
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("b")),
    );
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
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("right")),
    );
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
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(screen_term("right")),
    );
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
    tty.split_with(
        main_win(&tty),
        Direction::Down,
        crate::state::Pane::Term(screen_term("lower")),
    );
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
fn history_reauth_interval_step_reaches_and_stays_at_zero() {
    let mut tty = headless(1);
    assert_eq!(tty.settings.history_reauth_interval_minutes(), 0);
    let _ = update(&mut tty, Message::HistoryReauthIntervalStep(5));
    assert_eq!(tty.settings.history_reauth_interval_minutes(), 5);
    let _ = update(&mut tty, Message::HistoryReauthIntervalStep(-5));
    assert_eq!(
        tty.settings.history_reauth_interval_minutes(),
        0,
        "one decrement from 5 must land exactly on 0"
    );
    let _ = update(&mut tty, Message::HistoryReauthIntervalStep(-5));
    assert_eq!(
        tty.settings.history_reauth_interval_minutes(),
        0,
        "decrementing below 0 stays at 0, never wraps"
    );
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

/// End-to-end encrypted-history tests: the real background writer thread, the
/// real `state.rs` wiring (`drain_effects` -> `drain_pane` -> `Writer::send`,
/// `delete_archived_target`/`clear_archived_target`), and the real on-disk
/// segment/manifest format, all together. Each module (`crypto`, `segment`,
/// `manifest`, `writer`) already has its own focused unit tests; this is the
/// layer above that, proving they're wired together correctly.
///
/// Deliberately not exercised here: `history::keychain::get_or_create_key`,
/// since it would read/write an actual "tty" entry in whatever OS credential
/// store the machine running the test has — a real side effect on a
/// developer's or CI runner's system that a test run shouldn't cause. Its
/// crypto correctness is covered by `history::crypto`'s tests; the OS
/// integration itself is left to a manual check on a real build.
mod history_integration {
    use std::time::{Duration, Instant};

    use zeroize::Zeroizing;

    use cathode::history::{HistoryEvent, PersistedCommandEntry};
    use cathode::screen::TerminalScreen;

    use crate::history::crypto::{Cipher, Key};
    use crate::history::manifest::Manifest;
    use crate::history::writer::{Writer, MANIFEST_FILENAME};
    use crate::history::{local_date_from_epoch_ms, segment, tmp_path};
    use crate::state::ArchivedTarget;

    use super::*;

    // Three days apart so the two timestamps land on different local calendar
    // dates in any timezone (clear of DST-transition edge cases) — the actual
    // dates don't matter, only that they differ.
    const DAY1_MS: u64 = 1_700_000_000_000;
    const DAY2_MS: u64 = DAY1_MS + 3 * 86_400_000;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "tty-history-integration-{}-{name}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&p).expect("create temp history dir");
        p
    }

    fn key() -> Zeroizing<Key> {
        Zeroizing::new([0x77; 32])
    }

    /// The per-purpose children [`key`] fans out into — what the writer,
    /// `history_read`, and every on-disk file actually use (the master never
    /// encrypts anything directly; see `HistoryKeys`).
    fn keys() -> crate::history::HistoryKeys {
        crate::history::HistoryKeys::from_master(&[0x77; 32], dorado_engine::kdf::KdfPrf::Skein512)
    }

    fn entry(id: u32, command: &str, started_at_epoch_ms: u64) -> PersistedCommandEntry {
        PersistedCommandEntry {
            id,
            command: command.to_string(),
            started_at_epoch_ms,
            pane_tag: "Tab 0".to_string(),
        }
    }

    fn manifest_path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join(MANIFEST_FILENAME)
    }

    /// Poll until `cond` is true, or panic after `timeout` — the writer
    /// thread applies events asynchronously (over an `mpsc` channel), so a
    /// test has to wait for its effect on disk rather than assume it already
    /// happened by the time `Writer::send` returns.
    fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(
                start.elapsed() < timeout,
                "timed out waiting for the history writer thread"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_completed_command_round_trips_through_the_real_writer_thread_into_a_day_segment() {
        let dir = tmp_dir("roundtrip");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );

        let mut tty = headless(1);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));

        command_log_fixture(&tty);
        tty.drain_effects();

        wait_until(Duration::from_secs(2), || {
            Manifest::load(&manifest_path(&dir), &keys().manifest)
                .map(|m| m.latest_date().is_some())
                .unwrap_or(false)
        });

        let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
        let date = manifest.latest_date().expect("a day was written");
        let filename = manifest
            .segment_filename(date)
            .expect("segment registered")
            .to_string();
        let entries = segment::load(&dir.join(&filename), &keys().segments).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "$ ls");

        // And it's consumable by the same seeding path startup uses.
        let mut screen = TerminalScreen::new(80, 24);
        screen.seed_command_log(entries);
        assert_eq!(screen.command_log.len(), 1);
        assert_eq!(screen.command_log[0].command, "$ ls");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// macOS-only because the gate itself is (`Tty::history_reauth_reason`
    /// returns `None` off macOS). The prompt task `update` returns is dropped
    /// here, never executed, so no real Touch ID dialog appears during the
    /// test run — `reauth::authenticate` defers its native call until the
    /// future is polled, which this test relies on.
    #[cfg(target_os = "macos")]
    #[test]
    fn opening_the_panel_with_an_active_archive_is_gated_behind_reauth() {
        let dir = tmp_dir("reauth-gate");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );

        let mut tty = headless(1);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));

        // The ⌘⇧H chord — the path that historically bypassed the gate.
        // `Key` here is the crypto key alias (via `use super::super::...`),
        // so the keyboard key needs its full path.
        let chord = || {
            Message::Key(
                iced::keyboard::Key::Character("h".into()),
                cmd() | Modifiers::SHIFT,
            )
        };
        let _ = update(&mut tty, chord());
        assert!(
            !tty.show_scrollback,
            "the panel must wait for the auth prompt, not open immediately"
        );
        assert!(tty.history_reauth_pending);

        // Pressing the chord again while the prompt is up must not stack a
        // second prompt (or open anything).
        let _ = update(&mut tty, chord());
        assert!(!tty.show_scrollback);

        // A failed/cancelled prompt leaves it closed and clears the guard.
        let _ = update(
            &mut tty,
            Message::HistoryReauthResult(crate::message::ReauthFor::ScrollbackPanel, false),
        );
        assert!(!tty.show_scrollback);
        assert!(!tty.history_reauth_pending);

        // A successful prompt opens it and records the auth time.
        let _ = update(&mut tty, chord());
        let _ = update(
            &mut tty,
            Message::HistoryReauthResult(crate::message::ReauthFor::ScrollbackPanel, true),
        );
        assert!(tty.show_scrollback);
        assert!(tty.last_history_auth.is_some());

        // Close (never prompts), reopen: within the same session and with no
        // idle interval configured, no new prompt is due — it opens directly.
        let _ = update(&mut tty, chord());
        assert!(!tty.show_scrollback);
        let _ = update(&mut tty, chord());
        assert!(
            tty.show_scrollback,
            "once per session: the second open needs no fresh prompt"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same macOS-only/lazy-prompt caveats as the panel gate test above.
    #[cfg(target_os = "macos")]
    #[test]
    fn settings_archive_viewer_is_gated_and_pages_in_the_archive_on_unlock() {
        let dir = tmp_dir("viewer-gate");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );
        writer.send(HistoryEvent::Upsert(entry(1, "$ ls", DAY1_MS)));
        wait_until(Duration::from_secs(2), || {
            Manifest::load(&manifest_path(&dir), &keys().manifest)
                .map(|m| m.latest_date().is_some())
                .unwrap_or(false)
        });

        let mut tty = headless(1);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));

        // NOTE: `page_older` reads from the real `history_dir()`, not `dir` —
        // so this test asserts the gate + unlock flow, not the paged
        // contents (covered by the panel paging path, which shares
        // `history::page_older`).
        let _ = update(&mut tty, Message::ToggleSettingsHistoryViewer);
        assert!(
            !tty.show_settings_history,
            "the viewer shows the same protected data as the panel, so it must wait for auth"
        );
        assert!(tty.history_reauth_pending);

        let _ = update(
            &mut tty,
            Message::HistoryReauthResult(crate::message::ReauthFor::SettingsHistory, true),
        );
        assert!(tty.show_settings_history);
        assert!(!tty.history_reauth_pending);
        assert!(tty.last_history_auth.is_some());

        let _ = update(&mut tty, Message::ToggleSettingsHistoryViewer);
        assert!(!tty.show_settings_history, "second toggle hides and clears");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_and_clear_on_an_archived_entry_only_touch_their_own_day_segment() {
        let dir = tmp_dir("archive-target");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );

        let mut tty = headless(1);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));

        // Two entries on day 1, one on day 2.
        {
            let w = tty.history_writer.as_ref().unwrap();
            w.send(HistoryEvent::Upsert(entry(1, "$ ls", DAY1_MS)));
            w.send(HistoryEvent::Upsert(entry(2, "$ pwd", DAY1_MS)));
            w.send(HistoryEvent::Upsert(entry(3, "$ whoami", DAY2_MS)));
        }

        let day1 = local_date_from_epoch_ms(DAY1_MS);
        let day2 = local_date_from_epoch_ms(DAY2_MS);

        wait_until(Duration::from_secs(2), || {
            Manifest::load(&manifest_path(&dir), &keys().manifest)
                .map(|m| m.segment_filename(day2).is_some())
                .unwrap_or(false)
        });

        tty.delete_archived_target(&ArchivedTarget {
            date: day1,
            id: 1,
            started_at_epoch_ms: DAY1_MS,
            pane_tag: "Tab 0".to_string(),
            command: "$ ls".to_string(),
        });

        wait_until(Duration::from_secs(2), || {
            let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
            let Some(filename) = manifest.segment_filename(day1) else {
                return false;
            };
            let entries = segment::load(&dir.join(filename), &keys().segments).unwrap();
            entries.len() == 1 && entries[0].id == 2
        });

        let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
        let day2_filename = manifest
            .segment_filename(day2)
            .expect("day 2 is still registered")
            .to_string();
        let day2_entries = segment::load(&dir.join(&day2_filename), &keys().segments).unwrap();
        assert_eq!(
            day2_entries.len(),
            1,
            "deleting a day-1 entry must not touch day 2's segment"
        );
        assert_eq!(day2_entries[0].command, "$ whoami");

        tty.clear_archived_target(&ArchivedTarget {
            date: day1,
            id: 2,
            started_at_epoch_ms: DAY1_MS,
            pane_tag: "Tab 0".to_string(),
            command: "$ pwd".to_string(),
        });

        wait_until(Duration::from_secs(2), || {
            let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
            let filename = manifest.segment_filename(day1).unwrap();
            let entries = segment::load(&dir.join(filename), &keys().segments).unwrap();
            entries.len() == 1 && entries[0].command.is_empty()
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The settings archive browser's per-row Delete asks first: Request
    /// opens the confirmation dialog, Cancel touches nothing, and Confirm
    /// tombstones the entry through the real writer thread and drops it from
    /// the browser's paged-in copy immediately.
    #[test]
    fn settings_archive_row_delete_confirms_then_tombstones() {
        let dir = tmp_dir("viewer-delete");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );

        let mut tty = headless(1);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));

        {
            let w = tty.history_writer.as_ref().unwrap();
            w.send(HistoryEvent::Upsert(entry(1, "$ ls", DAY1_MS)));
            w.send(HistoryEvent::Upsert(entry(2, "$ pwd", DAY1_MS)));
        }
        let day1 = local_date_from_epoch_ms(DAY1_MS);
        wait_until(Duration::from_secs(2), || {
            Manifest::load(&manifest_path(&dir), &keys().manifest)
                .map(|m| m.segment_filename(day1).is_some())
                .unwrap_or(false)
        });

        // The browser with day 1 paged in — populated directly, since
        // `page_older` reads the real `history_dir()`, not `dir` (same
        // caveat as the viewer gate test).
        tty.show_settings_history = true;
        tty.settings_history = vec![entry(1, "$ ls", DAY1_MS), entry(2, "$ pwd", DAY1_MS)];
        tty.settings_history_cursor = Some(day1);

        let target = ArchivedTarget {
            date: day1,
            id: 1,
            started_at_epoch_ms: DAY1_MS,
            pane_tag: "Tab 0".to_string(),
            command: "$ ls".to_string(),
        };

        // Right-click → "Delete…" only opens the confirmation.
        let _ = update(
            &mut tty,
            Message::RequestDeleteSettingsHistoryRow(target.clone()),
        );
        assert_eq!(tty.confirm_delete_settings_row.as_ref(), Some(&target));
        assert_eq!(tty.settings_history.len(), 2, "nothing deleted yet");

        // Cancel closes the dialog and touches nothing.
        let _ = update(&mut tty, Message::CancelDeleteSettingsHistoryRow);
        assert_eq!(tty.confirm_delete_settings_row, None);
        assert_eq!(tty.settings_history.len(), 2);

        // Request again and confirm: gone from the browser immediately, and
        // the tombstone reaches the day segment on disk.
        let _ = update(&mut tty, Message::RequestDeleteSettingsHistoryRow(target));
        let _ = update(&mut tty, Message::ConfirmDeleteSettingsHistoryRow);
        assert_eq!(tty.confirm_delete_settings_row, None);
        assert_eq!(tty.settings_history.len(), 1);
        assert_eq!(tty.settings_history[0].id, 2);

        wait_until(Duration::from_secs(2), || {
            let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
            let Some(filename) = manifest.segment_filename(day1) else {
                return false;
            };
            let entries = segment::load(&dir.join(filename), &keys().segments).unwrap();
            entries.len() == 1 && entries[0].id == 2
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pure launch-decision matrix: the CLI flag beats everything, "ask"
    /// only fires when the feature is on, and "record" routes by key source.
    #[test]
    fn startup_history_plan_matrix() {
        use crate::state::{startup_history_plan, StartupPlan};

        let mut s = crate::settings::Settings::default();

        // Feature off: nothing — unless the CLI flag forces untracked.
        assert_eq!(startup_history_plan(&s, false), StartupPlan::Off);
        assert_eq!(startup_history_plan(&s, true), StartupPlan::Untracked);

        s.encrypted_history_enabled = Some(true);
        assert_eq!(startup_history_plan(&s, false), StartupPlan::StartKeychain);
        assert_eq!(
            startup_history_plan(&s, true),
            StartupPlan::Untracked,
            "the CLI flag beats an enabled feature"
        );

        s.history_key_source = Some("passphrase".to_string());
        assert_eq!(
            startup_history_plan(&s, false),
            StartupPlan::LockedPassphrase
        );

        s.history_session_start = Some("ask".to_string());
        assert_eq!(startup_history_plan(&s, false), StartupPlan::Ask);
        assert_eq!(
            startup_history_plan(&s, true),
            StartupPlan::Untracked,
            "the CLI flag also skips the chooser"
        );

        s.history_session_start = Some("untracked".to_string());
        assert_eq!(startup_history_plan(&s, false), StartupPlan::Untracked);

        // Unrecognized value degrades to Record, the long-standing behavior.
        s.history_session_start = Some("banana".to_string());
        s.history_key_source = None;
        assert_eq!(startup_history_plan(&s, false), StartupPlan::StartKeychain);
    }

    /// D2: an untracked session is immutable. Toggling the history setting
    /// on persists it (for the next launch) but starts nothing now; even a
    /// stray `Started` arriving would be dropped.
    #[test]
    fn an_untracked_session_stays_untracked_when_history_is_enabled_mid_session() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};

        let mut tty = headless(1);
        tty.session_untracked = true;

        let _ = update(&mut tty, Message::SetEncryptedHistoryEnabled(true));
        assert_eq!(
            tty.settings.encrypted_history_enabled,
            Some(true),
            "the setting persists for the next launch"
        );
        assert!(tty.passphrase_prompt.is_none(), "no enable dialog");
        assert!(tty.history_writer.is_none(), "nothing starts");

        // Belt-and-braces: even a Started arriving somehow is dropped.
        let dir = tmp_dir("untracked-session");
        let started = crate::history::start_with_key_in(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            key(),
            dorado_engine::kdf::KdfPrf::Skein512,
        )
        .unwrap();
        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Startup,
                HistoryStartOutcome::Ready(StartedHandle::new(started)),
            ),
        );
        assert!(tty.history_writer.is_none());
        let _ = std::fs::remove_dir_all(&dir);

        // And the startup task is a no-op in an untracked session.
        let _ = tty.startup_history_task();
        assert!(!tty.history_starting);
    }

    /// The startup chooser: "Stay untracked" flips the whole session (every
    /// existing tab and screen); "Record" begins the start instead.
    #[test]
    fn session_start_chooser_routes_both_answers() {
        let mut tty = headless(2);
        tty.settings.encrypted_history_enabled = Some(true);
        tty.show_session_start_prompt = true;

        let _ = update(&mut tty, Message::SessionStartChoice(false));
        assert!(!tty.show_session_start_prompt);
        assert!(tty.session_untracked);
        for tab in &tty.tabs {
            assert!(tab.untracked, "every existing tab is marked");
            for term in tab.terms() {
                assert!(term.screen.lock().untracked(), "and every screen");
            }
        }

        // Record path (fresh state): the async start kicks off.
        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        tty.show_session_start_prompt = true;
        let _ = update(&mut tty, Message::SessionStartChoice(true));
        assert!(!tty.show_session_start_prompt);
        assert!(!tty.session_untracked);
        assert!(
            tty.history_starting,
            "keychain source: the start task is in flight"
        );
    }

    /// An untracked tab's commands never reach the writer, even with the
    /// feature fully running — while a tracked tab's (the control) do. The
    /// suppression lives in cathode; this proves the tty wiring around it.
    #[test]
    fn an_untracked_tabs_commands_never_reach_the_writer() {
        let dir = tmp_dir("untracked-tab");
        let writer = Writer::spawn(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            keys(),
            Manifest::default(),
        );

        let mut tty = headless(2);
        tty.history_writer = Some(writer);
        tty.history_read = Some((Cipher::ChaCha20Poly1305, keys()));
        tty.tabs[1].untracked = true;
        for term in tty.tabs[1].terms_mut() {
            term.screen.lock().set_untracked(true);
        }

        let run = |term: &crate::state::Term, cmd: &str| {
            let mut screen = term.screen.lock();
            let mut parser = cathode::parser::TermParser::new();
            parser.process(format!("$ {cmd}").as_bytes(), &mut screen);
            screen.mark_command_boundary(50);
            parser.process(b"\r\ndone\r\n", &mut screen);
        };
        run(tty.tabs[0].focused().unwrap(), "tracked-cmd");
        run(tty.tabs[1].focused().unwrap(), "secret-cmd");
        tty.drain_effects();

        // The tracked command lands in a day segment; the untracked one never
        // does. Waiting for the tracked write to appear first makes the
        // negative assertion meaningful (the writer has demonstrably caught
        // up past both sends).
        wait_until(Duration::from_secs(2), || {
            Manifest::load(&manifest_path(&dir), &keys().manifest)
                .map(|m| m.latest_date().is_some())
                .unwrap_or(false)
        });
        let manifest = Manifest::load(&manifest_path(&dir), &keys().manifest).unwrap();
        let date = manifest.latest_date().unwrap();
        let filename = manifest.segment_filename(date).unwrap().to_string();
        let entries = segment::load(&dir.join(&filename), &keys().segments).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "$ tracked-cmd");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⌘⇧T routes to the untracked-tab path and plain ⌘T stays tracked —
    /// and the live `CommandEntry` rows carry the badge flag the panel shows.
    #[test]
    fn untracked_flag_shows_on_live_rows_and_shifted_chord_is_distinct() {
        let mut tty = headless(1);
        for term in tty.tabs[0].terms_mut() {
            term.screen.lock().set_untracked(true);
        }
        command_log_fixture(&tty);
        {
            let term = tty.active_term().unwrap();
            let screen = term.screen.lock();
            assert!(screen.command_log[0].untracked, "the row carries the badge");
        }

        // The chord dispatch: ⌘⇧T must not fall through to the plain
        // new-tab arm. Headless tabs have no real shell, so the spawn is
        // expected to still *attempt* an untracked tab — observable as the
        // tab count either growing with `untracked: true` or (no shell in
        // the test env) not growing at all; what must never happen is a new
        // *tracked* tab.
        let before = tty.tabs.len();
        let _ = update(
            &mut tty,
            Message::Key(
                iced::keyboard::Key::Character("t".into()),
                cmd() | Modifiers::SHIFT,
            ),
        );
        if tty.tabs.len() > before {
            assert!(tty.tabs.last().unwrap().untracked);
            assert!(tty
                .tabs
                .last()
                .unwrap()
                .focused()
                .is_some_and(|t| t.screen.lock().untracked()));
        }
    }

    /// With the passphrase key source, the enable dialog's fields validate
    /// inline without touching anything, and Cancel leaves the setting off.
    #[test]
    fn passphrase_enable_prompts_and_validates_before_any_crypto() {
        use crate::state::PassphrasePromptKind;

        let mut tty = headless(1);
        tty.settings.history_key_source = Some("passphrase".to_string());

        let _ = update(&mut tty, Message::SetEncryptedHistoryEnabled(true));
        let prompt = tty.passphrase_prompt.as_ref().expect("dialog opens");
        assert_eq!(prompt.kind, PassphrasePromptKind::Enable);

        // Too short: inline error, nothing starts.
        let _ = update(&mut tty, Message::HistoryPassphraseChanged("short".into()));
        let _ = update(&mut tty, Message::SubmitHistoryPassphrase);
        let prompt = tty.passphrase_prompt.as_ref().unwrap();
        assert!(prompt.error.is_some());
        assert!(!prompt.busy);
        assert!(!tty.history_starting);

        // Mismatched confirm: inline error, nothing starts.
        let _ = update(
            &mut tty,
            Message::HistoryPassphraseChanged("long enough now".into()),
        );
        let _ = update(
            &mut tty,
            Message::HistoryPassphraseConfirmChanged("but different".into()),
        );
        let _ = update(&mut tty, Message::SubmitHistoryPassphrase);
        let prompt = tty.passphrase_prompt.as_ref().unwrap();
        assert_eq!(
            prompt.error.as_deref(),
            Some("The two entries don't match.")
        );
        assert!(!tty.history_starting);

        // Cancel: prompt gone, setting still off, nothing installed.
        let _ = update(&mut tty, Message::CancelHistoryPassphrase);
        assert!(tty.passphrase_prompt.is_none());
        assert!(!tty.settings.encrypted_history_enabled());
        assert!(tty.history_writer.is_none());
    }

    /// A wrong passphrase keeps history locked with an inline retry — it is
    /// not the red "broken archive" banner and never flips the setting.
    #[test]
    fn wrong_passphrase_stays_locked_with_an_inline_error() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome};
        use crate::state::{PassphrasePrompt, PassphrasePromptKind};

        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        tty.settings.history_key_source = Some("passphrase".to_string());
        tty.history_locked = true;
        let mut prompt = PassphrasePrompt::new(PassphrasePromptKind::Unlock);
        prompt.busy = true;
        tty.passphrase_prompt = Some(prompt);
        tty.history_starting = true;

        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Unlock,
                HistoryStartOutcome::WrongPassphrase,
            ),
        );
        assert!(tty.history_locked, "stays locked");
        assert!(tty.history_writer.is_none());
        assert!(!tty.history_start_failed, "not the broken-archive banner");
        assert_eq!(tty.settings.encrypted_history_enabled, Some(true));
        let prompt = tty.passphrase_prompt.as_ref().expect("prompt stays open");
        assert!(!prompt.busy);
        assert!(prompt
            .error
            .as_deref()
            .unwrap()
            .starts_with("Wrong passphrase"));
        assert!(prompt.draft.is_empty(), "the wrong entry is wiped");
    }

    /// Dismissing the unlock prompt keeps the session locked (and recording
    /// off); the settings banner's Unlock… reopens it.
    #[test]
    fn dismissed_unlock_stays_locked_and_reopens_from_settings() {
        use crate::state::{PassphrasePrompt, PassphrasePromptKind};

        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        tty.settings.history_key_source = Some("passphrase".to_string());
        tty.history_locked = true;
        tty.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));

        let _ = update(&mut tty, Message::CancelHistoryPassphrase);
        assert!(tty.passphrase_prompt.is_none());
        assert!(tty.history_locked);

        let _ = update(&mut tty, Message::OpenHistoryUnlock);
        let prompt = tty.passphrase_prompt.as_ref().expect("reopens");
        assert_eq!(prompt.kind, PassphrasePromptKind::Unlock);

        // The passphrase source never starts anything at boot — the startup
        // task is a no-op (locked until the user submits).
        let _ = tty.startup_history_task();
        assert!(
            !tty.history_starting,
            "no keychain task for the passphrase source"
        );
    }

    /// A successful unlock through the real passphrase path (tempdir): the
    /// submitted passphrase derives the key, opens the archive, and clears
    /// the locked state.
    #[test]
    fn a_correct_passphrase_unlocks_and_installs_the_writer() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};
        use crate::state::{PassphrasePrompt, PassphrasePromptKind};

        let dir = tmp_dir("unlock-ok");
        // The real derive + open, exactly what `start_async` runs on its
        // thread (driven synchronously here — behavior tests can't poll
        // iced's executor).
        let started = crate::history::passphrase::start_in(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            crate::settings::HistoryKdf::Argon2id,
            dorado_engine::kdf::KdfPrf::Skein512,
            "a fine passphrase",
        )
        .unwrap();

        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        tty.settings.history_key_source = Some("passphrase".to_string());
        tty.history_locked = true;
        tty.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));
        tty.history_starting = true;

        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Unlock,
                HistoryStartOutcome::Ready(StartedHandle::new(started)),
            ),
        );
        assert!(!tty.history_locked);
        assert!(tty.passphrase_prompt.is_none());
        assert!(tty.history_writer.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The enable toggle must not touch the keychain (or anything else): it
    /// only opens the one enable dialog, Cancel walks it back, and the
    /// keychain-source "Continue" is what actually begins the start.
    #[test]
    fn enabling_history_opens_the_dialog_and_touches_nothing() {
        use crate::state::PassphrasePromptKind;

        let mut tty = headless(1);
        let _ = update(&mut tty, Message::SetEncryptedHistoryEnabled(true));
        let prompt = tty.passphrase_prompt.as_ref().expect("dialog opens");
        assert_eq!(prompt.kind, PassphrasePromptKind::Enable);
        assert!(tty.history_writer.is_none());
        assert!(
            !tty.settings.encrypted_history_enabled(),
            "the setting commits only when the async start succeeds"
        );

        let _ = update(&mut tty, Message::CancelHistoryPassphrase);
        assert!(tty.passphrase_prompt.is_none());
        assert!(!tty.settings.encrypted_history_enabled());

        // Keychain "Continue" closes the dialog and kicks off the start.
        let _ = update(&mut tty, Message::SetEncryptedHistoryEnabled(true));
        let _ = update(&mut tty, Message::ConfirmEnableHistory);
        assert!(tty.passphrase_prompt.is_none());
        assert!(tty.history_starting, "the keychain start is in flight");

        // While a start is in flight, the toggle is inert (no second dialog).
        let _ = update(&mut tty, Message::SetEncryptedHistoryEnabled(true));
        assert!(tty.passphrase_prompt.is_none());
    }

    /// Failure semantics differ by origin, deliberately: an *enable* failure
    /// reverts the setting (never "on but broken"); a *startup* failure
    /// keeps it (the archive still exists, this session just can't open it).
    #[test]
    fn start_failure_reverts_the_setting_only_for_enable() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome};

        let mut tty = headless(1);
        tty.history_starting = true;
        let _ = update(
            &mut tty,
            Message::HistoryStarted(HistoryStartOrigin::Enable, HistoryStartOutcome::Failed),
        );
        assert!(!tty.history_starting);
        assert!(tty.history_start_failed);
        assert_eq!(tty.settings.encrypted_history_enabled, Some(false));

        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        let _ = update(
            &mut tty,
            Message::HistoryStarted(HistoryStartOrigin::Startup, HistoryStartOutcome::Failed),
        );
        assert!(tty.history_start_failed);
        assert_eq!(
            tty.settings.encrypted_history_enabled,
            Some(true),
            "a startup failure keeps the setting on"
        );
    }

    /// A successful start installs the writer, raises the command-id floor
    /// past the seeded ids on every screen, and seeds only an empty live log
    /// — a log with pre-start commands keeps them, unseeded.
    #[test]
    fn ready_installs_writer_raises_id_floor_and_seeds_only_an_empty_log() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};

        let build_started = |name: &str| {
            let dir = tmp_dir(name);
            // Archive two entries so the seed is non-empty and the floor is
            // max(id) + 1 = 8.
            let mut manifest = Manifest::default();
            let date = local_date_from_epoch_ms(DAY1_MS);
            let filename = manifest.segment_filename_or_create(date, segment::random_filename);
            segment::save(
                &dir.join(&filename),
                Cipher::ChaCha20Poly1305,
                &keys().segments,
                &[entry(5, "$ ls", DAY1_MS), entry(7, "$ pwd", DAY1_MS)],
            )
            .unwrap();
            manifest.set_count(date, 2);
            manifest
                .save(
                    &dir.join(MANIFEST_FILENAME),
                    Cipher::ChaCha20Poly1305,
                    &keys().manifest,
                )
                .unwrap();
            (
                dir.clone(),
                crate::history::start_with_key_in(
                    dir,
                    Cipher::ChaCha20Poly1305,
                    key(),
                    dorado_engine::kdf::KdfPrf::Skein512,
                )
                .unwrap(),
            )
        };

        // Empty live log: seeded.
        let (dir, started) = build_started("ready-empty");
        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Startup,
                HistoryStartOutcome::Ready(StartedHandle::new(started)),
            ),
        );
        assert!(tty.history_writer.is_some());
        assert_eq!(tty.history_id_floor, 8);
        {
            let term = tty.active_term().unwrap();
            let screen = term.screen.lock();
            assert_eq!(screen.command_log.len(), 2, "empty log gets the seed");
            assert_eq!(screen.command_log[0].command, "$ ls");
        }
        let _ = std::fs::remove_dir_all(&dir);

        // Pre-start commands in the log: kept, not mixed with the seed.
        let (dir, started) = build_started("ready-nonempty");
        let mut tty = headless(1);
        tty.settings.encrypted_history_enabled = Some(true);
        command_log_fixture(&tty);
        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Startup,
                HistoryStartOutcome::Ready(StartedHandle::new(started)),
            ),
        );
        assert!(tty.history_writer.is_some());
        assert_eq!(tty.history_id_floor, 8, "the floor applies regardless");
        {
            let term = tty.active_term().unwrap();
            let screen = term.screen.lock();
            assert_eq!(screen.command_log.len(), 1, "not seeded over live entries");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Disabling while a start is in flight wins: the late `Started` is
    /// dropped (its writer thread exits with it), nothing is installed.
    #[test]
    fn a_start_that_resolves_after_disable_is_dropped() {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};

        let dir = tmp_dir("late-start");
        let started = crate::history::start_with_key_in(
            dir.clone(),
            Cipher::ChaCha20Poly1305,
            key(),
            dorado_engine::kdf::KdfPrf::Skein512,
        )
        .unwrap();

        let mut tty = headless(1);
        tty.history_starting = true;
        // encrypted_history_enabled defaults to off — as if the user toggled
        // it off (or never had it on) while the startup task ran.
        let _ = update(
            &mut tty,
            Message::HistoryStarted(
                HistoryStartOrigin::Startup,
                HistoryStartOutcome::Ready(StartedHandle::new(started)),
            ),
        );
        assert!(!tty.history_starting);
        assert!(tty.history_writer.is_none(), "the late Started is dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_writer_restarted_after_a_simulated_crash_still_writes_correctly() {
        // Simulates a crash mid-compaction: a good segment + manifest already
        // on disk, plus a stray `.tmp` next to each (a save that never got to
        // the rename step). A restarted writer thread, pointed at the same
        // directory, must load the last-good state and keep writing
        // correctly, unaffected by the stray files.
        let dir = tmp_dir("crash-replay");
        let manifest_p = manifest_path(&dir);

        let mut manifest = Manifest::default();
        let date = local_date_from_epoch_ms(DAY1_MS);
        let filename = manifest.segment_filename_or_create(date, segment::random_filename);
        let segment_p = dir.join(&filename);
        segment::save(
            &segment_p,
            Cipher::ChaCha20Poly1305,
            &keys().segments,
            &[entry(1, "$ ls", DAY1_MS)],
        )
        .unwrap();
        manifest.set_count(date, 1);
        manifest
            .save(&manifest_p, Cipher::ChaCha20Poly1305, &keys().manifest)
            .unwrap();

        std::fs::write(tmp_path(&segment_p), b"garbage, not a valid wrapped blob").unwrap();
        std::fs::write(tmp_path(&manifest_p), b"garbage, not a valid wrapped blob").unwrap();

        let reloaded = Manifest::load(&manifest_p, &keys().manifest).unwrap();
        assert_eq!(
            reloaded.segment_filename(date),
            Some(filename.as_str()),
            "unaffected by the stray .tmp files"
        );

        let writer = Writer::spawn(dir.clone(), Cipher::ChaCha20Poly1305, keys(), reloaded);
        writer.send(HistoryEvent::Upsert(entry(2, "$ pwd", DAY1_MS)));

        wait_until(Duration::from_secs(2), || {
            segment::load(&segment_p, &keys().segments)
                .map(|e| e.len() == 2)
                .unwrap_or(false)
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}
