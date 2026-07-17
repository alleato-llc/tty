use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use iced::keyboard::Modifiers;
use iced::widget::pane_grid;
use iced::Font;

use cathode::pty::PtySession;
use cathode::screen::TerminalScreen;

use crate::history;
use crate::settings::Settings;
use crate::theme::Theme;

/// The app-state data types (`Tty`, `Term`, `Pane`, `Tab`, `MenuKind`, …).
mod types;
pub use types::*;

/// `impl Tty` methods for the opt-in encrypted command history.
mod encrypted_history;
/// `impl Tty` methods for the status-bar metrics / drill-ins / metric panes.
mod metrics;
/// `impl Tty` methods for tabs, panes, and terminal I/O.
mod panes;
/// `impl Tty` methods for the Scrollback History panel + encrypted archive.
mod scrollback;

/// How far a tab must be dragged down out of the strip before the press becomes a
/// tear-off detach (a short drag is just a click / reorder gesture).
pub const TAB_TEAR_THRESHOLD: f32 = 50.0;

/// Height of the band above the window's bottom edge within which the
/// auto-hidden status bar reveals itself (the pointer this close shows it).
/// A little taller than the bar so it appears as the pointer approaches.
pub const STATUS_BAR_REVEAL_ZONE: f32 = 56.0;

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

impl Tty {
    pub fn new(cli_untracked: bool) -> Self {
        let settings = Settings::load();
        let theme = Theme::from_settings(&settings);
        // A terminal needs a monospace face; honor a user font override if it sets one.
        let font = settings
            .font_family
            .as_deref()
            .map(named_font)
            .unwrap_or(Font::MONOSPACE);
        // Encrypted history does NOT start here: the keychain read can block
        // on an OS access dialog, and this runs on the main thread during
        // boot. `main` chains `startup_history_task()` instead, and the
        // writer/read/seed land via `apply_history_started` once it resolves.
        // The passphrase source starts *locked* — no crypto at all until the
        // user enters the passphrase in the unlock prompt opened here. An
        // untracked launch (setting, or the CLI flag) does even less: no
        // prompt, no key read, nothing.
        let plan = startup_history_plan(&settings, cli_untracked);
        if cli_untracked {
            tracing::info!("session untracked: launched with --untracked");
        } else if plan == StartupPlan::Untracked {
            tracing::info!("session untracked: the history_session_start setting");
        }
        let history_locked = plan == StartupPlan::LockedPassphrase;
        let passphrase_prompt =
            history_locked.then(|| PassphrasePrompt::new(PassphrasePromptKind::Unlock));
        let mut tty = Self {
            tabs: Vec::new(),
            active: 0,
            theme,
            font,
            font_size: settings.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            modifiers: Modifiers::default(),
            window_height: 620.0,
            window_width: 0.0,
            metric_details: Vec::new(),
            metric_detail_resize: None,
            metric_detail_move_drag: None,
            pane_replace_pending: None,
            pane_replace_confirm: None,
            hovered_tab: None,
            selection: None,
            search: None,
            search_match: 0,
            show_scrollback: false,
            scrollback_query: String::new(),
            scrollback_selected: None,
            scrollback_scroll: 0.0,
            scrollback_expanded: std::collections::HashSet::new(),
            settings,
            show_settings: false,
            settings_section: 0,
            appearance_tab: 0,
            base16_input: String::new(),
            focused: true,
            pointer: iced::Point::ORIGIN,
            menu: None,
            renaming: None,
            main_window: None,
            focused_window: None,
            detached: HashMap::new(),
            detach_origin: HashMap::new(),
            tab_drag: None,
            window_bounds: HashMap::new(),
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
            history_locked,
            passphrase_prompt,
            session_untracked: plan == StartupPlan::Untracked,
            untracked_forced_by_cli: cli_untracked,
            show_session_start_prompt: plan == StartupPlan::Ask,
            metrics: crate::metrics::Metrics::default(),
            status_bar_scroll: 0,
            status_bar_edit: false,
            status_metric_press: None,
            status_metric_drag: None,
            status_metric_drop: None,
            proc_sort: (ProcSortColumn::Cpu, true),
            proc_table_scroll: 0.0,
            proc_detail_pid: None,
        };
        tty.new_tab();
        tty
    }

