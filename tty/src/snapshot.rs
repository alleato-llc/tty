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

use crate::state::{Term, Tty, DEFAULT_FONT_SIZE};
use crate::theme::Theme;
use crate::view::view;

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
        tabs: vec![tab, second],
        active: 0,
        theme: Theme::default(),
        font: Font::MONOSPACE,
        font_size: DEFAULT_FONT_SIZE,
        modifiers: iced::keyboard::Modifiers::default(),
        window_height: 600.0,
        hovered_tab: None,
        selection: None,
        search: None,
        settings: Default::default(),
        show_settings: false,
        settings_section: 0,
        base16_input: String::new(),
    }
}

#[test]
fn terminal_view() {
    let tty = populated();
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(view(&tty));
    let snap = sim.snapshot(&tty.theme.iced()).expect("render snapshot");
    let matches = snap
        .matches_image("snapshots/tty-terminal.png")
        .expect("write/compare snapshot");
    assert!(
        matches,
        "snapshot `tty-terminal` changed — delete its PNG to re-baseline"
    );
}
