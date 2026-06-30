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

/// The "no override" label in the font picker — the iced built-in monospace.
pub const DEFAULT_FONT_LABEL: &str = "System Monospace";

/// A small curated set of common terminal fonts offered in the settings picker. iced
/// loads a family by name from whatever the OS has installed; a missing font silently
/// falls back, so the list is a convenience, not a guarantee it's present.
pub const FONT_CHOICES: &[&str] = &[
    DEFAULT_FONT_LABEL,
    "Menlo",
    "Monaco",
    "SF Mono",
    "JetBrains Mono",
    "Fira Code",
    "Cascadia Code",
    "Hack",
    "Source Code Pro",
    "IBM Plex Mono",
];

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
    /// Persisted preferences (theme, font, custom palette).
    pub settings: Settings,
    /// Whether the `⌘,` settings panel is open.
    pub show_settings: bool,
    /// The active settings section (0 = Appearance, 1 = Palette).
    pub settings_section: usize,
    /// The base16 paste box's contents (16 hex colors to import).
    pub base16_input: String,
    /// Whether the window currently has focus (drives the unfocused-opacity effect).
    pub focused: bool,
}

impl Tty {
    pub fn new() -> Self {
        let settings = Settings::load();
        let theme = Theme::from_settings(&settings);
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
            settings,
            show_settings: false,
            settings_section: 0,
            base16_input: String::new(),
            focused: true,
        };
        tty.new_tab();
        tty
    }

    /// The whole-window opacity to render with right now: opaque while focused, the
    /// configured unfocused opacity otherwise. Applied to every surface + text color.
    pub fn window_opacity(&self) -> f32 {
        if self.focused {
            1.0
        } else {
            self.settings.unfocused_opacity()
        }
    }

    /// Set the unfocused-window opacity (`1.0` = off). Persisted.
    pub fn set_unfocused_opacity(&mut self, opacity: f32) {
        self.settings.unfocused_opacity = Some(opacity.clamp(0.3, 1.0));
        self.settings.save();
    }

    /// Open/close the settings panel.
    pub fn toggle_settings(&mut self) {
        self.show_settings = !self.show_settings;
    }

    /// Rebuild the live theme from the current settings and persist them. Every settings
    /// mutation funnels through here so the panel, disk, and render all stay in step.
    fn apply_settings(&mut self) {
        self.theme = Theme::from_settings(&self.settings);
        self.settings.save();
    }

    /// Pick a named built-in theme. Selecting one drops any custom palette so the
    /// theme's own colors take over. The synthetic "Custom" entry (shown while a custom
    /// palette is active) isn't a real theme, so re-selecting it is a no-op.
    pub fn set_theme(&mut self, name: &str) {
        if name.eq_ignore_ascii_case("custom") {
            return;
        }
        self.settings.theme = Some(name.to_string());
        self.settings.palette = None;
        self.apply_settings();
    }

    /// Set the terminal font family (or revert to the default monospace).
    pub fn set_font(&mut self, family: &str) {
        if family.is_empty() || family.eq_ignore_ascii_case(DEFAULT_FONT_LABEL) {
            self.settings.font_family = None;
            self.font = Font::MONOSPACE;
        } else {
            self.settings.font_family = Some(family.to_string());
            self.font = named_font(family);
        }
        self.settings.save();
    }

    /// Nudge the font size from the settings stepper (clamped, persisted).
    pub fn step_font_size(&mut self, delta: f32) {
        self.zoom(delta);
        self.settings.font_size = Some(self.font_size);
        self.settings.save();
    }

    /// Import the base16 colors in `base16_input` as the terminal palette. No-op if the
    /// box doesn't hold exactly 16 parseable hex colors.
    pub fn apply_base16(&mut self) {
        if let Some(style) = crate::theme::base16::parse(&self.base16_input) {
            self.settings.set_palette(&style);
            self.apply_settings();
        }
    }

    /// Drop the custom palette, back to the built-in dark/light colors.
    pub fn reset_palette(&mut self) {
        self.settings.palette = None;
        self.apply_settings();
    }

    /// Edit one palette slot (`0..16` = ANSI, `16`=fg, `17`=bg, `18`=cursor), starting
    /// from the live palette so single-color tweaks compose.
    pub fn edit_color(&mut self, idx: usize, color: iced::Color) {
        let mut style = self.theme.terminal;
        match idx {
            0..=15 => style.ansi[idx] = color,
            16 => style.fg = color,
            17 => style.bg = color,
            18 => style.cursor = color,
            _ => return,
        }
        self.settings.set_palette(&style);
        self.apply_settings();
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

/// Per-window theme for the iced runtime (scrollbars etc.), faded with the rest of the
/// window when it's unfocused so built-in widgets dim in step.
pub fn theme(state: &Tty) -> iced::Theme {
    let op = state.window_opacity();
    crate::theme::fade_palette(state.theme.palette, op).iced_theme("tty")
}