    /// Begin renaming tab `idx`, seeding the draft with its current label and closing the
    /// context menu. The view focuses the rename field.
    pub fn start_rename(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get(idx) {
            self.renaming = Some((idx, tab.label()));
            self.menu = None;
        }
    }

    /// Update the in-progress rename draft.
    pub fn set_rename_draft(&mut self, text: String) {
        if let Some((_, draft)) = self.renaming.as_mut() {
            *draft = text;
        }
    }

    /// Commit the rename: a non-empty draft becomes the tab's name; an empty one clears
    /// the override (back to the program/shell title).
    pub fn commit_rename(&mut self) {
        if let Some((idx, draft)) = self.renaming.take() {
            if let Some(tab) = self.tabs.get_mut(idx) {
                let name = draft.trim();
                tab.title = (!name.is_empty()).then(|| name.to_string());
            }
        }
    }

    /// Abandon an in-progress rename (Escape / focus lost).
    pub fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    /// Open the pane context menu for a clicked pane in the main window: focus it, then
    /// anchor at the cursor. (Detached windows carry no context menu in v1.)
    pub fn open_pane_menu(&mut self, pane: pane_grid::Pane) {
        if let Some(main) = self.main_window {
            self.focus_pane(main, pane);
        }
        self.menu = Some((MenuKind::Pane, self.pointer));
    }

    /// Open the tab context menu from a right-clicked tab: activate the tab, then anchor
    /// at the cursor (its actions target that tab / its focused pane).
    pub fn open_tab_menu(&mut self, idx: usize) {
        self.activate(idx);
        self.menu = Some((MenuKind::Tab, self.pointer));
    }

    /// Dismiss any open context menu.
    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// The whole-window opacity to render with right now: the configured focused
    /// opacity while focused, the unfocused opacity otherwise. Both default to
    /// `1.0` (opaque). Applied to every surface + text color.
    pub fn window_opacity(&self) -> f32 {
        if self.focused {
            self.settings.focused_opacity()
        } else {
            self.settings.unfocused_opacity()
        }
    }

    /// Set the unfocused-window opacity (`1.0` = off). Persisted.
    pub fn set_unfocused_opacity(&mut self, opacity: f32) {
        self.settings.unfocused_opacity = Some(opacity.clamp(crate::settings::MIN_OPACITY, 1.0));
        self.settings.save();
    }

    /// Set the focused-window opacity (`1.0` = off), floored at
    /// [`crate::settings::MIN_FOCUSED_OPACITY`]. Persisted.
    pub fn set_focused_opacity(&mut self, opacity: f32) {
        self.settings.focused_opacity =
            Some(opacity.clamp(crate::settings::MIN_FOCUSED_OPACITY, 1.0));
        self.settings.save();
    }

    /// The window level for every window right now, from the always-on-top
    /// setting: `AlwaysOnTop` when on, else `Normal`. Applied at window open and
    /// whenever the setting toggles.
    pub fn window_level(&self) -> iced::window::Level {
        if self.settings.window_always_on_top() {
            iced::window::Level::AlwaysOnTop
        } else {
            iced::window::Level::Normal
        }
    }

    /// Every open window id: the main window (if any) and all detached ones. Used
    /// to broadcast a window-level change to the whole app.
    pub fn all_window_ids(&self) -> Vec<iced::window::Id> {
        self.main_window
            .into_iter()
            .chain(self.detached.keys().copied())
            .collect()
    }

    /// Toggle keeping the window above other windows. Persisted. The caller
    /// applies the new [`Self::window_level`] to the live windows.
    pub fn set_window_always_on_top(&mut self, on: bool) {
        self.settings.window_always_on_top = Some(on);
        self.settings.save();
    }

    /// Toggle inking the active tab with the accent color. Persisted.
    pub fn set_tab_highlight(&mut self, on: bool) {
        self.settings.tab_highlight = Some(on);
        self.settings.save();
    }

