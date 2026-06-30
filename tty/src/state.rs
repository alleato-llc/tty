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
    /// Set by the read thread on output; the UI swaps it false each redraw to light a
    /// background tab's activity dot.
    pub dirty: Arc<AtomicBool>,
    /// Unseen output / bell on a background tab (shown as a • in the tab strip).
    pub activity: bool,
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
    /// The scrollback search query when the `⌘F` find bar is open (`None` = closed).
    pub search: Option<String>,
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
            search: None,
        };
        tty.new_tab();
        tty
    }

    /// Toggle the `⌘F` find bar. Opening returns the search-field id to focus.
    pub fn toggle_search(&mut self) -> bool {
        if self.search.is_some() {
            self.search = None;
            false
        } else {
            self.search = Some(String::new());
            true
        }
    }

    pub fn active_term(&self) -> Option<&Term> {
        self.tabs.get(self.active)
    }

    /// Make tab `idx` active and clear its unseen-activity dot.
    pub fn activate(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
            self.tabs[idx].activity = false;
        }
    }

    /// Spawn a shell in a new tab and make it active. The new shell starts in the
    /// active tab's reported working directory (OSC 7) when known.
    pub fn new_tab(&mut self) {
        let cwd = self.active_term().and_then(|t| t.screen.lock().cwd.clone());
        if let Some(term) = spawn_term(80, 24, cwd.as_deref()) {
            self.tabs.push(term);
            self.active = self.tabs.len() - 1;
        }
    }

    /// The active tab's DEC application-cursor-keys mode (affects arrow-key bytes).
    pub fn active_app_cursor(&self) -> bool {
        self.active_term()
            .map(|t| t.screen.lock().app_cursor_keys)
            .unwrap_or(false)
    }

    /// Paste `text` into the active shell, wrapping it in bracketed-paste markers when
    /// the app enabled mode 2004 (so multi-line paste can't auto-execute).
    pub fn paste(&mut self, text: &str) {
        let bracketed = self
            .active_term()
            .map(|t| t.screen.lock().bracketed_paste)
            .unwrap_or(false);
        let mut bytes = Vec::new();
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        self.write_active(&bytes);
    }

    /// Per-redraw housekeeping: light activity dots on background tabs that produced
    /// output or rang the bell, clear the active tab's dot, and surface any OSC 52
    /// clipboard-write request for the host to put on the system clipboard.
    pub fn drain_effects(&mut self) -> Option<String> {
        let active = self.active;
        let mut clip = None;
        for (i, term) in self.tabs.iter_mut().enumerate() {
            let (bell, requested) = {
                let mut s = term.screen.lock();
                (s.take_bell(), s.take_clipboard())
            };
            if let Some(c) = requested {
                clip = Some(c);
            }
            let was_dirty = term.dirty.swap(false, Ordering::Relaxed);
            if (was_dirty || bell) && i != active {
                term.activity = true;
            }
        }
        if let Some(t) = self.tabs.get_mut(active) {
            t.activity = false;
        }
        clip
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
/// and return the tab. `None` if the shell couldn't start. `cwd` starts the shell in a
/// directory (new-tab-in-cwd); `None` uses the default.
fn spawn_term(cols: u16, rows: u16, cwd: Option<&str>) -> Option<Term> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let dir = cwd.map(std::path::Path::new);
    let (session, mut rx) = match PtySession::spawn_in(&shell, cols, rows, dir) {
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
    let dirty = Arc::new(AtomicBool::new(false));
    let read_into = screen.clone();
    let alive_flag = alive.clone();
    let dirty_flag = dirty.clone();
    std::thread::spawn(move || {
        let mut parser = cathode::parser::TermParser::new();
        while let Some(data) = rx.blocking_recv() {
            parser.process(&data, &mut read_into.lock());
            dirty_flag.store(true, Ordering::Relaxed);
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
        dirty,
        activity: false,
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
