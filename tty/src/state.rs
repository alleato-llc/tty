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
            kill_confirm: None,
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
            clock_override: None,
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

    /// Toggle the OSC 133 master switch (persisted) and apply it live to every open
    /// pane, so turning it off drops the marks (and their notifications / gutter / jump
    /// targets) immediately, not just for shells spawned afterward.
    pub fn set_shell_integration_enabled(&mut self, on: bool) {
        self.settings.shell_integration.enabled = Some(on);
        self.settings.save();
        for tab in self.tabs.iter().chain(self.detached.values()) {
            for term in tab.terms() {
                term.screen.lock().set_honor_osc133(on);
            }
        }
    }

    /// Toggle completion notifications (persisted).
    pub fn set_notify_on_command_finish(&mut self, on: bool) {
        self.settings.shell_integration.notify = Some(on);
        self.settings.save();
    }

    /// Nudge the completion-notification duration threshold (seconds, clamped,
    /// persisted).
    pub fn step_notify_min_seconds(&mut self, delta: i64) {
        let current = self.settings.shell_integration().notify_min_seconds as i64;
        let next = (current + delta).clamp(
            crate::settings::MIN_NOTIFY_MIN_SECONDS as i64,
            crate::settings::MAX_NOTIFY_MIN_SECONDS as i64,
        );
        self.settings.shell_integration.notify_min_seconds = Some(next as u32);
        self.settings.save();
    }

    /// Toggle auto-installing the OSC 133 shell hooks into new shells (persisted).
    /// Applies to shells spawned after this — existing panes keep their environment.
    pub fn set_shell_integration_autoinstall(&mut self, on: bool) {
        self.settings.shell_integration.autoinstall = Some(on);
        self.settings.save();
    }

    /// Toggle the OSC 133 prompt gutter (persisted).
    pub fn set_prompt_gutter(&mut self, on: bool) {
        self.settings.shell_integration.gutter = Some(on);
        self.settings.save();
    }

    /// Set the ⌘-click open-file command template (persisted). A blank value clears
    /// it, restoring the OS-opener default (see
    /// [`crate::settings::resolve_open_file_command`]).
    pub fn set_open_file_command(&mut self, template: String) {
        let trimmed = template.trim();
        self.settings.open_file_command = (!trimmed.is_empty()).then(|| trimmed.to_string());
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

    /// Re-read `tty.toml` and adopt it if it changed on disk since we last wrote it —
    /// the **live-reload** path for a hand-edit made in another editor. Called when a
    /// window regains focus (the moment the user switches back to tty). No-op when the
    /// file matches what's already in memory (including right after our own GUI save).
    /// Does not write back: we're adopting the file, not re-serializing it.
    pub fn reload_settings_if_changed(&mut self) -> bool {
        // Like `Settings::save`, never touch the real config file under test — a test
        // run must not read whoever-ran-`cargo test`'s settings into the fixture.
        if cfg!(test) {
            return false;
        }
        self.adopt_settings(Settings::load())
    }

    /// Adopt `next` as the live settings if it differs from what's in memory, rebuilding
    /// the render-derived state (theme, font) and pushing the live-applicable settings
    /// (the scrollback cap) to open panes. Returns whether anything changed. Does not
    /// save — the caller is reconciling *from* disk, not *to* it.
    pub(crate) fn adopt_settings(&mut self, next: Settings) -> bool {
        if next == self.settings {
            return false;
        }
        self.settings = next;
        // Rebuild the render-derived state the way `Tty::new` does.
        self.theme = Theme::from_settings(&self.settings);
        self.font = self
            .settings
            .font_family
            .as_deref()
            .map(named_font)
            .unwrap_or(Font::MONOSPACE);
        self.font_size = self.settings.font_size.unwrap_or(DEFAULT_FONT_SIZE);
        // The scrollback cap and the OSC 133 master gate have to reach every open pane
        // (mirrors `set_max_scrollback` / `set_shell_integration_enabled`). Opacity,
        // status-bar flags, metrics, notify/gutter, etc. are read from `settings` at
        // render/drain time, so adopting the struct is enough for them.
        let cap = self.settings.max_scrollback();
        let honor_osc133 = self.settings.shell_integration().enabled;
        for tab in self.tabs.iter().chain(self.detached.values()) {
            for term in tab.terms() {
                let mut screen = term.screen.lock();
                screen.set_max_scrollback(cap);
                screen.set_honor_osc133(honor_osc133);
            }
        }
        true
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

    /// OSC 133 **prompt-jump**: move the focused pane's [`Self::scroll_target`] to the
    /// previous (`back`) or next command prompt — every prompt (`⌘↑`/`⌘↓`), or only the
    /// **failed** ones when `failed_only` (`⌘⇧↑`/`⌘⇧↓`). Positions come from the pane's
    /// recorded [`command regions`](cathode::screen::TerminalScreen::command_regions), so
    /// it's a no-op when the shell has no integration (no marks). The view feeds the
    /// resulting target to the pane's `scroll_to`.
    pub fn jump_to_prompt(&mut self, window: iced::window::Id, back: bool, failed_only: bool) {
        let Some(term) = self.tab_for(window).and_then(Tab::focused) else {
            return;
        };
        let mut prompts: Vec<usize> = term
            .screen
            .lock()
            .command_regions()
            .iter()
            .filter(|r| !failed_only || r.failed())
            .map(|r| r.prompt_row)
            .collect();
        prompts.sort_unstable();
        prompts.dedup();
        if prompts.is_empty() {
            return;
        }
        let cur = self.scroll_target;
        self.scroll_target = if back {
            // Previous (earlier) prompt: the largest below the current target; on the
            // first press (no target) that's the newest prompt. Stays put past the oldest.
            let bound = cur.unwrap_or(usize::MAX);
            prompts.iter().rev().find(|&&p| p < bound).copied().or(cur)
        } else {
            // Next (later) prompt: the smallest above the current target. At the live
            // bottom (`None`) there's nothing below; past the newest it stays put.
            match cur {
                Some(c) => prompts.iter().find(|&&p| p > c).copied().or(cur),
                None => None,
            }
        };
    }

    /// Clear the prompt-jump target (back to the live bottom) — called when the user
    /// sends input to the shell, so the next `⌘↑` starts from the newest prompt again.
    pub fn clear_prompt_jump(&mut self) {
        self.scroll_target = None;
    }

    /// Toggle the Env view feature (persisted). Turning it off closes the popover if
    /// open. Takes effect on shells started after (existing ones weren't handed a
    /// capture file) — the popover's empty state says so.
    pub fn set_env_view(&mut self, on: bool) {
        self.settings.shell_integration.env_view = Some(on);
        self.settings.save();
        if !on && self.show_env {
            self.toggle_env_view();
        }
    }

    /// Open/close the **Env view** for the active pane. Opening flips the shell's capture
    /// flag on (`<env_file>.on`, so it re-dumps each prompt) and reads the current
    /// baseline; closing flips it off. See [`crate::env`].
    pub fn toggle_env_view(&mut self) {
        if self.show_env {
            self.show_env = false;
            self.env_vars.clear();
            self.env_source = EnvSource::None;
            self.env_os_cache = None;
            if let Some(flag) = self.active_env_flag() {
                let _ = std::fs::remove_file(flag);
            }
            return;
        }
        self.show_env = true;
        self.env_filter.clear();
        if let Some(flag) = self.active_env_flag() {
            let _ = std::fs::write(flag, b"");
        }
        self.refresh_env();
    }

    /// Refresh the Env view (no-op when closed) — called each redraw while open so it
    /// tracks the shell across commands. Prefers the **live** hook capture (the shell
    /// dumps its env each prompt); when that's empty — the feature is off, the hooks
    /// aren't installed, or no prompt has fired — it falls back to the pane process's
    /// **launch-time** environment read straight from the OS, so the view shows real
    /// variables with zero setup. The OS read is a full process-detail scan, so it's
    /// cached per pid ([`Self::active_process_env`]) rather than repeated every frame.
    pub fn refresh_env(&mut self) {
        if !self.show_env {
            return;
        }
        let hook = self
            .active_term()
            .and_then(|t| t.env_file.as_deref())
            .map(crate::env::read)
            .unwrap_or_default();
        if !hook.is_empty() {
            self.env_vars = hook;
            self.env_source = EnvSource::Hook;
            return;
        }
        let os = self.active_process_env();
        self.env_source = if os.is_empty() {
            EnvSource::None
        } else {
            EnvSource::Process
        };
        self.env_vars = os;
    }

    /// The active pane shell's pid, if it's still running.
    fn active_shell_pid(&self) -> Option<i32> {
        self.active_term()
            .and_then(|t| t.pty.as_ref())
            .and_then(|s| s.child.process_id())
            .map(|p| p as i32)
    }

    /// The active pane shell's launch-time environment, read from the kernel via
    /// `prexp-core` (`KERN_PROCARGS2` on macOS, `/proc/<pid>/environ` on Linux) and
    /// cached by pid — the read is a heavy process-detail scan and the launch env is
    /// static, so it runs once per pid, not each redraw. Empty when there's no live pid
    /// or the read fails (the process exited, or the OS denied it).
    fn active_process_env(&mut self) -> Vec<crate::env::EnvVar> {
        let Some(pid) = self.active_shell_pid() else {
            self.env_os_cache = None;
            return Vec::new();
        };
        if self.env_os_cache.as_ref().map(|(p, _)| *p) != Some(pid) {
            use prexp_core::source::ProcessSource;
            let vars = prexp_core::backend::NativeSource::new()
                .process_detail(pid, "")
                .map(|d| crate::env::from_pairs(d.environment))
                .unwrap_or_default();
            self.env_os_cache = Some((pid, vars));
        }
        self.env_os_cache
            .as_ref()
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    /// "Now" in Unix-epoch milliseconds for wall-clock-relative labels (the History
    /// viewer's "N ago" and archived-date columns). Reads the real clock unless
    /// [`Tty::clock_override`] pins it (snapshot tests, for determinism across midnight).
    pub fn now_ms(&self) -> u64 {
        self.clock_override.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        })
    }

    /// The `<env_file>.on` capture-enable flag path for the active pane's shell.
    fn active_env_flag(&self) -> Option<std::path::PathBuf> {
        self.active_term()
            .and_then(|t| t.env_file.as_ref())
            .map(|p| p.with_extension("on"))
    }

    /// Commit the "add to new shells" draft into the persisted env overlay
    /// ([`crate::settings::Settings::env`], applied at spawn). A blank name is ignored;
    /// on success the draft fields clear. Affects shells spawned *after* this, not open ones.
    pub fn add_env_overlay(&mut self) {
        let name = self.env_overlay_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.settings
            .env
            .insert(name, std::mem::take(&mut self.env_overlay_value));
        self.settings.save();
        self.env_overlay_name.clear();
    }

    /// Remove a variable from the new-shells env overlay (persisted).
    pub fn remove_env_overlay(&mut self, name: &str) {
        self.settings.env.remove(name);
        self.settings.save();
    }

    /// Inject `export NAME='value'` at the focused shell's prompt (the Env popover's
    /// "set in this pane" action). Visible on purpose — it shows in the terminal, so
    /// the change is self-documenting. No-op unless the name is valid (so the value
    /// quotes cleanly) and the shell is at a prompt. Clears the draft on success.
    pub fn inject_env_set(&mut self) {
        let bytes = crate::env::export_command(self.env_set_name.trim(), &self.env_set_value);
        if let Some(bytes) = bytes {
            if self.inject_env(bytes) {
                self.env_set_name.clear();
                self.env_set_value.clear();
            }
        }
    }

    /// Inject `unset NAME` at the focused shell's prompt.
    pub fn inject_env_unset(&mut self) {
        if let Some(bytes) = crate::env::unset_command(self.env_set_name.trim()) {
            if self.inject_env(bytes) {
                self.env_set_name.clear();
                self.env_set_value.clear();
            }
        }
    }

    /// Write env-edit bytes to the focused pane, but only when the shell is at a prompt
    /// (OSC 133) — never into a running foreground program. Returns whether it was sent.
    fn inject_env(&mut self, bytes: Vec<u8>) -> bool {
        // Opt-in: editing a running shell types into it, so it's off unless enabled.
        if !self.settings.shell_integration().env_editing {
            return false;
        }
        let Some(win) = self.main_window else {
            return false;
        };
        if self
            .active_term()
            .is_some_and(|t| t.screen.lock().command_running())
        {
            return false;
        }
        self.write_focused(win, &bytes);
        true
    }

    /// The Env popover's current top-left position, defaulting to centered until the
    /// user drags it (then [`Self::env_pos`] remembers where they left it).
    pub fn env_effective_pos(&self) -> (f32, f32) {
        self.env_pos.unwrap_or_else(|| {
            let (w, h) = self.env_size;
            let x = ((self.window_width - w) / 2.0).max(0.0);
            let y = ((self.window_height - h) / 2.0).max(20.0);
            (x, y)
        })
    }

    /// The captured output of the most recent OSC 133 command in `window`'s focused
    /// pane, as text, or `None` when there's no finished command with output (no shell
    /// integration, or nothing has run). The `C`→`D` line span is recorded per region;
    /// the text comes from the buffer at those lines.
    pub fn last_command_output(&self, window: iced::window::Id) -> Option<String> {
        let term = self.tab_for(window).and_then(Tab::focused)?;
        let screen = term.screen.lock();
        let (start, end) = screen
            .command_regions()
            .into_iter()
            .rev()
            .find_map(|r| r.output)?;
        let lines = screen.transcript_lines();
        // `end` is exclusive (the `D` row, one past the last output line).
        let end = end.min(lines.len());
        let text = lines.get(start..end)?.join("\n");
        let text = text.trim_end().to_string();
        (!text.is_empty()).then_some(text)
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
fn drain_pane(
    term: &mut Term,
    writer: Option<&history::writer::Writer>,
) -> (
    bool,
    Option<String>,
    Vec<cathode::screen::CommandCompletion>,
) {
    let (bell, requested, history_events, completions) = {
        let mut s = term.screen.lock();
        (
            s.take_bell(),
            s.take_clipboard(),
            s.take_pending_history_events(),
            s.take_command_completions(),
        )
    };
    if let Some(writer) = writer {
        for event in history_events {
            writer.send(event);
        }
    }
    let was_dirty = term.dirty.swap(false, Ordering::Relaxed);
    (was_dirty || bell, requested, completions)
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
#[allow(clippy::too_many_arguments)]
fn spawn_term(
    cols: u16,
    rows: u16,
    cwd: Option<&str>,
    max_scrollback: usize,
    pane_tag: &str,
    id_floor: u32,
    untracked: bool,
    integration: crate::settings::ResolvedShellIntegration,
    overlay: &std::collections::BTreeMap<String, String>,
) -> Option<Term> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let dir = cwd.map(std::path::Path::new);
    let mut env = if integration.autoinstall {
        crate::shell_integration::autoinstall_env(&shell, integration.env_view)
    } else {
        Vec::new()
    };
    // The Env view's "new sessions" overlay — vars the user set to apply to every shell.
    for (k, v) in overlay {
        env.push((k.clone(), v.clone()));
    }
    // Only when the Env view is enabled: hand the shell a per-session file to capture
    // its env into. Without `TTY_ENV_FILE` the hook's `_tty_capture_env` is a no-op
    // (one string test), so an install that doesn't use the view does no env work.
    let env_file = integration
        .env_view
        .then(crate::shell_integration::env_channel_path)
        .flatten();
    if let Some(path) = &env_file {
        env.push((
            "TTY_ENV_FILE".to_string(),
            path.to_string_lossy().into_owned(),
        ));
    }
    let (session, mut rx) = match PtySession::spawn_in_env(&shell, cols, rows, dir, &env) {
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
    initial_screen.set_honor_osc133(integration.enabled);
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
        env_file,
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