    /// Toggle whether a drill-in can graduate into a split pane (the ⊞ control).
    pub fn set_graduate_metrics(&mut self, on: bool) {
        self.settings.graduate_metrics = Some(on);
        self.settings.save();
    }

    /// Toggle the accent border on the focused pane (multi-pane tabs).
    pub fn set_highlight_focused_pane(&mut self, on: bool) {
        self.settings.highlight_focused_pane = Some(on);
        self.settings.save();
    }

    /// Toggle the auto-hiding status bar (persisted).
    pub fn set_status_bar_autohide(&mut self, on: bool) {
        self.settings.status_bar_autohide = Some(on);
        self.settings.save();
    }

    /// Turn the status bar off entirely, or back on (persisted). Turning it off
    /// also closes any open metric popovers (their sparklines are gone).
    pub fn set_status_bar_disabled(&mut self, on: bool) {
        self.settings.status_bar_disabled = Some(on);
        if on {
            self.metric_details.clear();
        }
        self.settings.save();
    }

    /// Toggle whether metric popovers stay pinned on a click away (persisted).
    /// Turning it off drops back to one-at-a-time: any open popovers past the
    /// first are closed so the view can't keep a stack the mode no longer allows.
    pub fn set_status_bar_metrics_pinned(&mut self, on: bool) {
        self.settings.status_bar_metrics_pinned = Some(on);
        if !on {
            self.metric_details.truncate(1);
        }
        self.settings.save();
    }

    /// Clock cell format toggles (persisted).
    pub fn set_clock_24h(&mut self, on: bool) {
        self.settings.clock_24h = Some(on);
        self.settings.save();
    }
    pub fn set_clock_seconds(&mut self, on: bool) {
        self.settings.clock_seconds = Some(on);
        self.settings.save();
    }
    pub fn set_clock_date(&mut self, on: bool) {
        self.settings.clock_date = Some(on);
        self.settings.save();
    }

    /// Whether the floating (auto-hide) status bar should show right now: the
    /// pointer sits within [`STATUS_BAR_REVEAL_ZONE`] of the window's bottom
    /// edge. Only consulted when `settings.status_bar_autohide()` is on.
    pub fn status_bar_revealed(&self) -> bool {
        self.window_height > 0.0 && self.pointer.y >= self.window_height - STATUS_BAR_REVEAL_ZONE
    }

    /// Open/close the settings panel. Closing also drops anything the archive
    /// viewer had paged in (see [`Self::close_settings_history_viewer`]).
    pub fn toggle_settings(&mut self) {
        self.show_settings = !self.show_settings;
        if !self.show_settings {
            self.close_settings_history_viewer();
        }
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
            self.search_match = 0;
            true
        }
    }

    /// Change the max-scrollback setting (clamped), persist it, and apply live to
    /// every open pane — main strip and detached windows alike — so a lowered cap
    /// truncates an already-open terminal immediately, not just new ones.
    pub fn set_max_scrollback(&mut self, n: usize) {
        let n = n.clamp(
            crate::settings::MIN_MAX_SCROLLBACK,
            crate::settings::MAX_MAX_SCROLLBACK,
        );
        self.settings.max_scrollback = Some(n);
        self.settings.save();
        for tab in self.tabs.iter().chain(self.detached.values()) {
            for term in tab.terms() {
                term.screen.lock().set_max_scrollback(n);
            }
        }
    }

    /// Nudge the max-scrollback setting from the settings stepper.
    pub fn step_max_scrollback(&mut self, delta: i64) {
        let current = self.settings.max_scrollback() as i64;
        self.set_max_scrollback((current + delta).max(0) as usize);
    }

    /// Set the default output-line cap for new commands (clamped, persisted). Applies
    /// going forward — a command already in progress keeps the cap it started with.
    pub fn set_default_output_lines(&mut self, n: usize) {
        self.settings.default_output_lines = Some(n.clamp(
            crate::settings::MIN_OUTPUT_LINES,
            crate::settings::MAX_OUTPUT_LINES,
        ));
        self.settings.save();
    }

    /// Nudge the default-output-lines setting from the settings stepper.
    pub fn step_default_output_lines(&mut self, delta: i64) {
        let current = self.settings.default_output_lines() as i64;
        self.set_default_output_lines((current + delta).max(0) as usize);
    }

    /// Mark a command boundary in `window`'s focused pane — call right before an
    /// Enter keystroke is forwarded to the shell. Resolves the per-command output cap
    /// from settings before recording (see `Settings::resolve_output_cap`).
    pub fn mark_command_boundary(&self, window: iced::window::Id) {
        let Some(term) = self.tab_for(window).and_then(Tab::focused) else {
            return;
        };
        let mut screen = term.screen.lock();
        let command = screen.current_row_text();
        let cap = self.settings.resolve_output_cap(&command);
        screen.mark_command_boundary(cap);
    }

}

