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
fn split_pane_view() {
    use iced::widget::pane_grid::Direction;
    // Split the active tab into two side-by-side panes (the new one, on the right, takes
    // focus and gets the accent border).
    let mut tty = populated();
    let win = tty.main_window.unwrap();
    tty.split_with(
        win,
        Direction::Right,
        painted_term(
            "zsh",
            28,
            6,
            b"$ cargo test\r\n\x1b[32m   Compiling\x1b[0m tty\r\n$ ",
        ),
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
