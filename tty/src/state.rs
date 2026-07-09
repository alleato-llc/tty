use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use iced::keyboard::Modifiers;
use iced::widget::pane_grid;
use iced::Font;

use cathode::pty::PtySession;
use cathode::screen::TerminalScreen;

use crate::message::Message;
use crate::settings::Settings;
use crate::theme::Theme;

/// How far a tab must be dragged down out of the strip before the press becomes a
/// tear-off detach (a short drag is just a click / reorder gesture).
pub const TAB_TEAR_THRESHOLD: f32 = 50.0;

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

/// One tab: a tree of terminal panes (a single pane until the user splits) plus which
/// pane currently has focus. The split tree, drag-to-resize dividers, and cardinal
/// navigation are owned by iced's `pane_grid::State`; we just hold a `Term` per pane.
pub struct Tab {
    pub panes: pane_grid::State<Term>,
    pub focus: pane_grid::Pane,
    /// A user-set name (via "Rename tab"). When `None`, the label comes from the focused
    /// pane's program title (OSC 0/2) or shell name.
    pub title: Option<String>,
}

impl Tab {
    /// A new tab wrapping a single shell pane.
    pub fn new(term: Term) -> Self {
        let (panes, focus) = pane_grid::State::new(term);
        Self {
            panes,
            focus,
            title: None,
        }
    }

    /// The focused pane's terminal.
    pub fn focused(&self) -> Option<&Term> {
        self.panes.get(self.focus)
    }

    /// The tab's display label: a user-set name wins, else the focused pane's program
    /// title (OSC 0/2), else its shell name.
    pub fn label(&self) -> String {
        if let Some(name) = &self.title {
            return name.clone();
        }
        self.focused()
            .map(|term| {
                term.screen
                    .lock()
                    .title
                    .clone()
                    .unwrap_or_else(|| term.title.clone())
            })
            .unwrap_or_default()
    }

    /// Whether any pane in this tab still has a live shell.
    fn has_live_pane(&self) -> bool {
        self.panes
            .iter()
            .any(|(_, t)| t.alive.load(Ordering::Relaxed))
    }
}

/// The whole app: a stack of tabs (each a pane tree) + theme/font chrome. The terminal
/// counterpart of `fed` — thin glue over `cathode` (engine) + `phosphor` (widget) + `rime`.
pub struct Tty {
    pub tabs: Vec<Tab>,
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
    /// How many times Enter/⇧Enter advanced through the `⌘F` matches — an
    /// ever-moving counter, not a bounds-checked index; the view reduces it modulo
    /// the live match count (which can shrink as the query changes or output
    /// arrives) via `rem_euclid`, so it never needs resetting except on a new query.
    pub search_match: i64,
    /// Whether the scrollback history panel (⌘⇧H) is open for the active pane.
    pub show_scrollback: bool,
    /// The scrollback panel's own filter query (independent of `⌘F`'s `search`).
    pub scrollback_query: String,
    /// The scrollback panel table's selected row (a table-row index — a command
    /// header row or one of its expanded output rows).
    pub scrollback_selected: Option<usize>,
    /// The scrollback panel table's vertical scroll offset (pixels).
    pub scrollback_scroll: f32,
    /// Which commands (indices into the *filtered* command list) have their output
    /// expanded — reset whenever the filter changes, since indices shift with it.
    pub scrollback_expanded: std::collections::HashSet<usize>,
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
    /// Last known cursor position (window-relative), used to anchor the right-click menu.
    pub pointer: iced::Point,
    /// When `Some`, a right-click context menu is open: its kind (tab vs pane, which
    /// picks the item set) and the point to anchor it at. Both act on the active tab.
    pub menu: Option<(MenuKind, iced::Point)>,
    /// When `Some`, a tab is being renamed: its index and the in-progress draft text.
    pub renaming: Option<(usize, String)>,

    // ---- multi-window: detachable tabs (ADR 0003) ----
    /// The main window's id (the tabbed strip). Set once in `boot`.
    pub main_window: Option<iced::window::Id>,
    /// Which window currently has keyboard focus — chords/typing route to its tab.
    pub focused_window: Option<iced::window::Id>,
    /// Tabs torn off into their own OS windows. The owned pane tree moves here and back.
    pub detached: HashMap<iced::window::Id, Tab>,
    /// The main-strip index each detached tab came from, so reattach drops it back there.
    pub detach_origin: HashMap<iced::window::Id, usize>,
    /// An armed tab tear-off: the pressed tab index + the pointer at press. A drag past
    /// [`TAB_TEAR_THRESHOLD`] on release detaches it.
    pub tab_drag: Option<(usize, iced::Point)>,
    /// Each window's last-known outer bounds (for the drag-to-dock heuristic).
    pub window_bounds: HashMap<iced::window::Id, iced::Rectangle>,
    /// The most recent detached-window move + when, debounced by `detach_drag`.
    pub last_detached_move: Option<(iced::window::Id, Instant)>,
}

