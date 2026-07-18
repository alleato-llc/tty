//! Headless view snapshot (dev/test only) — the tty counterpart of fed's.
//!
//! Renders the tabbed terminal chrome (rime tab strip + `phosphor` terminal widget +
//! status bar) to a PNG under `tty/snapshots/`. Excluded from the default run
//! (backend-specific baselines) via nextest's `default-filter`; run it with
//! `cargo nextest run --ignore-default-filter -E 'test(snapshot)'`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::Mutex;

use iced::Font;

use cathode::parser::TermParser;
use cathode::screen::TerminalScreen;

use crate::message::Message;
use crate::state::{MenuKind, Tab, Term, Tty, DEFAULT_FONT_SIZE};
use crate::theme::Theme;
use crate::view::root_view;

/// A tab wrapping a screen pre-painted by feeding `bytes` through the parser (no shell).
fn painted_term(title: &str, cols: usize, rows: usize, bytes: &[u8]) -> Term {
    let mut screen = TerminalScreen::new(cols, rows);
    TermParser::new().process(bytes, &mut screen);
    Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: title.into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    }
}

/// Two tabs (so the tab strip shows) over a screen exercising 16-color, bold,
/// underline, inverse, 256-color and truecolor — the full attribute matrix.
fn populated() -> Tty {
    let tab = painted_term(
        "zsh",
        56,
        6,
        b"\x1b[1;32muser@host\x1b[0m:\x1b[34m~/dev\x1b[0m$ ls --color\r\n\
          \x1b[1;34msrc\x1b[0m  \x1b[32mREADME.md\x1b[0m  \x1b[1;31mtarget\x1b[0m  Cargo.toml\r\n\
          \x1b[33mwarn\x1b[0m: \x1b[4munused\x1b[0m  \x1b[7minverse\x1b[0m  \x1b[38;5;208m256-orange\x1b[0m\r\n\
          \x1b[38;2;120;200;255mtruecolor\x1b[0m  \x1b[31mred \x1b[32mgreen \x1b[34mblue\x1b[0m\r\n\
          $ ",
    );
    let second = painted_term("zsh", 56, 6, b"$ ");
    Tty {
        tabs: vec![Tab::new(tab), Tab::new(second)],
        active: 0,
        theme: Theme::default(),
        font: Font::MONOSPACE,
        font_size: DEFAULT_FONT_SIZE,
        modifiers: iced::keyboard::Modifiers::default(),
        window_height: 600.0,
        window_width: 0.0,
        hovered_tab: None,
        selection: None,
        search: None,
        search_match: 0,
        scroll_target: None,
        show_env: false,
        env_vars: Vec::new(),
        env_source: crate::state::EnvSource::None,
        env_os_cache: None,
        env_filter: String::new(),
        env_reveal: false,
        env_expanded: false,
        env_add_open: false,
        env_pos: None,
        env_size: (620.0, 400.0),
        env_move_drag: None,
        env_resize: None,
        env_overlay_name: String::new(),
        env_overlay_value: String::new(),
        env_set_name: String::new(),
        env_set_value: String::new(),
        show_scrollback: false,
        scrollback_query: String::new(),
        scrollback_selected: None,
        scrollback_scroll: 0.0,
        scrollback_expanded: std::collections::HashSet::new(),
        // The status bar now auto-hides by default; pin it visible for the
        // general chrome fixtures so they keep exercising it (the dedicated
        // status-bar tests override this to exercise auto-hide).
        settings: crate::settings::Settings {
            status_bar_autohide: Some(false),
            ..Default::default()
        },
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
        clock_override: None,
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

/// Render the main (tabbed) chrome of `tty` for a snapshot — the daemon's `root_view`
/// keyed on the main window.
fn main_chrome(tty: &Tty) -> iced::Element<'_, crate::message::Message> {
    root_view(tty, tty.main_window.expect("populated sets a main window"))
}

#[test]
fn terminal_view() {
    let tty = populated();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-terminal.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-terminal` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn failed_command_marker_view() {
    // Two OSC 133 commands: one succeeds (exit 0), one fails (exit 1). The failed
    // command's prompt line gets the red wash + left bar; the successful one doesn't.
    let mut tty = populated();
    let term = painted_term(
        "zsh",
        56,
        8,
        b"\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07README.md  src\r\n\x1b]133;D;0\x07\
          \x1b]133;A\x07$ cargo test\r\n\x1b]133;C\x07error: test failed\r\n\x1b]133;D;1\x07\
          \x1b]133;A\x07$ ",
    );
    tty.tabs[0] = Tab::new(term);
    tty.active = 0;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-failed-command.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-failed-command` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn env_status_bar_cell_view() {
    // The "Env" launcher cell in the status bar — a text cell (no sampler); clicking it
    // opens the Env popover. Bar pinned visible so the cell renders.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("env", "sparkline")];
    tty.settings.status_bar_autohide = Some(false);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-env-status-cell.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-env-status-cell` changed — delete its PNG to re-baseline"
    );
}

/// A representative environment for the Env-view snapshots.
fn sample_env_vars() -> Vec<crate::env::EnvVar> {
    [
        ("EDITOR", "nvim"),
        ("HOME", "/Users/dev"),
        ("LANG", "en_US.UTF-8"),
        ("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"),
        ("SHELL", "/bin/zsh"),
        ("TERM", "xterm-256color"),
    ]
    .into_iter()
    .map(|(name, value)| crate::env::EnvVar {
        name: name.into(),
        value: value.into(),
    })
    .collect()
}

#[test]
fn env_view_compact_view() {
    // How the Env view opens by default: compact — a masked list plus the Add button,
    // no filter / reveal / summary. Editing is on, so the Add button shows.
    let mut tty = populated();
    tty.show_env = true;
    tty.env_size = (320.0, 340.0); // the compact default width
    tty.env_source = crate::state::EnvSource::Process;
    tty.settings.shell_integration.env_editing = Some(true);
    tty.env_vars = sample_env_vars();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-env-compact.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-env-compact` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn env_view_compact_scroll_view() {
    // A long environment in compact: the card caps at its max height and the list
    // scrolls, rather than the card growing to fit every variable.
    let mut tty = populated();
    tty.show_env = true;
    tty.env_size = (320.0, 340.0);
    tty.env_source = crate::state::EnvSource::Process;
    tty.settings.shell_integration.env_editing = Some(true);
    tty.env_vars = (0..24)
        .map(|i| crate::env::EnvVar {
            name: format!("VAR_{i:02}"),
            value: format!("value-{i}"),
        })
        .collect();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-env-compact-scroll.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-env-compact-scroll` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn env_view_view() {
    // The expanded Env view: filter, revealed values, the launch-time source note, and
    // the Add button.
    let mut tty = populated();
    tty.show_env = true;
    tty.env_expanded = true;
    tty.env_reveal = true;
    tty.env_source = crate::state::EnvSource::Process;
    tty.settings.shell_integration.env_editing = Some(true);
    tty.env_vars = sample_env_vars();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-env-view.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-env-view` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn env_add_modal_view() {
    // The "Set a variable" modal, opened by the Add button, over the expanded view.
    let mut tty = populated();
    tty.show_env = true;
    tty.env_expanded = true;
    tty.env_source = crate::state::EnvSource::Process;
    tty.settings.shell_integration.env_editing = Some(true);
    tty.env_vars = sample_env_vars();
    tty.env_add_open = true;
    tty.env_set_name = "API_TOKEN".into();
    tty.env_set_value = "s3cr3t".into();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-env-add-modal.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-env-add-modal` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn prompt_gutter_view() {
    // Same commands as the failed-marker test, but with the prompt gutter on: every
    // prompt gets a dot to its left (red for the failed one), and the grid shifts right.
    let mut tty = populated();
    tty.settings.shell_integration.gutter = Some(true);
    let term = painted_term(
        "zsh",
        56,
        8,
        b"\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07README.md  src\r\n\x1b]133;D;0\x07\
          \x1b]133;A\x07$ cargo test\r\n\x1b]133;C\x07error: test failed\r\n\x1b]133;D;1\x07\
          \x1b]133;A\x07$ ",
    );
    tty.tabs[0] = Tab::new(term);
    tty.active = 0;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-prompt-gutter.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-prompt-gutter` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn split_pane_view() {
    use iced::widget::pane_grid::Direction;
    // Split the active tab into two side-by-side panes (the new one, on the right, takes
    // focus and gets the accent border).
    let mut tty = populated();
    let win = tty.main_window.unwrap();
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(painted_term(
            "zsh",
            28,
            6,
            b"$ cargo test\r\n\x1b[32m   Compiling\x1b[0m tty\r\n$ ",
        )),
    );
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-split.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-split` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_pane_view() {
    use iced::widget::pane_grid::Direction;
    // A Processes drill-in "graduated" into a real split pane beside the terminal:
    // a header (name + maximize/close) over the live table, resizable like any pane.
    let mut tty = populated();
    let win = tty.main_window.unwrap();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Metric(crate::settings::MetricKind::Procs),
    );
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-pane.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-pane` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn pane_replace_pick_view() {
    // "Replace a pane" pick mode: the grid dims under a scrim with an instruction
    // pill; a pane click (falling through the scrim) replaces it.
    let mut tty = populated();
    tty.pane_replace_pending = Some(crate::settings::MetricKind::Cpu);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-pane-replace-pick.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-pane-replace-pick` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn pane_replace_confirm_view() {
    // Confirming before replacing a live terminal pane with a metric view.
    let mut tty = populated();
    let win = tty.main_window.unwrap();
    let pane = tty.tabs[tty.active].focus;
    tty.pane_replace_confirm = Some((win, pane, crate::settings::MetricKind::Cpu));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-pane-replace-confirm.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-pane-replace-confirm` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn force_kill_confirm_view() {
    // "Force Quit…" from the Processes drill-in confirms before the SIGKILL.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Procs,
    )];
    tty.kill_confirm = Some((412, "Google Chrome".to_string()));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-force-kill-confirm.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-force-kill-confirm` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn rename_bar_view() {
    // "Rename tab" opens a focused, prefilled field under the tab strip.
    let mut tty = populated();
    tty.renaming = Some((0, "deploy".to_string()));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-rename.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-rename` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn detached_window_view() {
    // A tab torn off into its own window: just its terminal, a slim strip with the
    // Reattach button, and a status bar — no tab strip.
    let mut tty = populated();
    let tab = tty.tabs.remove(0);
    tty.active = 0;
    let win = iced::window::Id::unique();
    tty.detached.insert(win, tab);
    tty.detach_origin.insert(win, 0);
    tty.focused_window = Some(win);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(root_view(&tty, win));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-detached.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-detached` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn scrollback_panel_view() {
    // A 6-row screen fed a shell session with a `mark_command_boundary` right before
    // each command's Enter (mirroring what `update::handle_key` does live), so
    // `command_log` ends up with 5 real command/output entries — exercising the
    // accordion table's header rows, an expanded command's output rows, zebra
    // striping, and a selected/highlighted output row.
    let mut screen = TerminalScreen::new(56, 6);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml  src  target\r\n", &mut screen);

    parser.process(b"$ cargo build", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(
        b"\r\n\x1b[32m   Compiling\x1b[0m tty v0.1.0\r\n\x1b[32m    Finished\x1b[0m dev profile\r\n",
        &mut screen,
    );

    parser.process(b"$ cargo test", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(
        b"\r\nrunning 10 tests\r\ntest result: \x1b[32mok\x1b[0m. 10 passed\r\n",
        &mut screen,
    );

    parser.process(b"$ git status", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(
        b"\r\nOn branch main\r\nnothing to commit, working tree clean\r\n",
        &mut screen,
    );

    parser.process(b"$ git log --oneline -3", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(
        b"\r\na1b2c3d fix: scrollback history table\r\ne4f5a6b feat: max scrollback setting\r\n\
          9c8d7e6 docs: update keybindings\r\n",
        &mut screen,
    );

    parser.process(b"$ ", &mut screen);

    let tab = Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: "zsh".into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    };
    let mut tty = Tty {
        tabs: vec![Tab::new(tab)],
        ..populated()
    };
    tty.show_scrollback = true;
    // Rows: 0=Header(ls), 1=Header(cargo build, expanded) -> 2,3=its output,
    // 4=Header(cargo test), 5=Header(git status), 6=Header(git log).
    tty.scrollback_expanded.insert(1);
    tty.scrollback_selected = Some(3);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-scrollback-history.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-scrollback-history` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn scrollback_cleared_command_row_view() {
    // What a command row looks like after "Clear" (blanks the command's own text
    // and its output, per `TerminalScreen::clear_command_output`) — a real render
    // check that a blank command text doesn't render as something broken/misaligned.
    let mut screen = TerminalScreen::new(56, 6);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml  src  target\r\n", &mut screen);
    parser.process(b"$ cd /tmp", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\n", &mut screen);
    parser.process(b"$ ", &mut screen);
    screen.clear_command_output(0);

    let tab = Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: "zsh".into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    };
    let mut tty = Tty {
        tabs: vec![Tab::new(tab)],
        ..populated()
    };
    tty.show_scrollback = true;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-scrollback-cleared-row.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-scrollback-cleared-row` changed — delete its PNG to re-baseline"
    );
}

/// A `Tty` with the scrollback panel open over a single recorded `$ ls` command —
/// the shared fixture for the real-widget-tree right-click dispatch tests below.
fn tty_with_open_scrollback_panel() -> Tty {
    let mut screen = TerminalScreen::new(56, 6);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml  src  target\r\n", &mut screen);
    parser.process(b"$ ", &mut screen);

    let tab = Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: "zsh".into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    };
    let mut tty = Tty {
        tabs: vec![Tab::new(tab)],
        ..populated()
    };
    tty.show_scrollback = true;
    tty
}

/// Assert `events`, simulated at the "$ ls" header row, dispatch a
/// `ScrollbackRowRightClick` for that row through the real view tree — table →
/// rime's `modal` → chrome — proving the hit-test/event-capture chain
/// (`opaque`/`Stack`/`mouse_area` nesting) actually delivers the press instead of
/// it being swallowed or misrouted to something else (e.g. the pane underneath).
fn assert_dispatches_scrollback_right_click(events: Vec<iced::Event>) {
    let tty = tty_with_open_scrollback_panel();
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let _ = sim.snapshot(&crate::state::theme(&tty)); // force a layout pass
    sim.point_at(iced::Point::new(300.0, 273.0)); // over the "$ ls" header row
    let statuses = sim.simulate(events);
    assert_eq!(
        statuses[statuses.len() - 2], // the ButtonPressed, one before the trailing release
        iced::event::Status::Captured,
        "the table (or something above it) must claim the press: {statuses:?}"
    );
    let messages: Vec<_> = sim.into_messages().collect();
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::ScrollbackRowRightClick(0, crate::state::HistoryRowTarget::Live(crate::state::ScrollbackTarget::Command { log_index: 0, text }))
                if text == "$ ls"
        )),
        "expected a ScrollbackRowRightClick for the header row, got {messages:?}"
    );
}

#[test]
fn scrollback_row_right_click_dispatches_through_the_real_widget_tree() {
    assert_dispatches_scrollback_right_click(vec![
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
            iced::mouse::Button::Right,
        )),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
            iced::mouse::Button::Right,
        )),
    ]);
}

#[test]
fn scrollback_row_ctrl_click_dispatches_through_the_real_widget_tree() {
    // macOS's secondary-click convention (Ctrl held on a *left* press) must resolve
    // to the same right-click menu as a real right button — this is the exact bug
    // reported live: a Ctrl+trackpad-click on a header row toggled its expand state
    // (the left-click behavior) instead of opening the copy/clear menu.
    assert_dispatches_scrollback_right_click(vec![
        iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
            iced::keyboard::Modifiers::CTRL,
        )),
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
            iced::mouse::Button::Left,
        )),
    ]);
}

#[test]
fn scrollback_output_row_context_menu_view() {
    // Right-clicking an output row in the scrollback panel opens its copy/clear menu
    // (no "Delete" — there's no row concept to remove for a single line), anchored
    // over the panel — same fixture as `scrollback_panel_view`.
    let mut screen = TerminalScreen::new(56, 6);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml  src  target\r\n", &mut screen);
    parser.process(b"$ ", &mut screen);

    let tab = Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: "zsh".into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    };
    let mut tty = Tty {
        tabs: vec![Tab::new(tab)],
        ..populated()
    };
    tty.show_scrollback = true;
    let at = iced::Point::new(220.0, 160.0);
    tty.pointer = at;
    tty.scrollback_selected = Some(1);
    tty.menu = Some((
        MenuKind::ScrollbackRow(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Output {
                log_index: 0,
                line: 0,
                text: "Cargo.toml  src  target".to_string(),
            },
        )),
        at,
    ));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-scrollback-row-menu.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-scrollback-row-menu` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn scrollback_command_row_context_menu_view() {
    // Right-clicking a command's header row gets a "Delete" item too (below a
    // separator), unlike an output row's menu — same fixture as
    // `scrollback_output_row_context_menu_view`, just targeting the header.
    let mut screen = TerminalScreen::new(56, 6);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\nCargo.toml  src  target\r\n", &mut screen);
    parser.process(b"$ ", &mut screen);

    let tab = Term {
        screen: Arc::new(Mutex::new(screen)),
        pty: None,
        title: "zsh".into(),
        alive: Arc::new(AtomicBool::new(true)),
        dirty: Arc::new(AtomicBool::new(false)),
        activity: false,
        env_file: None,
    };
    let mut tty = Tty {
        tabs: vec![Tab::new(tab)],
        ..populated()
    };
    tty.show_scrollback = true;
    let at = iced::Point::new(220.0, 160.0);
    tty.pointer = at;
    tty.scrollback_selected = Some(0);
    tty.menu = Some((
        MenuKind::ScrollbackRow(crate::state::HistoryRowTarget::Live(
            crate::state::ScrollbackTarget::Command {
                log_index: 0,
                text: "$ ls".to_string(),
            },
        )),
        at,
    ));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-scrollback-command-row-menu.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-scrollback-command-row-menu` changed — delete its PNG to re-baseline"
    );
}

/// A `Tty` sitting in Settings → History with the drill-in archive browser
/// open over a handful of archived commands spanning two days. `age_from_*`
/// renders relative to now, so pin the timestamps off `Utc::now()` to keep the
/// ages sensible (5m / 2h / yesterday) whenever the snapshot is regenerated.
fn tty_in_archive_browser() -> (Tty, Vec<cathode::history::PersistedCommandEntry>) {
    // A FIXED anchor (not `now()`), pinned into the view via `clock_override` below, so
    // the "N ago" + archived-date columns render identically no matter when the test
    // runs — previously they drifted, and crossing midnight flipped the dates. Midday
    // UTC keeps the local date clear of a day boundary.
    let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T18:00:00Z")
        .expect("valid anchor")
        .timestamp_millis() as u64;
    let min = 60_000u64;
    let hr = 60 * min;
    let day = 24 * hr;
    let entries = vec![
        cathode::history::PersistedCommandEntry {
            id: 12,
            command: "$ cargo nextest run -p tty".into(),
            started_at_epoch_ms: now - 4 * min,
            pane_tag: "zsh".into(),
        },
        cathode::history::PersistedCommandEntry {
            id: 11,
            command: "$ git push origin main".into(),
            started_at_epoch_ms: now - 2 * hr,
            pane_tag: "zsh".into(),
        },
        cathode::history::PersistedCommandEntry {
            id: 9,
            command: "$ docker compose up -d".into(),
            started_at_epoch_ms: now - 5 * hr,
            pane_tag: "build".into(),
        },
        cathode::history::PersistedCommandEntry {
            id: 4,
            command: "$ rg --hidden TODO src/".into(),
            started_at_epoch_ms: now - day - 3 * hr,
            pane_tag: "zsh".into(),
        },
        cathode::history::PersistedCommandEntry {
            id: 2,
            command: "$ ssh deploy@edge-01 systemctl restart tty".into(),
            started_at_epoch_ms: now - day - 6 * hr,
            pane_tag: "ops".into(),
        },
    ];
    let cursor =
        crate::history::local_date_from_epoch_ms(entries.last().unwrap().started_at_epoch_ms);
    let tty = Tty {
        tabs: vec![Tab::new(painted_term("zsh", 56, 6, b"$ "))],
        show_settings: true,
        settings_section: 4,
        show_settings_history: true,
        settings_history: entries.clone(),
        settings_history_cursor: Some(cursor),
        settings_history_selected: Some(1),
        clock_override: Some(now),
        ..populated()
    };
    (tty, entries)
}

#[test]
fn settings_history_browser_view() {
    // Settings → History, drilled into the archive browser: the "‹ Back" /
    // "Archived Commands" / "Load older day" header, the "archived back to
    // <date>" caption, and the monospace command table with one row selected.
    let (tty, _) = tty_in_archive_browser();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-browser.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-browser` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_history_row_context_menu_view() {
    // Right-clicking an archived row opens its Copy / Delete… menu (the
    // per-row actions the archive browser adds over the read-only list).
    let (mut tty, entries) = tty_in_archive_browser();
    let e = &entries[1];
    let target = crate::state::ArchivedTarget {
        date: crate::history::local_date_from_epoch_ms(e.started_at_epoch_ms),
        id: e.id,
        started_at_epoch_ms: e.started_at_epoch_ms,
        pane_tag: e.pane_tag.clone(),
        command: e.command.clone(),
    };
    let at = iced::Point::new(300.0, 210.0);
    tty.pointer = at;
    tty.menu = Some((MenuKind::SettingsHistoryRow(target), at));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-row-menu.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-row-menu` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_appearance_theme_pane_view() {
    // Settings → Appearance, landing on the "Theme" sub-tab: the horizontal
    // sub-tab strip (Theme active, the rest muted) over just that pane's controls
    // (theme / font / font size), instead of the whole section as one scroll.
    let mut tty = populated();
    tty.show_settings = true;
    tty.settings_section = 0;
    tty.appearance_tab = 0;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-appearance-theme.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-appearance-theme` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_shell_section_view() {
    // The top-level Shell section: the OSC 133 master toggle with its sub-options
    // (moved out of Appearance → Terminal into its own section).
    let mut tty = populated();
    tty.settings.shell_integration.gutter = Some(true);
    tty.show_settings = true;
    tty.settings_section = 5;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-shell.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-shell` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_appearance_statusbar_pane_view() {
    // The Appearance section switched to the "Status bar" sub-tab: just the bar's
    // own chrome (disable + auto-hide). The machine-stat cells live in the Metrics
    // section now (see `metrics_section_view`).
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    tty.show_settings = true;
    tty.settings_section = 0;
    tty.appearance_tab = 2;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-appearance-statusbar.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-appearance-statusbar` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metrics_section_view() {
    // The new top-level Metrics section (above History): pin-popovers, the
    // graduate-into-a-pane toggle, reorder hold, and the machine-stat cell editor.
    let mut tty = populated();
    tty.settings.status_bar_metrics =
        vec![metric("cpu", "sparkline"), metric("procs", "sparkline")];
    // Hide the live bar so only the deterministic settings panel renders.
    tty.settings.status_bar_autohide = Some(true);
    tty.pointer = iced::Point::new(300.0, 40.0);
    tty.show_settings = true;
    tty.settings_section = 3;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-metrics.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-metrics` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_appearance_clock_format_view() {
    // With a clock cell configured, the Metrics section grows a "Clock format"
    // block (24-hour / seconds / date). Snapshots the settings, not the live
    // time, so it stays deterministic.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("clock", "sparkline")];
    tty.settings.clock_24h = Some(true);
    // Hide the bar (auto-hide on, pointer at top) so the live clock cell doesn't
    // render into the snapshot — only the deterministic settings panel does.
    tty.settings.status_bar_autohide = Some(true);
    tty.pointer = iced::Point::new(300.0, 40.0);
    tty.show_settings = true;
    tty.settings_section = 3;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-clock-format.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-clock-format` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_appearance_terminal_pane_view() {
    // The "Terminal" sub-tab: scrollback depth, per-command output caps, and the
    // ⌘-click "open file" command (shown here with a VS Code template set).
    let mut tty = populated();
    tty.settings.open_file_command = Some("code -g {file}:{line}:{col}".into());
    tty.show_settings = true;
    tty.settings_section = 0;
    tty.appearance_tab = 3;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-appearance-terminal.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-appearance-terminal` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_appearance_window_pane_view() {
    // The "Window" sub-tab: keep-on-top toggle plus the two transparency
    // sliders — "When Active" (0–50%) and "On Blur" (0–95%).
    let mut tty = populated();
    tty.show_settings = true;
    tty.settings_section = 0;
    tty.appearance_tab = 4;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-appearance-window.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-appearance-window` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn tab_context_menu_view() {
    // Right-clicking a tab opens its menu (new tab / split / close tab).
    let mut tty = populated();
    let at = iced::Point::new(64.0, 44.0);
    tty.pointer = at;
    tty.menu = Some((MenuKind::Tab, at));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-tab-menu.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-tab-menu` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_machine_stats_view() {
    // Machine stats enabled with a landed sample: the status bar leads with
    // `CPU nn% · MEM used/total` before the grid/tab/font cluster. (populated()
    // pins auto-hide off, so the bar is visible.)
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    tty.metrics.latest = Some(crate::metrics::MachineStats {
        cpu_percent: 72.0,
        mem_used: 41_000_000_000,
        mem_total: 128 * 1024 * 1024 * 1024,
        ..Default::default()
    });
    // A recent CPU history that ramps up (so the sparkline shows a curve and the
    // color grades into the caution band at 72%), plus a steady memory line.
    tty.metrics.cpu_history = [
        8.0, 12.0, 10.0, 18.0, 30.0, 26.0, 40.0, 55.0, 48.0, 60.0, 68.0, 72.0,
    ]
    .into_iter()
    .collect();
    tty.metrics.mem_history = [30.0, 31.0, 30.0, 32.0, 33.0, 32.0, 31.0, 32.0]
        .into_iter()
        .collect();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-machine-stats.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-machine-stats` changed — delete its PNG to re-baseline"
    );
}

/// Give `tty` a landed sample with a ramping CPU history and a steady memory
/// line, plus network/disk rate series, so the metric cells render with real
/// curves (shared by the config-list snapshots below).
fn seed_metric_sample(tty: &mut Tty) {
    const M: f32 = 1024.0 * 1024.0;
    tty.metrics.latest = Some(crate::metrics::MachineStats {
        cpu_percent: 72.0,
        mem_used: 41_000_000_000,
        mem_total: 128 * 1024 * 1024 * 1024,
        swap_used: 1_288_490_188, // ~1.2G
        swap_total: 8 * 1024 * 1024 * 1024,
        net_rx_bps: 3.2 * M,
        net_tx_bps: 512.0 * 1024.0,
        disk_r_bps: 8.0 * M,
        disk_w_bps: 1.0 * M,
    });
    tty.metrics.cpu_history = [
        8.0, 12.0, 10.0, 18.0, 30.0, 26.0, 40.0, 55.0, 48.0, 60.0, 68.0, 72.0,
    ]
    .into_iter()
    .collect();
    tty.metrics.mem_history = [30.0, 31.0, 30.0, 32.0, 33.0, 32.0, 31.0, 32.0]
        .into_iter()
        .collect();
    tty.metrics.net_rx_history = [0.4 * M, 1.1 * M, 0.8 * M, 2.0 * M, 2.6 * M, 3.2 * M]
        .into_iter()
        .collect();
    tty.metrics.net_tx_history = [
        80.0 * 1024.0,
        120.0 * 1024.0,
        300.0 * 1024.0,
        512.0 * 1024.0,
    ]
    .into_iter()
    .collect();
    tty.metrics.disk_r_history = [1.0 * M, 3.0 * M, 2.0 * M, 6.0 * M, 7.5 * M, 8.0 * M]
        .into_iter()
        .collect();
    tty.metrics.disk_w_history = [0.2 * M, 0.5 * M, 0.3 * M, 1.0 * M].into_iter().collect();
}

fn metric(kind: &str, style: &str) -> crate::settings::MetricConfig {
    crate::settings::MetricConfig {
        metric: kind.to_string(),
        style: style.to_string(),
        warn: None,
        alarm: None,
    }
}

#[test]
fn status_bar_metric_styles_view() {
    // A mixed config: CPU as a sparkline, memory as the plain `number` style
    // (label only, no canvas), proving the per-metric style selection renders.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "number")];
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-metric-styles.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-metric-styles` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_metrics_shed_when_narrow_view() {
    // Two metrics configured, but a narrow tracked window width: the rightmost
    // cell (memory) is shed before the bar overflows, so only CPU shows.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    tty.window_width = 400.0;
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-metrics-shed.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-metrics-shed` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_rate_metrics_view() {
    // The network/disk throughput metrics: rate sparklines (neutral accent,
    // auto-scaled to their own peak) with formatted byte-rate labels, in the
    // configured order.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![
        metric("net_rx", "sparkline"),
        metric("net_tx", "sparkline"),
        metric("disk_w", "sparkline"),
    ];
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-rate-metrics.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-rate-metrics` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_net_io_view() {
    // The combined network I/O metric: rx (accent) and tx (warn) overlaid on a
    // single sparkline, with both rates in the label.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("net_io", "sparkline")];
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-net-io.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-net-io` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_disk_io_view() {
    // The combined disk I/O metric: read (accent) and write (warn) overlaid on a
    // single sparkline, with both rates in the label.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("disk_io", "sparkline")];
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-disk-io.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-disk-io` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_popover_view() {
    // Clicking the disk I/O sparkline drills in: the bottom-centered popover
    // card shows the full-size line chart (read accent + write warn on a shared
    // scale, peak byte-rate on the y axis), the current readout, the two-line
    // legend, and the sample-count caption, floating over the status bar.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("disk_io", "sparkline")];
    seed_metric_sample(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::DiskIo,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-popover.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-popover` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_popover_empty_view() {
    // Drilling into a metric whose history isn't chartable yet (here disk read,
    // with a sample landed but no rate history — as on a platform without the
    // sampler): the popover still shows a card, with the "collecting" note in
    // place of a blank chart, so click-away/Escape have a target.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("disk_r", "sparkline")];
    tty.metrics.latest = Some(crate::metrics::MachineStats::default());
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::DiskR,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-popover-empty.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-popover-empty` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_memory_view() {
    // Drilling into memory: the chart is a fixed 0..100% gauge, so a ~32%-used
    // line sits about a third up the plot rather than filling it. Guards the
    // bounded-metric scaling (a peak-scaled axis would push any steady line to
    // the top, misreading a third of RAM as all of it).
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("mem", "sparkline")];
    seed_metric_sample(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Mem,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-memory.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-memory` changed — delete its PNG to re-baseline"
    );
}

/// Seed a 16-core machine (4 Efficiency + 12 Performance, matching this
/// project's dev hardware) with a per-core history and perf levels, so the CPU
/// per-core / combined drill-ins render a full grid.
fn seed_cpu_cores(tty: &mut Tty) {
    use prexp_core::system::CpuKind;
    let currents: [f32; 16] = [
        12.0, 8.0, 20.0, 5.0, // E cores
        72.0, 95.0, 40.0, 18.0, 60.0, 33.0, 88.0, 27.0, 55.0, 10.0, 70.0, 45.0, // P cores
    ];
    tty.metrics.core_history = currents
        .iter()
        .map(|&cur| {
            let mut d: std::collections::VecDeque<f32> = (0..8)
                .map(|k| (cur * (0.4 + 0.07 * k as f32)).min(100.0))
                .collect();
            *d.back_mut().unwrap() = cur;
            d
        })
        .collect();
    tty.metrics.perf_levels = Some(
        (0..16)
            .map(|i| {
                if i < 4 {
                    CpuKind::Efficiency
                } else {
                    CpuKind::Performance
                }
            })
            .collect(),
    );
}

#[test]
fn status_bar_uptime_cells_view() {
    // Uptime and Session as status-bar cells: text (not sparklines), showing the
    // abbreviated form ("up 3d 4h" / "up 2h 15m").
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![
        metric("uptime", "sparkline"),
        metric("session", "sparkline"),
    ];
    seed_metric_sample(&mut tty);
    tty.metrics.system_uptime_secs = Some(3 * 86_400 + 4 * 3600 + 12 * 60);
    tty.metrics.session_uptime_secs = Some(2 * 3600 + 15 * 60);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-uptime.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-uptime` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_uptime_view() {
    // Clicking the uptime cell drills into the full breakdown ("3 days, 4 hours,
    // 12 minutes") under the metric name and a note on what it counts from.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("uptime", "sparkline")];
    seed_metric_sample(&mut tty);
    tty.metrics.system_uptime_secs = Some(3 * 86_400 + 4 * 3600 + 12 * 60);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Uptime,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-uptime.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-uptime` changed — delete its PNG to re-baseline"
    );
}

/// Seed a deterministic load average + 1-minute history for the load snapshots.
fn seed_load(tty: &mut Tty) {
    tty.metrics.load_avg = Some([1.23, 0.95, 0.80]);
    tty.metrics.load1_history = [0.6, 0.8, 0.7, 1.0, 1.1, 0.9, 1.3, 1.23]
        .into_iter()
        .collect();
}

#[test]
fn status_bar_edit_mode_view() {
    // Live edit mode: each metric cell gets an accent outline (draggable), and the
    // right end shows the "drag to reorder · Esc to finish" hint.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    seed_metric_sample(&mut tty);
    tty.status_bar_edit = true;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-edit.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-edit` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_edit_dragging_view() {
    // Mid-drag in edit mode: the dragged cell (CPU) is filled/"lifted", and an
    // accent insertion bar shows where it would drop (before Memory).
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![
        metric("cpu", "sparkline"),
        metric("mem", "sparkline"),
        metric("net_io", "sparkline"),
    ];
    seed_metric_sample(&mut tty);
    tty.status_bar_edit = true;
    tty.status_metric_drag = Some(0);
    tty.status_metric_drop = Some(1);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-edit-dragging.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-edit-dragging` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_scrolled_view() {
    // A narrow bar with more metrics than fit, scrolled to the middle of the list:
    // one windowed cell (memory) flanked by ‹ and › chevrons that say there are
    // more metrics off each edge.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![
        metric("cpu", "sparkline"),
        metric("mem", "sparkline"),
        metric("net_io", "sparkline"),
    ];
    tty.window_width = 400.0;
    tty.status_bar_scroll = 1;
    seed_metric_sample(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-scrolled.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-scrolled` changed — delete its PNG to re-baseline"
    );
}

/// Seed a deterministic process list for the Processes snapshots.
fn seed_processes(tty: &mut Tty) {
    let mb = 1024 * 1024;
    let p = |pid, name: &str, cpu: f32, mem: u64| crate::metrics::ProcInfo {
        pid,
        name: name.to_string(),
        cpu_percent: cpu,
        memory_bytes: mem,
    };
    tty.metrics.processes = vec![
        p(412, "Google Chrome", 92.0, 4900 * mb),
        // 64% grades amber (>=60), 92% red (>=85) — exercises the CPU-hog coloring.
        p(88, "rustc", 64.0, 1900 * mb),
        // A long name, to exercise the fill-column truncation.
        p(310, "com.apple.WebKit.WebContent", 6.0, 820 * mb),
        p(1, "Terminal", 4.0, 240 * mb),
        p(233, "zsh", 1.0, 12 * mb),
        p(700, "Spotify", 8.0, 1400 * mb),
        p(9, "kernel_task", 12.0, 760 * mb),
    ];
}

#[test]
fn status_bar_procs_view() {
    // The Processes cell: the busiest process by CPU% (`↑ Google Chrome 92%`).
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-procs.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-procs` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_procs_view() {
    // The Processes drill-in: the clickable header (CPU active, ▾) over the
    // scrollable table, sorted by CPU% descending.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Procs,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-procs.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-procs` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_proc_one_view() {
    // Drilling into one process: a "‹ Back" control, the live CPU chart, the
    // memory / thread readout, and the scrollable list of open file descriptors.
    use prexp_core::models::{OpenResource, ResourceKind};
    let mb = 1024 * 1024;
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Procs,
    )];
    tty.proc_detail_pid = Some(412);
    let fd = |descriptor, kind, path: Option<&str>| OpenResource {
        descriptor,
        kind,
        path: path.map(str::to_string),
    };
    tty.metrics.proc_detail = Some(crate::metrics::ProcDetail::for_test(
        412,
        "Google Chrome",
        34,
        4900 * mb,
        [8.0, 14.0, 40.0, 62.0, 55.0, 71.0, 48.0, 33.0, 44.0, 58.0],
        vec![
            fd(0, ResourceKind::Device, Some("/dev/null")),
            fd(
                3,
                ResourceKind::File,
                Some("/Applications/Google Chrome.app"),
            ),
            fd(7, ResourceKind::Socket, Some("tcp4 → 142.250.72.174:443")),
            fd(9, ResourceKind::Pipe, None),
            fd(
                12,
                ResourceKind::File,
                Some("/Users/me/Library/Caches/Chrome/index"),
            ),
        ],
    ));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-proc-one.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-proc-one` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn proc_row_context_menu_view() {
    // Right-clicking a process row opens its context menu (View Process
    // + copy actions) at the pointer, over the process list.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("procs", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_processes(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Procs,
    )];
    tty.pointer = iced::Point::new(760.0, 360.0);
    tty.menu = Some((
        crate::state::MenuKind::ProcRow {
            pid: 412,
            name: "Google Chrome".to_string(),
        },
        tty.pointer,
    ));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-proc-row-menu.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-proc-row-menu` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_alert_view() {
    // A graded cell past its alarm threshold: CPU at 92% recolors both the
    // sparkline and the label (the louder alert), while memory stays calm.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    seed_metric_sample(&mut tty);
    if let Some(s) = tty.metrics.latest.as_mut() {
        s.cpu_percent = 92.0;
    }
    tty.metrics.cpu_history = [70.0, 78.0, 85.0, 88.0, 90.0, 91.0, 92.0, 92.0]
        .into_iter()
        .collect();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-alert.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-alert` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_load_view() {
    // The load cell: a sparkline of the 1-minute load beside its value.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("load", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_load(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-load.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-load` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_load_view() {
    // The load drill-in: the 1-minute load line chart over the full 1/5/15-minute
    // triple.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("load", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_load(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Load,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-load.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-load` changed — delete its PNG to re-baseline"
    );
}

/// Seed a deterministic charging battery for the battery snapshots.
fn seed_battery(tty: &mut Tty) {
    tty.metrics.battery = Some(prexp_core::system::BatteryInfo {
        percent: 82.0,
        charging: true,
        time_to_empty_min: -1,
        time_to_full_min: 45,
    });
    tty.metrics.battery_history = [70.0, 72.0, 74.0, 77.0, 79.0, 80.0, 81.0, 82.0]
        .into_iter()
        .collect();
}

#[test]
fn status_bar_battery_view() {
    // The battery cell: a 0..100% gauge sparkline beside `bat 82% ↑` (charging).
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("battery", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_battery(&mut tty);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-battery.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-battery` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_battery_view() {
    // The battery drill-in: the charge gauge over the charging state + estimate.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("battery", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_battery(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::Battery,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-battery.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-battery` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_cpu_all_view() {
    // The "CPU (all)" drill-in: the aggregate line chart *and* the per-core grid
    // stacked — a sparkline per logical core (color-graded by load, current %
    // below), grouped into Performance and Efficiency sections.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu_all", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_cpu_cores(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::CpuAll,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-cpu-all.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-cpu-all` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_cpu_cores_view() {
    // The standalone "CPU Cores" drill-in: the per-core grid alone (no aggregate
    // line chart — that is the separate "CPU" drill-in), under a compact readout.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu_cores", "sparkline")];
    seed_metric_sample(&mut tty);
    seed_cpu_cores(&mut tty);
    tty.metric_details = vec![crate::state::MetricPopover::new(
        crate::settings::MetricKind::CpuCores,
    )];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-cpu-cores.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-cpu-cores` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_popover_resized_view() {
    // A drag-resized compact popover: a per-popover `size` override makes the
    // card wider and its chart taller than the default.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("disk_io", "sparkline")];
    seed_metric_sample(&mut tty);
    let mut pop = crate::state::MetricPopover::new(crate::settings::MetricKind::DiskIo);
    pop.size = Some((480.0, 280.0));
    tty.metric_details = vec![pop];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-popover-resized.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-popover-resized` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_popover_expanded_view() {
    // The popover's "Expand" state: a large centered card whose line chart fills
    // most of the window (here Net I/O, two series), sized off the window
    // geometry. The chart carries a "Collapse" affordance top-right.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("net_io", "sparkline")];
    tty.window_width = 1100.0;
    tty.window_height = 800.0;
    seed_metric_sample(&mut tty);
    let mut pop = crate::state::MetricPopover::new(crate::settings::MetricKind::NetIo);
    pop.expanded = true;
    tty.metric_details = vec![pop];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(1100.0, 800.0),
        main_chrome(&tty),
    );
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-popover-expanded.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-popover-expanded` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn metric_detail_pinned_view() {
    // With popovers pinned, several stay open at once: two cards (memory + CPU),
    // cascaded up-and-right so they don't fully overlap, each carrying its own
    // "×" close button in the top-right control cluster.
    let mut tty = populated();
    tty.settings.status_bar_metrics = vec![metric("cpu", "sparkline"), metric("mem", "sparkline")];
    tty.settings.status_bar_metrics_pinned = Some(true);
    tty.window_width = 1024.0;
    tty.window_height = 768.0;
    seed_metric_sample(&mut tty);
    tty.metric_details = vec![
        crate::state::MetricPopover::new(crate::settings::MetricKind::Mem),
        crate::state::MetricPopover::new(crate::settings::MetricKind::Cpu),
    ];
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(1024.0, 768.0),
        main_chrome(&tty),
    );
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-metric-detail-pinned.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-metric-detail-pinned` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_autohidden_view() {
    // Auto-hide on, pointer up near the top: the status bar is gone and the
    // pane grid takes the full height (no reflow versus the revealed state).
    let mut tty = populated();
    tty.settings.status_bar_autohide = Some(true);
    tty.pointer = iced::Point::new(300.0, 40.0);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-autohidden.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-autohidden` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_disabled_view() {
    // The status bar turned off entirely: the pane grid takes the full height and
    // the bar never shows — not even with the pointer down in the reveal zone
    // (unlike auto-hide, which would float it in there).
    let mut tty = populated();
    tty.settings.status_bar_disabled = Some(true);
    tty.window_height = 768.0;
    tty.pointer = iced::Point::new(300.0, 760.0);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(1024.0, 768.0),
        main_chrome(&tty),
    );
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-disabled.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-disabled` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn status_bar_revealed_view() {
    // Auto-hide on, pointer down within the reveal zone: the status bar floats
    // back in over the bottom edge.
    let mut tty = populated();
    tty.settings.status_bar_autohide = Some(true);
    tty.pointer = iced::Point::new(300.0, tty.window_height - 8.0);
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-status-bar-revealed.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-status-bar-revealed` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn enable_history_dialog_with_fanout_knob_view() {
    // The "Enable encrypted history" dialog with the Passphrase key source
    // selected, so every fixed-at-enable choice shows: key source, KDF,
    // passphrase fields, cipher, and the new fan-out PRF knob (Auto/Skein/
    // BLAKE3) with its family-matched caption.
    use crate::state::{PassphrasePrompt, PassphrasePromptKind};
    let mut tty = populated();
    tty.show_settings = true;
    tty.settings_section = 4;
    tty.settings.encrypted_history_enabled = Some(false);
    tty.settings.history_key_source = Some("passphrase".to_string());
    tty.settings.history_fanout = Some("auto".to_string());
    tty.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Enable));
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-enable-history-fanout.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-enable-history-fanout` changed — delete its PNG to re-baseline"
    );
}

/// The History settings section, with the app's live status bar hidden so only
/// the deterministic panel renders (no wall-clock cell etc.).
fn history_section_tty() -> Tty {
    let mut tty = populated();
    tty.settings.status_bar_autohide = Some(true);
    tty.pointer = iced::Point::new(300.0, 40.0);
    tty.show_settings = true;
    tty.settings_section = 4;
    tty
}

#[test]
fn settings_history_off_view() {
    // The default off state: the fixed-at-enable choices show greyed out (key
    // source, KDF for a passphrase source, cipher, fan-out) so the section reads
    // as "configured on enable", not a set of dead controls.
    let mut tty = history_section_tty();
    tty.settings.encrypted_history_enabled = Some(false);
    tty.settings.history_key_source = Some("passphrase".to_string());
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-off.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-off` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_history_on_keychain_view() {
    // Enabled with the OS-keychain source: the live config stats (key source,
    // cipher, fan-out) and the "At startup" picker.
    let mut tty = history_section_tty();
    tty.settings.encrypted_history_enabled = Some(true);
    tty.settings.history_key_source = Some("keychain".to_string());
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-on-keychain.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-on-keychain` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_history_locked_view() {
    // Enabled with a passphrase source but locked: the "locked — not recording ·
    // Unlock…" banner over the config stats (incl. the passphrase-only KDF row).
    let mut tty = history_section_tty();
    tty.settings.encrypted_history_enabled = Some(true);
    tty.settings.history_key_source = Some("passphrase".to_string());
    tty.history_locked = true;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-locked.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-locked` changed — delete its PNG to re-baseline"
    );
}

#[test]
fn settings_history_start_failed_view() {
    // The archive couldn't be read: the error banner, with the keychain-specific
    // "switch to a passphrase" hint (since the source is the keychain).
    let mut tty = history_section_tty();
    tty.settings.encrypted_history_enabled = Some(false);
    tty.settings.history_key_source = Some("keychain".to_string());
    tty.history_start_failed = true;
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim
        .snapshot(&crate::state::theme(&tty))
        .expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-settings-history-start-failed.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-settings-history-start-failed` changed — delete its PNG to re-baseline"
    );
}

/// Regenerate the landing-page screenshots in `web/public/shots/` from the *real* app
/// render (the same headless wgpu path as the snapshot tests — not a mockup). Ignored by
/// default; run explicitly from the crate dir:
///   cargo nextest run -p tty --ignore-default-filter --run-ignored all -E 'test(generate_landing_shots)'
#[test]
#[ignore = "screenshot generator for web/public/shots — run explicitly"]
fn generate_landing_shots() {
    use iced::widget::pane_grid::Direction;
    std::fs::create_dir_all("snapshots").ok();
    std::fs::create_dir_all("../web/public/shots").ok();

    // Render one shot at a window size proportioned to its content, so the terminal fills
    // the frame (a small window is genuinely how the app renders small — no cropping).
    let save1 = |tty: &Tty, name: &str, w: f32, h: f32| {
        let mut sim = iced_test::Simulator::with_size(
            Default::default(),
            iced::Size::new(w, h),
            main_chrome(tty),
        );
        let snap = sim.snapshot(&crate::state::theme(tty)).expect("render");
        let tmp = format!("snapshots/_shot-{name}.png");
        let _ = snap.matches_image(&tmp); // writes _shot-<name>-wgpu.png
        std::fs::copy(
            format!("snapshots/_shot-{name}-wgpu.png"),
            format!("../web/public/shots/{name}.png"),
        )
        .expect("copy shot to web");
    };
    // A feature shot, rendered in both the page's dark (Dracula) and light (Solarized Light)
    // themes as `<name>.png` / `<name>-light.png`, so the captures match the page chrome as
    // its ◐ toggle flips. (The theme-showcase grid below stays single-theme via `save1`.)
    // Keep these two in step with web/src/styles/global.css `:root[data-theme=…]`.
    let save = |mut tty: Tty, name: &str, w: f32, h: f32| {
        tty.theme = Theme::named("Dracula");
        save1(&tty, name, w, h);
        tty.theme = Theme::named("Solarized Light");
        save1(&tty, &format!("{name}-light"), w, h);
    };
    const TW: f32 = 820.0; // a compact terminal window
    const TH: f32 = 340.0;

    // hero — the signature look
    save(populated(), "hero", TW, TH);

    // build — output-driven repaint / speed
    let mut tty = populated();
    tty.tabs[0] = Tab::new(painted_term(
        "zsh",
        56,
        6,
        b"\x1b[1;32muser@host\x1b[0m:\x1b[34m~/dev/tty\x1b[0m$ cargo build --release\r\n\
          \x1b[1;32m   Compiling\x1b[0m cathode v0.1.0\r\n\
          \x1b[1;32m   Compiling\x1b[0m phosphor v0.1.0\r\n\
          \x1b[1;32m    Finished\x1b[0m `release` in 9.4s\r\n$ ",
    ));
    save(tty, "build", TW, TH);

    // splits — a tab split into two panes
    let mut tty = populated();
    let win = tty.main_window.unwrap();
    tty.split_with(
        win,
        Direction::Right,
        crate::state::Pane::Term(painted_term(
            "zsh",
            28,
            6,
            b"\x1b[34m~/dev/tty\x1b[0m$ cargo test\r\n\x1b[1;32m   Compiling\x1b[0m tty\r\n\
              \x1b[32mtest result: ok\x1b[0m\r\n$ ",
        )),
    );
    save(tty, "splits", 1040.0, 400.0);

    // shell — OSC 133 prompt gutter with a failed command (red dot)
    let mut tty = populated();
    tty.settings.shell_integration.gutter = Some(true);
    tty.tabs[0] = Tab::new(painted_term(
        "zsh",
        56,
        8,
        b"\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07README.md  src  Cargo.toml\r\n\x1b]133;D;0\x07\
          \x1b]133;A\x07$ cargo test\r\n\x1b]133;C\x07\x1b[31merror: test failed\x1b[0m\r\n\x1b]133;D;1\x07\
          \x1b]133;A\x07$ ",
    ));
    save(tty, "shell", TW, TH);

    // scrollback — a longer colored transcript (scrollback feel)
    let mut tty = populated();
    tty.tabs[0] = Tab::new(painted_term(
        "zsh",
        56,
        10,
        b"\x1b[34m~/dev/tty\x1b[0m$ git log --oneline -3\r\n\
          \x1b[33md0efb35\x1b[0m ligatures\r\n\x1b[33m661a280\x1b[0m preview\r\n\x1b[33mce397e3\x1b[0m docs\r\n\
          \x1b[34m~/dev/tty\x1b[0m$ ls --color\r\n\
          \x1b[1;34msrc\x1b[0m  \x1b[32mREADME.md\x1b[0m  \x1b[1;31mtarget\x1b[0m  Cargo.toml\r\n$ ",
    ));
    save(tty, "scrollback", TW, 400.0);

    // history — the ⌘⇧H searchable command log (the encrypted-history feature)
    {
        let mut screen = TerminalScreen::new(56, 12);
        let mut parser = TermParser::new();
        for (cmd, out) in [
            (
                b"$ git status".as_slice(),
                b"\r\nOn branch main - clean\r\n".as_slice(),
            ),
            (b"$ cargo test", b"\r\ntest result: ok. 312 passed\r\n"),
            (b"$ ls --color", b"\r\nsrc  README.md  target\r\n"),
            (
                b"$ grep -rn TODO src",
                b"\r\nsrc/view.rs:88: TODO polish\r\n",
            ),
        ] {
            parser.process(cmd, &mut screen);
            screen.mark_command_boundary(50);
            parser.process(out, &mut screen);
        }
        parser.process(b"$ ", &mut screen);
        let tab = Term {
            screen: Arc::new(Mutex::new(screen)),
            pty: None,
            title: "zsh".into(),
            alive: Arc::new(AtomicBool::new(true)),
            dirty: Arc::new(AtomicBool::new(false)),
            activity: false,
            env_file: None,
        };
        let mut tty = Tty {
            tabs: vec![Tab::new(tab)],
            ..populated()
        };
        tty.show_scrollback = true;
        save(tty, "history", TW, 480.0);
    }

    // embed — the phosphor widget usage, in a terminal
    let mut tty = populated();
    tty.tabs[0] = Tab::new(painted_term(
        "zsh",
        56,
        6,
        b"\x1b[34m~/dev/app\x1b[0m$ bat src/main.rs\r\n\
          \x1b[90m 1\x1b[0m \x1b[35muse\x1b[0m phosphor::terminal;\r\n\
          \x1b[90m 2\x1b[0m\r\n\
          \x1b[90m 3\x1b[0m \x1b[35mlet\x1b[0m term = terminal(screen, style, font, size, ..);\r\n$ ",
    ));
    save(tty, "embed", TW, TH);

    // ligatures — JetBrains Mono, ligatures on
    let mut tty = populated();
    tty.font = Font::with_name("JetBrains Mono");
    tty.settings.terminal_ligatures = Some(true);
    tty.tabs[0] = Tab::new(painted_term(
        "zsh",
        56,
        6,
        b"\x1b[34m~/dev\x1b[0m$ cat lib.rs\r\n\
          fn check(x: i32) -> bool { x != 0 && x >= 1 }\r\n\
          let ok = a >= b && c <= d;  // => -> != == |>\r\n$ ",
    ));
    save(tty, "ligatures", TW, TH);

    // widgets — the optional, configurable status bar (pick the cells you want) with two
    // drill-in panels floating over the terminal: the per-core CPU grid (upper-right) and
    // the Processes table (lower-left), pinned so several stay open at once.
    {
        let mut tty = populated();
        tty.settings.status_bar_metrics = vec![
            metric("disk_io", "sparkline"),
            metric("clock", "sparkline"),
            metric("load", "sparkline"),
            metric("cpu", "sparkline"),
            metric("procs", "sparkline"),
            metric("uptime", "sparkline"),
        ];
        tty.settings.status_bar_metrics_pinned = Some(true);
        tty.window_width = 1120.0;
        tty.window_height = 640.0;
        seed_metric_sample(&mut tty);
        seed_cpu_cores(&mut tty);
        seed_processes(&mut tty);
        tty.metrics.load_avg = Some([1.32, 1.10, 0.95]);
        tty.metrics.load1_history = [0.8, 1.0, 1.2, 1.1, 1.3, 1.32].into_iter().collect();
        tty.metrics.system_uptime_secs = Some(2 * 60);
        let mut cores = crate::state::MetricPopover::new(crate::settings::MetricKind::CpuCores);
        cores.move_offset = (250.0, -150.0); // right + up
        let mut procs = crate::state::MetricPopover::new(crate::settings::MetricKind::Procs);
        procs.move_offset = (-290.0, 40.0); // left + down
        tty.metric_details = vec![cores, procs];
        save(tty, "widgets", 1120.0, 640.0);
    }

    // themes — the whole app re-skinned; same content, different palettes
    for (name, theme) in [
        ("theme-phosphor", "Phosphor"),
        ("theme-dracula", "Dracula"),
        ("theme-solarized", "Solarized Light"),
        ("theme-light", "GitHub Light"),
    ] {
        let mut tty = populated();
        tty.theme = Theme::named(theme);
        save1(&tty, name, TW, TH);
    }
}
