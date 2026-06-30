use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use iced::keyboard::Modifiers;
use iced::Font;

use cathode::pty::PtySession;
use cathode::screen::TerminalScreen;

use crate::settings::Settings;
use crate::theme::Theme;

pub const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Zoom clamp (⌘+/⌘−).
pub const MIN_FONT_SIZE: f32 = 7.0;
pub const MAX_FONT_SIZE: f32 = 40.0;

/// One terminal tab: a shell PTY plus the screen its output is parsed into (shared
/// with the background read thread).
pub struct Term {
    pub screen: Arc<Mutex<TerminalScreen>>,
    /// `None` only in tests (a screen-only tab with no shell behind it).
    pub pty: Option<PtySession>,
    pub title: String,
    /// Cleared by the read thread when the PTY closes (the shell exited), so the UI
    /// can reap the tab.
    pub alive: Arc<AtomicBool>,
}

/// The whole app: a stack of terminal tabs + theme/font chrome. The terminal counterpart
/// of `fed` — thin glue over `cathode` (engine) + `phosphor` (widget) + `rime`.
pub struct Tty {
    pub tabs: Vec<Term>,
    pub active: usize,
    pub theme: Theme,
    pub font: Font,
    pub font_size: f32,
    pub modifiers: Modifiers,
    pub window_height: f32,
    pub hovered_tab: Option<usize>,
    /// The active terminal's current selection text (for ⌘C).
    pub selection: Option<String>,
}

impl Tty {
    pub fn new() -> Self {
        let settings = Settings::load();
        let theme = Theme::new(settings.theme_choice());
        // A terminal needs a monospace face; honor a user font override if it sets one.
        let font = settings
            .font_family
            .as_deref()
            .map(named_font)
            .unwrap_or(Font::MONOSPACE);
        let mut tty = Self {
            tabs: Vec::new(),
            active: 0,
            theme,
            font,
            font_size: settings.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            modifiers: Modifiers::default(),
            window_height: 620.0,
            hovered_tab: None,
            selection: None,
        };
        tty.new_tab();
        tty
    }

    pub fn active_term(&self) -> Option<&Term> {
        self.tabs.get(self.active)
    }

    /// Spawn a shell in a new tab and make it active.
    pub fn new_tab(&mut self) {
        if let Some(term) = spawn_term(80, 24) {
            self.tabs.push(term);
            self.active = self.tabs.len() - 1;
        }
    }

    /// Close tab `idx`. Returns `false` when the last tab closes (the caller exits).
    pub fn close_tab(&mut self, idx: usize) -> bool {
        if idx < self.tabs.len() {
            self.tabs.remove(idx);
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        true
    }

    /// Forward `bytes` to the active tab's shell.
    pub fn write_active(&mut self, bytes: &[u8]) {
        if let Some(term) = self.tabs.get_mut(self.active) {
            if let Some(pty) = term.pty.as_mut() {
                if let Err(e) = pty.write_bytes(bytes) {
                    tracing::warn!("PTY write failed: {e}");
                }
            }
        }
    }

    /// Resize the active tab's grid + PTY (SIGWINCH) to what the widget reports fits.
    pub fn resize_active(&mut self, cols: usize, rows: usize) {
        if let Some(term) = self.tabs.get(self.active) {
            term.screen.lock().resize(cols, rows);
            if let Some(pty) = term.pty.as_ref() {
                let _ = pty.resize(cols as u16, rows as u16);
            }
        }
    }

    /// Adjust the font size (⌘+/⌘−/⌘0), clamped. The widget re-measures the grid on
    /// the next event and resizes the PTY.
    pub fn zoom(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    }

    pub fn reset_zoom(&mut self) {
        self.font_size = DEFAULT_FONT_SIZE;
    }

    /// Drop tabs whose shell has exited. Returns `false` when none remain (the app
    /// should exit). Keeps the active tab valid.
    pub fn reap_dead(&mut self) -> bool {
        let active_alive = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.alive.load(Ordering::Relaxed));
        self.tabs.retain(|t| t.alive.load(Ordering::Relaxed));
        if self.tabs.is_empty() {
            return false;
        }
        // If the active tab died, fall back to the last; otherwise just clamp.
        if !active_alive || self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        true
    }
}

impl Default for Tty {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a shell PTY + screen, run the read→parse→screen loop on a background thread,
/// and return the tab. `None` if the shell couldn't start.
fn spawn_term(cols: u16, rows: u16) -> Option<Term> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let (session, mut rx) = match PtySession::spawn(&shell, cols, rows) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to spawn shell {shell:?}: {e}");
            return None;
        }
    };
    let screen = Arc::new(Mutex::new(TerminalScreen::new(
        cols as usize,
        rows as usize,
    )));
    let alive = Arc::new(AtomicBool::new(true));
    let read_into = screen.clone();
    let alive_flag = alive.clone();
    std::thread::spawn(move || {
        let mut parser = cathode::parser::TermParser::new();
        while let Some(data) = rx.blocking_recv() {
            parser.process(&data, &mut read_into.lock());
            cathode::wake::signal(); // repaint on output
        }
        // The channel closed — cathode's reader hit EOF, i.e. the shell exited.
        alive_flag.store(false, Ordering::Relaxed);
        cathode::wake::signal(); // wake the UI to reap the dead tab
    });
    let title = shell.rsplit('/').next().unwrap_or("shell").to_string();
    Some(Term {
        screen,
        pty: Some(session),
        title,
        alive,
    })
}

/// An iced `Font` for a family name. The name is leaked to `&'static str` (font
/// families are chosen once at startup, so this is a bounded, intentional leak).
fn named_font(family: &str) -> Font {
    Font::with_name(Box::leak(family.to_string().into_boxed_str()))
}

/// Per-window theme for the iced runtime (scrollbars etc.).
pub fn theme(state: &Tty) -> iced::Theme {
    state.theme.iced()
}