impl Default for Tty {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Take a pane's pending bell + OSC 52 clipboard request, forward any queued
/// persisted-history changes to `writer` (`None` when the feature is off),
/// and clear the pane's dirty flag. Returns `(produced_signal,
/// clipboard_request)` — `produced_signal` is true if the pane wrote output
/// or rang the bell (drives the background-activity dot).
fn drain_pane(term: &mut Term, writer: Option<&history::writer::Writer>) -> (bool, Option<String>) {
    let (bell, requested, history_events) = {
        let mut s = term.screen.lock();
        (
            s.take_bell(),
            s.take_clipboard(),
            s.take_pending_history_events(),
        )
    };
    if let Some(writer) = writer {
        for event in history_events {
            writer.send(event);
        }
    }
    let was_dirty = term.dirty.swap(false, Ordering::Relaxed);
    (was_dirty || bell, requested)
}

/// Close every pane in `tab` whose shell has exited (one at a time; `close` is a no-op
/// on the last pane, so an all-dead tab keeps a single dead pane for the caller's
/// `retain`/`has_live_pane` check). Re-points focus at a survivor if it was reaped.
fn reap_tab_panes(tab: &mut Tab) {
    loop {
        // Only terminals reap (a dead shell); metric panes never exit on their own.
        let dead = tab
            .panes
            .iter()
            .find(|(_, p)| {
                p.as_term()
                    .is_some_and(|t| !t.alive.load(Ordering::Relaxed))
            })
            .map(|(p, _)| *p);
        let Some(dead) = dead else { break };
        if tab.panes.close(dead).is_none() {
            break;
        }
    }
    if tab.panes.get(tab.focus).is_none() {
        if let Some((&p, _)) = tab.panes.iter().next() {
            tab.focus = p;
        }
    }
}

/// Spawn a shell PTY + screen, run the read→parse→screen loop on a background thread,
/// and return the tab. `None` if the shell couldn't start. `cwd` starts the shell in a
/// directory (new-tab-in-cwd); `None` uses the default. `max_scrollback` is the
/// configured cap (from settings) for this terminal's scrollback buffer. `pane_tag`
/// is a display label (e.g. "Tab 2") recorded on every command persisted from this
/// screen, for context in the encrypted history archive — see
/// `TerminalScreen::set_pane_tag`.
fn spawn_term(
    cols: u16,
    rows: u16,
    cwd: Option<&str>,
    max_scrollback: usize,
    pane_tag: &str,
    id_floor: u32,
    untracked: bool,
) -> Option<Term> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let dir = cwd.map(std::path::Path::new);
    let (session, mut rx) = match PtySession::spawn_in(&shell, cols, rows, dir) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to spawn shell {shell:?}: {e}");
            return None;
        }
    };
    let mut initial_screen =
        TerminalScreen::with_scrollback(cols as usize, rows as usize, max_scrollback);
    initial_screen.set_pane_tag(pane_tag.to_string());
    // A pane spawned after the archive opened starts past the ids already
    // used today (see `Tty::history_id_floor`).
    initial_screen.reserve_command_ids(id_floor);
    initial_screen.set_untracked(untracked);
    let screen = Arc::new(Mutex::new(initial_screen));
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