/// Which right-click menu is open — a tab's (split + tab actions), a pane's (split +
/// close pane), or a link's (open/copy a detected URL). The first two target the
/// active tab's focused pane for splits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuKind {
    Tab,
    Pane,
    Link(String),
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
            search_match: 0,
            show_scrollback: false,
            scrollback_query: String::new(),
            scrollback_selected: None,
            scrollback_scroll: 0.0,
            scrollback_expanded: std::collections::HashSet::new(),
            settings,
            show_settings: false,
            settings_section: 0,
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
        self.settings.unfocused_opacity = Some(opacity.clamp(crate::settings::MIN_OPACITY, 1.0));
        self.settings.save();
    }

    /// Toggle inking the active tab with the accent color. Persisted.
    pub fn set_tab_highlight(&mut self, on: bool) {
        self.settings.tab_highlight = Some(on);
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
            self.search_match = 0;
            true
        }
    }

    /// Toggle the scrollback history panel; closing clears its filter so reopening
    /// starts fresh.
    pub fn toggle_scrollback_panel(&mut self) {
        self.show_scrollback = !self.show_scrollback;
        if !self.show_scrollback {
            self.scrollback_query.clear();
            self.scrollback_selected = None;
            self.scrollback_scroll = 0.0;
            self.scrollback_expanded.clear();
        }
    }

    /// Update the scrollback panel's filter — a new query invalidates the row
    /// selection and any expanded commands (both index into the filtered list,
    /// which just changed).
    pub fn set_scrollback_query(&mut self, query: String) {
        self.scrollback_query = query;
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
    }

    /// Toggle whether a command (its index into the *filtered* command list) shows
    /// its output.
    pub fn toggle_scrollback_expand(&mut self, index: usize) {
        if !self.scrollback_expanded.remove(&index) {
            self.scrollback_expanded.insert(index);
        }
    }

    /// Drop the active pane's buffered scrollback (the live on-screen grid is
    /// untouched — this is "clear history," not the shell's own `clear`).
    pub fn clear_active_scrollback(&mut self) {
        if let Some(term) = self.active_term() {
            term.screen.lock().clear_scrollback();
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
            for (_, term) in tab.panes.iter() {
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

    /// The active tab's focused pane terminal.
    pub fn active_term(&self) -> Option<&Term> {
        self.tabs.get(self.active).and_then(Tab::focused)
    }

    /// The `Tab` a window hosts: the main window shows the active tab; a detached window
    /// shows its own tab. The linchpin for routing window-tagged pane messages.
    pub fn tab_for(&self, window: iced::window::Id) -> Option<&Tab> {
        if self.main_window == Some(window) {
            self.tabs.get(self.active)
        } else {
            self.detached.get(&window)
        }
    }

    /// Mutable [`tab_for`](Self::tab_for).
    pub fn tab_for_mut(&mut self, window: iced::window::Id) -> Option<&mut Tab> {
        if self.main_window == Some(window) {
            self.tabs.get_mut(self.active)
        } else {
            self.detached.get_mut(&window)
        }
    }

    /// The window the keyboard should act on: the focused window, else the main window.
    pub fn keyboard_window(&self) -> Option<iced::window::Id> {
        self.focused_window.or(self.main_window)
    }

    /// The DEC application-cursor-keys mode of `window`'s focused pane (affects arrow
    /// bytes).
    pub fn app_cursor_for(&self, window: iced::window::Id) -> bool {
        self.tab_for(window)
            .and_then(Tab::focused)
            .map(|t| t.screen.lock().app_cursor_keys)
            .unwrap_or(false)
    }

    /// Make tab `idx` active and clear the unseen-activity dot on all its panes (the
    /// whole tab — every pane — becomes visible).
    pub fn activate(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get_mut(idx) {
            self.active = idx;
            for (_, term) in tab.panes.iter_mut() {
                term.activity = false;
            }
        }
    }

    /// Spawn a shell in a new tab and make it active. The new shell starts in the
    /// active pane's reported working directory (OSC 7) when known.
    pub fn new_tab(&mut self) {
        let cwd = self.active_term().and_then(|t| t.screen.lock().cwd.clone());
        if let Some(term) = spawn_term(80, 24, cwd.as_deref(), self.settings.max_scrollback()) {
            self.tabs.push(Tab::new(term));
            self.active = self.tabs.len() - 1;
        }
    }

    /// Split `window`'s focused pane toward `dir`, spawning a fresh shell there (seeded
    /// with the focused pane's cwd) and focusing it. Left/Right split the column (vertical
    /// divider); Up/Down split the row (horizontal divider).
    pub fn split_focused(&mut self, window: iced::window::Id, dir: pane_grid::Direction) {
        let cwd = self
            .tab_for(window)
            .and_then(Tab::focused)
            .and_then(|t| t.screen.lock().cwd.clone());
        if let Some(term) = spawn_term(80, 24, cwd.as_deref(), self.settings.max_scrollback()) {
            self.split_with(window, dir, term);
        }
    }

    /// Place `term` as a new pane split off `window`'s focused pane toward `dir`, and
    /// focus it. (The spawn-free core of [`split_focused`], so tests can inject a pty-less
    /// pane.)
    pub fn split_with(&mut self, window: iced::window::Id, dir: pane_grid::Direction, term: Term) {
        use pane_grid::{Axis, Direction};
        let axis = match dir {
            Direction::Left | Direction::Right => Axis::Vertical,
            Direction::Up | Direction::Down => Axis::Horizontal,
        };
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some((new_pane, _split)) = tab.panes.split(axis, tab.focus, term) {
                // `split` always places the newcomer after the target (right/below); for
                // Left/Up, swap so the new shell lands on the requested side.
                if matches!(dir, Direction::Left | Direction::Up) {
                    tab.panes.swap(tab.focus, new_pane);
                }
                tab.focus = new_pane;
            }
        }
    }

    /// Move focus to the neighbouring pane in `dir` within `window`'s tab (no-op at the
    /// edge).
    pub fn focus_dir(&mut self, window: iced::window::Id, dir: pane_grid::Direction) {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some(p) = tab.panes.adjacent(tab.focus, dir) {
                tab.focus = p;
            }
        }
    }

    /// Focus a specific pane in `window`'s tab (a click landed on it).
    pub fn focus_pane(&mut self, window: iced::window::Id, pane: pane_grid::Pane) {
        if let Some(tab) = self.tab_for_mut(window) {
            tab.focus = pane;
        }
    }

    /// Drag-resize the divider at `split` to `ratio` (0..=1) in `window`'s tab.
    pub fn resize_split(&mut self, window: iced::window::Id, split: pane_grid::Split, ratio: f32) {
        if let Some(tab) = self.tab_for_mut(window) {
            tab.panes.resize(split, ratio);
        }
    }

    /// Close the active tab's focused pane. Closing the last pane in a tab closes the
    /// tab; closing the last tab returns `false` (the caller exits). The dropped `Term`
    /// drops its `PtySession`, ending that shell.
    pub fn close_focused_pane(&mut self) -> bool {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if let Some((_term, sibling)) = tab.panes.close(tab.focus) {
                tab.focus = sibling;
                return true;
            }
        }
        // It was the tab's only pane → close the whole tab.
        self.close_tab(self.active)
    }

    /// Paste `text` into `window`'s focused shell, wrapping it in bracketed-paste markers
    /// when the app enabled mode 2004 (so a compliant shell holds embedded newlines as
    /// literal text in one edit buffer instead of auto-executing each line).
    ///
    /// Without that (`bracketed` false), the destination can't tell paste apart from
    /// typing — every embedded newline runs immediately as its own command, exactly
    /// like a real Enter, just not through one. So each complete pasted line queues its
    /// own Scrollback History boundary *before* any of it is sent, using the
    /// already-known line text (there's nothing to read off the terminal grid yet — see
    /// `TerminalScreen::mark_command_boundary_with`). A final line with no trailing
    /// newline is left alone: it hasn't been "entered," the same as normal typing.
    pub fn paste(&mut self, window: iced::window::Id, text: &str) {
        let bracketed = self
            .tab_for(window)
            .and_then(Tab::focused)
            .map(|t| t.screen.lock().bracketed_paste)
            .unwrap_or(false);

        if !bracketed {
            if let Some(term) = self.tab_for(window).and_then(Tab::focused) {
                let mut screen = term.screen.lock();
                let mut lines: Vec<&str> = text.split('\n').collect();
                if !text.ends_with('\n') {
                    lines.pop(); // an unterminated final fragment — nothing to mark yet
                }
                for line in lines {
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    if line.is_empty() {
                        continue;
                    }
                    let cap = self.settings.resolve_output_cap(line);
                    screen.mark_command_boundary_with(line.to_string(), cap);
                }
            }
        }

        let mut bytes = Vec::new();
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        self.write_focused(window, &bytes);
    }

    /// Per-redraw housekeeping: light activity dots on background tabs that produced
    /// output or rang the bell, clear the active tab's dot, and surface any OSC 52
    /// clipboard-write request for the host to put on the system clipboard. Walks both
    /// the main strip and the detached windows (a detached tab is always on-screen in its
    /// own window, so it never carries a dot).
    pub fn drain_effects(&mut self) -> Option<String> {
        let active = self.active;
        let mut clip = None;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            for (_, term) in tab.panes.iter_mut() {
                let (signal, requested) = drain_pane(term);
                if let Some(c) = requested {
                    clip = Some(c);
                }
                // Every pane of the active tab is on screen, so it never carries a dot;
                // a background tab's panes light one on output or a bell.
                if i == active {
                    term.activity = false;
                } else if signal {
                    term.activity = true;
                }
            }
        }
        for tab in self.detached.values_mut() {
            for (_, term) in tab.panes.iter_mut() {
                let (_signal, requested) = drain_pane(term);
                if let Some(c) = requested {
                    clip = Some(c);
                }
                term.activity = false;
            }
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

    /// Forward `bytes` to a specific pane's shell in `window`'s tab (mouse reporting
    /// targets the pane under the cursor).
    pub fn write_pane(&mut self, window: iced::window::Id, pane: pane_grid::Pane, bytes: &[u8]) {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some(term) = tab.panes.get_mut(pane) {
                if let Some(pty) = term.pty.as_mut() {
                    if let Err(e) = pty.write_bytes(bytes) {
                        tracing::warn!("PTY write failed: {e}");
                    }
                }
            }
        }
    }

    /// Forward `bytes` to `window`'s focused pane (keyboard / paste).
    pub fn write_focused(&mut self, window: iced::window::Id, bytes: &[u8]) {
        if let Some(focus) = self.tab_for(window).map(|t| t.focus) {
            self.write_pane(window, focus, bytes);
        }
    }

    /// Resize one pane's grid + PTY (SIGWINCH) to what its widget reports fits, in
    /// `window`'s tab.
    pub fn resize_pane(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        cols: usize,
        rows: usize,
    ) {
        if let Some(tab) = self.tab_for(window) {
            if let Some(term) = tab.panes.get(pane) {
                term.screen.lock().resize(cols, rows);
                if let Some(pty) = term.pty.as_ref() {
                    let _ = pty.resize(cols as u16, rows as u16);
                }
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

    /// Drop panes whose shell has exited, then any tab left with no live pane — across
    /// both the main strip and the detached windows. Returns `(any_tabs_remain, windows
    /// to close)`: the caller closes each dead detached window's OS window, and exits when
    /// no tabs remain anywhere. Keeps focus + active valid.
    pub fn reap_dead(&mut self) -> (bool, Vec<iced::window::Id>) {
        let active_alive = self.tabs.get(self.active).is_some_and(Tab::has_live_pane);
        for tab in self.tabs.iter_mut() {
            reap_tab_panes(tab);
        }
        self.tabs.retain(Tab::has_live_pane);

        // Detached tabs: reap their panes, then collect windows whose tab fully died.
        let mut dead_windows = Vec::new();
        for (win, tab) in self.detached.iter_mut() {
            reap_tab_panes(tab);
            if !tab.has_live_pane() {
                dead_windows.push(*win);
            }
        }
        for win in &dead_windows {
            self.detached.remove(win);
            self.detach_origin.remove(win);
            self.window_bounds.remove(win);
        }

        let any = !self.tabs.is_empty() || !self.detached.is_empty();
        // If the active tab died, fall back to the last; otherwise just clamp.
        if !self.tabs.is_empty() && (!active_alive || self.active >= self.tabs.len()) {
            self.active = self.tabs.len() - 1;
        }
        (any, dead_windows)
    }

    // ---- detach / reattach (ADR 0003) ----

    /// Detach the main strip's tab `idx` into its own OS window. The owned `Tab` moves
    /// into `detached`; if that would empty the main strip, a fresh shell tab is spawned
    /// so the main window is never empty. Returns the task that opens the window (and
    /// fetches both windows' positions to align the drag-dock band).
    pub fn detach_tab(&mut self, idx: usize) -> Option<iced::Task<Message>> {
        self.menu = None;
        if idx >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.new_tab();
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        let size = iced::Size::new(720.0, 600.0);
        let (id, open) = iced::window::open(iced::window::Settings {
            size,
            ..Default::default()
        });
        self.detached.insert(id, tab);
        self.detach_origin.insert(id, idx);
        crate::detach_drag::on_opened(self, id, size);
        let open = open.then(move |id| {
            iced::window::position(id).map(move |p| Message::WindowPosition(id, p))
        });
        match self.main_window {
            Some(main) => Some(iced::Task::batch([
                open,
                iced::window::position(main).map(move |p| Message::WindowPosition(main, p)),
            ])),
            None => Some(open),
        }
    }

    /// Dock a detached window's tab back into the main strip at its origin index.
    pub fn reattach_window(&mut self, window: iced::window::Id) {
        if let Some(tab) = self.detached.remove(&window) {
            let at = self
                .detach_origin
                .remove(&window)
                .unwrap_or(usize::MAX)
                .min(self.tabs.len());
            self.tabs.insert(at, tab);
            self.active = at;
            self.window_bounds.remove(&window);
        }
    }

    /// While a tab tear-off is armed, dragging the pointer over a *different* tab
    /// live-reorders the dragged tab to that slot (browser-style). The drag anchor
    /// follows so successive crossings keep moving it; no-op when not dragging.
    pub fn reorder_dragged_tab(&mut self, target: usize) {
        let Some((from, start)) = self.tab_drag else {
            return;
        };
        if from == target || from >= self.tabs.len() || target >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(target, tab);
        self.active = target;
        self.tab_drag = Some((target, start));
    }

    /// Complete an armed tab tear-off on pointer release: a drag past
    /// [`TAB_TEAR_THRESHOLD`] detaches the pressed tab; a short drag is just a click.
    pub fn finish_tab_drag(&mut self) -> Option<iced::Task<Message>> {
        let (idx, start) = self.tab_drag.take()?;
        if self.pointer.y - start.y > TAB_TEAR_THRESHOLD {
            self.detach_tab(idx)
        } else {
            None
        }
    }

    /// Record which window has the keyboard (chords/typing route to its tab).
    pub fn focus_window(&mut self, window: iced::window::Id) {
        self.focused_window = Some(window);
    }

    /// Close the focused pane of a detached `window`. Returns `Some(window)` to close
    /// when its last pane went — the tab is removed from `detached` *first*, so the
    /// ensuing `WindowClosed` no-ops instead of reattaching (⌘W through the last pane
    /// kills the window; an OS-close reattaches). `None` means the pane closed in place.
    pub fn close_detached_focused_pane(
        &mut self,
        window: iced::window::Id,
    ) -> Option<iced::window::Id> {
        let tab = self.detached.get_mut(&window)?;
        if let Some((_term, sibling)) = tab.panes.close(tab.focus) {
            tab.focus = sibling;
            return None;
        }
        self.detached.remove(&window);
        self.detach_origin.remove(&window);
        self.window_bounds.remove(&window);
        Some(window)
    }
}

impl Default for Tty {
    fn default() -> Self {
        Self::new()
    }
}

/// Take a pane's pending bell + OSC 52 clipboard request and clear its dirty flag.
/// Returns `(produced_signal, clipboard_request)` — `produced_signal` is true if the
/// pane wrote output or rang the bell (drives the background-activity dot).
fn drain_pane(term: &mut Term) -> (bool, Option<String>) {
    let (bell, requested) = {
        let mut s = term.screen.lock();
        (s.take_bell(), s.take_clipboard())
    };
    let was_dirty = term.dirty.swap(false, Ordering::Relaxed);
    (was_dirty || bell, requested)
}

/// Close every pane in `tab` whose shell has exited (one at a time; `close` is a no-op
/// on the last pane, so an all-dead tab keeps a single dead pane for the caller's
/// `retain`/`has_live_pane` check). Re-points focus at a survivor if it was reaped.
fn reap_tab_panes(tab: &mut Tab) {
    loop {
        let dead = tab
            .panes
            .iter()
            .find(|(_, t)| !t.alive.load(Ordering::Relaxed))
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
/// configured cap (from settings) for this terminal's scrollback buffer.
fn spawn_term(cols: u16, rows: u16, cwd: Option<&str>, max_scrollback: usize) -> Option<Term> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let dir = cwd.map(std::path::Path::new);
    let (session, mut rx) = match PtySession::spawn_in(&shell, cols, rows, dir) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to spawn shell {shell:?}: {e}");
            return None;
        }
    };
    let screen = Arc::new(Mutex::new(TerminalScreen::with_scrollback(
        cols as usize,
        rows as usize,
        max_scrollback,
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
