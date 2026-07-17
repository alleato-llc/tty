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

use crate::history;
use crate::settings::Settings;
use crate::theme::Theme;

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

/// What a single pane holds. A pane is usually a terminal, but a metric drill-in
/// can be "graduated" from a floating popover into a real pane (see
/// [`Tty::promote_metric_to_pane`]). The enum is the extension point: new
/// non-terminal pane kinds slot in here without touching the split/resize/focus
/// machinery, which is generic over the pane content.
pub enum Pane {
    /// A shell terminal (the common case) — owns the PTY + screen.
    Term(Term),
    /// A live metric view (CPU chart, process table, …), keyed by its kind. It has
    /// no PTY and never reaps; its data is read from the shared `Metrics`.
    Metric(crate::settings::MetricKind),
}

impl Pane {
    /// The terminal this pane holds, or `None` for a non-terminal pane. Terminal
    /// operations (keystrokes, resize, reaping) filter through this.
    pub fn as_term(&self) -> Option<&Term> {
        match self {
            Pane::Term(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_term_mut(&mut self) -> Option<&mut Term> {
        match self {
            Pane::Term(t) => Some(t),
            _ => None,
        }
    }
}

/// One tab: a tree of panes (a single pane until the user splits) plus which pane
/// currently has focus. The split tree, drag-to-resize dividers, and cardinal
/// navigation are owned by iced's `pane_grid::State`; we hold a [`Pane`] per slot
/// (a terminal, or a graduated metric view).
pub struct Tab {
    pub panes: pane_grid::State<Pane>,
    pub focus: pane_grid::Pane,
    /// A user-set name (via "Rename tab"). When `None`, the label comes from the focused
    /// pane's program title (OSC 0/2) or shell name.
    pub title: Option<String>,
    /// An untracked ("incognito") tab: its panes' commands are never written
    /// to encrypted history. This mirror of the screens'
    /// `TerminalScreen::untracked` flag exists so the chrome (tab strip,
    /// window title, status bar) can render markers without locking a
    /// screen; the suppression itself lives in cathode, at the source.
    pub untracked: bool,
}

impl Tab {
    /// A new tab wrapping a single shell pane.
    pub fn new(term: Term) -> Self {
        let (panes, focus) = pane_grid::State::new(Pane::Term(term));
        Self {
            panes,
            focus,
            title: None,
            untracked: false,
        }
    }

    /// The focused pane's terminal, or `None` when the focused pane is not a
    /// terminal (e.g. a metric pane).
    pub fn focused(&self) -> Option<&Term> {
        self.panes.get(self.focus).and_then(Pane::as_term)
    }

    /// The terminals in this tab (skipping any non-terminal panes).
    pub fn terms(&self) -> impl Iterator<Item = &Term> {
        self.panes.iter().filter_map(|(_, p)| p.as_term())
    }

    /// The terminals in this tab, mutably (skipping any non-terminal panes).
    pub fn terms_mut(&mut self) -> impl Iterator<Item = &mut Term> {
        self.panes.iter_mut().filter_map(|(_, p)| p.as_term_mut())
    }

    /// The tab's display label: a user-set name wins, else the focused pane's program
    /// title (OSC 0/2) or shell name, else (for a metric pane) the metric's name.
    pub fn label(&self) -> String {
        if let Some(name) = &self.title {
            return name.clone();
        }
        match self.panes.get(self.focus) {
            Some(Pane::Term(term)) => term
                .screen
                .lock()
                .title
                .clone()
                .unwrap_or_else(|| term.title.clone()),
            Some(Pane::Metric(kind)) => kind.to_string(),
            None => String::new(),
        }
    }

    /// Whether any pane in this tab keeps it open: a live shell, or a metric pane
    /// (which never exits on its own).
    fn has_live_pane(&self) -> bool {
        self.panes.iter().any(|(_, p)| match p {
            Pane::Term(t) => t.alive.load(Ordering::Relaxed),
            Pane::Metric(_) => true,
        })
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
    /// The main window's current width, tracked from resize events. Drives the
    /// status bar's width-shedding (drop the rightmost metric cells before the
    /// bar would overflow); `0.0` (unknown, pre-first-resize) sheds nothing.
    pub window_width: f32,
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
    /// The active sub-tab within the Appearance section (see
    /// `view::APPEARANCE_TABS`), so its groups show one pane at a time rather
    /// than one long list. Persists across section switches (a return to
    /// Appearance lands where it left off).
    pub appearance_tab: usize,
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

    /// The background history writer thread, running only when
    /// `Settings::encrypted_history_enabled()` is on and startup (keychain +
    /// manifest) succeeded — `None` otherwise, meaning the feature is simply
    /// off for this session. `drain_effects` forwards each pane's queued
    /// history events here every `Tick`.
    pub history_writer: Option<history::writer::Writer>,
    /// A second, read-only copy of the cipher/keys, for paging into the
    /// archive from the main thread — see `history::Started::keys`. `None`
    /// exactly when `history_writer` is `None`.
    pub history_read: Option<(history::crypto::Cipher, history::HistoryKeys)>,
    /// Archive entries paged in via [`Self::page_scrollback_older`],
    /// oldest-first, prepended before the live `command_log` in the
    /// Scrollback History panel. Empty until the user pages back; reset when
    /// the panel closes.
    pub scrollback_archived: Vec<cathode::history::PersistedCommandEntry>,
    /// The oldest date already paged in (`None` = nothing paged yet, so the
    /// next "page older" starts from the archive's most recent date).
    pub scrollback_archive_cursor: Option<chrono::NaiveDate>,
    /// Set when `history::start` fails while the feature was expected to be
    /// running (an unreadable/corrupted archive, e.g. a key mismatch) —
    /// surfaces a recovery offer in the History settings section instead of
    /// only a log warning.
    pub history_start_failed: bool,
    /// Whether the "Reset encrypted history" confirmation dialog is open.
    pub confirm_reset_history: bool,
    /// When the encrypted history archive was last successfully unlocked via
    /// `history::reauth` this session (`None` = never, so the next access is
    /// always gated) — session-only, never persisted. See
    /// `Tty::history_reauth_reason`.
    pub last_history_auth: Option<Instant>,
    /// Whether a re-auth prompt is currently in flight — pressing ⌘⇧H again
    /// while the native dialog is up must not stack a second prompt (whose
    /// second success would toggle the panel right back closed).
    pub history_reauth_pending: bool,
    /// Whether the settings History section's read-only archive viewer is
    /// open (behind the same re-auth gate as the panel — it shows the same
    /// protected data).
    pub show_settings_history: bool,
    /// Archive entries paged into the settings viewer, oldest-first —
    /// independent of the panel's `scrollback_archived` so the two surfaces
    /// don't fight over one cursor. Cleared when the viewer or settings close.
    pub settings_history: Vec<cathode::history::PersistedCommandEntry>,
    /// The oldest date paged into the settings viewer (`None` = nothing yet).
    pub settings_history_cursor: Option<chrono::NaiveDate>,
    /// The settings viewer's selected row.
    pub settings_history_selected: Option<usize>,
    /// The settings viewer's vertical scroll offset (pixels).
    pub settings_history_scroll: f32,
    /// The archived entry a "Delete this command?" confirmation dialog is
    /// open for (`None` = no dialog). The panel deletes immediately; the
    /// settings viewer confirms first.
    pub confirm_delete_settings_row: Option<ArchivedTarget>,
    /// Whether an async history start is in flight (startup, enable, or the
    /// post-Reset restart) — guards against double-firing and drives the
    /// "unlocking history key…" status.
    pub history_starting: bool,
    /// The command-id floor applied to every screen (existing and future)
    /// once the archive is open: ids below this were already used by earlier
    /// launches *today*, and day segments upsert by id — a screen minting
    /// them again would overwrite archived entries. See
    /// `TerminalScreen::reserve_command_ids`.
    pub history_id_floor: u32,
    /// Passphrase key source only: the feature is enabled but the archive
    /// hasn't been opened this session (no passphrase entered yet, or the
    /// prompt was dismissed). Locked means *nothing is being recorded* —
    /// the banner and status chip say so.
    pub history_locked: bool,
    /// The passphrase prompt (enable or unlock), when open.
    pub passphrase_prompt: Option<PassphrasePrompt>,
    /// The whole session is untracked (the `history_session_start` setting,
    /// the startup chooser's "Stay untracked", or `--untracked`): every tab
    /// is untracked, nothing is recorded, no key is read — immutable until
    /// relaunch. Turning the history *setting* on mid-session persists for
    /// the next launch but does not un-untrack this one: the startup promise
    /// ("nothing typed this session is saved") holds.
    pub session_untracked: bool,
    /// `session_untracked` came from the `--untracked` CLI flag — shown in
    /// the settings note so the override's origin is clear.
    pub untracked_forced_by_cli: bool,
    /// The startup "Record this session's commands?" chooser is open
    /// (`history_session_start == Ask`).
    pub show_session_start_prompt: bool,
    /// Machine-stats sampler for the status bar (CPU/memory). Idle and empty
    /// unless `settings.status_bar_metrics()` is non-empty; fed by the periodic
    /// `SampleMetrics` tick. See `metrics.rs`.
    pub metrics: crate::metrics::Metrics,
    /// The start index of the visible window of status-bar metric cells: when the
    /// bar is too narrow to hold every cell, scrolling over it slides this window
    /// through the full list (so the shed cells are still reachable). `0` = the
    /// front. Clamped to the scrollable range in `update`.
    pub status_bar_scroll: usize,
    /// Whether the status bar is in live-edit (drag-to-reorder) mode. Entered by
    /// pressing and holding a metric cell; left by Escape or a press on empty bar
    /// space.
    pub status_bar_edit: bool,
    /// A pending press on a metric cell `(config index, when it began)`: a quick
    /// release opens that cell's drill-in, while a hold past
    /// [`crate::settings::Settings::status_bar_edit_hold_secs`] (checked by the
    /// tick) enters edit mode and starts dragging it. `None` when no press is down.
    pub status_metric_press: Option<(usize, std::time::Instant)>,
    /// The config index of the metric cell currently being dragged (edit mode),
    /// and the drop-target config index where the insertion bar shows. The
    /// reorder is committed on release. Both `None` when not dragging.
    pub status_metric_drag: Option<usize>,
    pub status_metric_drop: Option<usize>,
    /// The Processes drill-in's sort column + descending flag, and its table
    /// scroll offset (px). A header click re-sorts; the body scrolls.
    pub proc_sort: (ProcSortColumn, bool),
    pub proc_table_scroll: f32,
    /// The pid whose per-process detail is open within the Processes drill-in, or
    /// `None` to show the process list. Double- or right-clicking a row opens one;
    /// its live sampling lives in [`crate::metrics::Metrics::proc_detail`].
    pub proc_detail_pid: Option<i32>,
    /// The metric drill-in popovers currently open (a click on a status-bar
    /// sparkline opens one), each with its own layout. Empty when none are open.
    /// In the default one-at-a-time mode this holds 0 or 1; with
    /// [`crate::settings::Settings::status_bar_metrics_pinned`] on it can hold
    /// several, stacked and independently placed.
    pub metric_details: Vec<MetricPopover>,
    /// An in-progress popover resize drag: which popover (index into
    /// [`Self::metric_details`]), the pointer where it began, the size then, and
    /// which edge/corner was grabbed (so only the dragged axes change). `None`
    /// when not resizing. Ended by `PointerReleased`.
    pub metric_detail_resize: Option<(usize, iced::Point, (f32, f32), ResizeEdge)>,
    /// An in-progress popover move drag: which popover (index), the pointer where
    /// it began, and its offset then; `None` when not moving. Ended by
    /// `PointerReleased`.
    pub metric_detail_move_drag: Option<(usize, iced::Point, (f32, f32))>,
    /// "Replace a pane" pick mode: the metric awaiting a target pane. When `Some`,
    /// the pane grid dims and the next pane click replaces that pane with this
    /// metric (see [`Self::request_pane_replace`]). `Esc` cancels.
    pub pane_replace_pending: Option<crate::settings::MetricKind>,
    /// A pending confirm before replacing a *live* pane: the window + pane to
    /// replace and the metric to put there. `Some` shows the "end the shell?"
    /// dialog; taken on confirm, cleared on cancel.
    pub pane_replace_confirm: Option<(
        iced::window::Id,
        pane_grid::Pane,
        crate::settings::MetricKind,
    )>,
}

/// One open metric drill-in popover: its metric plus per-popover layout, so
/// several can be pinned at once with independent expand / size / position.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricPopover {
    /// Which metric this popover charts.
    pub kind: crate::settings::MetricKind,
    /// Expanded to a large card (the "+" affordance) vs the compact default.
    pub expanded: bool,
    /// A user-dragged size override `(card_width, chart_height)` in px, or `None`
    /// for the current state's default. Reset on expand toggle.
    pub size: Option<(f32, f32)>,
    /// A user-dragged position offset `(dx, dy)` in px from this popover's anchor
    /// (its cascade slot). `(0, 0)` keeps the default placement.
    pub move_offset: (f32, f32),
}

impl MetricPopover {
    /// A freshly opened popover at the compact default size and position.
    pub fn new(kind: crate::settings::MetricKind) -> Self {
        Self {
            kind,
            expanded: false,
            size: None,
            move_offset: (0.0, 0.0),
        }
    }

    /// This popover's current size `(card_width, chart_height)`: the user's
    /// dragged override if any, else the default for its compact/expanded state
    /// (the expanded default is sized off the window). Shared by the view (to lay
    /// out) and the update loop (to seed a resize drag).
    pub fn effective_size(&self, window_width: f32, window_height: f32) -> (f32, f32) {
        self.size.unwrap_or_else(|| {
            if self.expanded {
                let w = if window_width > 1.0 {
                    (window_width - 96.0).clamp(420.0, 1200.0)
                } else {
                    900.0
                };
                let h = if window_height > 1.0 {
                    (window_height - 240.0).clamp(220.0, 900.0)
                } else {
                    360.0
                };
                (w, h)
            } else {
                (320.0, 150.0)
            }
        })
    }
}

/// A sortable column of the Processes drill-in table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcSortColumn {
    Name,
    Cpu,
    Mem,
}

/// Which edge or corner of the metric popover a resize drag grabbed. The card
/// is anchored at its top-left, so only the right edge, bottom edge, and
/// bottom-right corner resize; each maps to which axes the drag adjusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    /// Drag the right edge: width only.
    Right,
    /// Drag the bottom edge: height only.
    Bottom,
    /// Drag the bottom-right corner: both axes.
    Corner,
}

impl ResizeEdge {
    /// `(adjust_width, adjust_height)` — which axes this grab resizes.
    pub fn axes(self) -> (bool, bool) {
        match self {
            Self::Right => (true, false),
            Self::Bottom => (false, true),
            Self::Corner => (true, true),
        }
    }
}

/// What a launch should do about encrypted history — the pure decision core
/// behind `Tty::new` + `startup_history_task`, kept side-effect-free so the
/// whole matrix is unit-testable. The CLI flag beats everything: a user who
/// typed `--untracked` gets an untracked session regardless of settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPlan {
    /// Feature off: nothing to do.
    Off,
    /// Start recording via the keychain key (async, alongside boot).
    StartKeychain,
    /// Passphrase source: boot locked, unlock prompt open, no crypto yet.
    LockedPassphrase,
    /// Ask the user first (the startup chooser).
    Ask,
    /// The whole session is untracked — no writer, no key read, no seed.
    Untracked,
}

pub fn startup_history_plan(settings: &Settings, cli_untracked: bool) -> StartupPlan {
    if cli_untracked {
        return StartupPlan::Untracked;
    }
    if !settings.encrypted_history_enabled() {
        return StartupPlan::Off;
    }
    match settings.history_session_start() {
        crate::settings::SessionStart::Untracked => StartupPlan::Untracked,
        crate::settings::SessionStart::Ask => StartupPlan::Ask,
        crate::settings::SessionStart::Record => match settings.history_key_source() {
            crate::settings::KeySource::Keychain => StartupPlan::StartKeychain,
            crate::settings::KeySource::Passphrase => StartupPlan::LockedPassphrase,
        },
    }
}

/// Which flow the passphrase prompt is serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassphrasePromptKind {
    /// First enable (or the post-Reset fresh archive): choose a passphrase,
    /// entered twice.
    Enable,
    /// Startup (or reopened from the locked banner): enter the existing
    /// archive's passphrase.
    Unlock,
}

/// The passphrase modal's state. Drafts are `Zeroizing` so dropping the
/// prompt wipes them (best-effort: the text widget keeps its own internal
/// copy — recorded honestly in ADR 0007).
pub struct PassphrasePrompt {
    pub kind: PassphrasePromptKind,
    pub draft: zeroize::Zeroizing<String>,
    pub confirm: zeroize::Zeroizing<String>,
    /// Inline validation/auth error shown under the fields.
    pub error: Option<String>,
    /// A derivation is in flight (Argon2id is deliberately slow) — the
    /// submit button is replaced by a progress label and re-submits are
    /// ignored.
    pub busy: bool,
}

impl PassphrasePrompt {
    pub fn new(kind: PassphrasePromptKind) -> Self {
        Self {
            kind,
            draft: zeroize::Zeroizing::new(String::new()),
            confirm: zeroize::Zeroizing::new(String::new()),
            error: None,
            busy: false,
        }
    }
}

/// Which right-click menu is open — a tab's (split + tab actions), a pane's (split +
/// close pane), a link's (open/copy a detected URL), a Scrollback History row's
/// (copy/clear/delete), or a settings archive-viewer row's (copy, or delete behind
/// a confirmation dialog). The first two target the active tab's focused pane for
/// splits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuKind {
    Tab,
    Pane,
    Link(String),
    ScrollbackRow(HistoryRowTarget),
    /// The full archive address of the row — Copy uses its `command`, Delete
    /// needs the rest to tombstone it on disk.
    SettingsHistoryRow(ArchivedTarget),
    /// A right-clicked row in the Processes drill-in: view its fds, or copy its
    /// path / pid / name. The path is resolved lazily when copied (not held here).
    ProcRow {
        pid: i32,
        name: String,
    },
    /// A right-clicked open-file-descriptor row in a process detail: copy its path.
    FdRow {
        path: String,
    },
    /// The "move to pane" menu for a metric popover — pick a direction to graduate
    /// it into a real split pane.
    PromotePopover {
        kind: crate::settings::MetricKind,
    },
}

/// What a right-clicked Scrollback History row refers to, live (in
/// `command_log`) or paged in from the encrypted archive — the two need
/// different addressing (a live position vs. a stable archive id) and
/// different Clear/Delete plumbing (through `TerminalScreen` vs. straight to
/// the background writer), but share one context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryRowTarget {
    Live(ScrollbackTarget),
    Archived(ArchivedTarget),
}

impl HistoryRowTarget {
    pub fn text(&self) -> &str {
        match self {
            HistoryRowTarget::Live(t) => t.text(),
            HistoryRowTarget::Archived(t) => t.text(),
        }
    }
}

/// What a right-clicked Scrollback History row refers to — a command's header row
/// or one of its output lines — carrying both the resolved text (for "Copy") and
/// enough to locate it in `command_log` (for "Clear"). `log_index` is the row's
/// index into `TerminalScreen::command_log` at the time of the click, not the
/// *filtered* row index the panel renders (those shift as the filter changes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollbackTarget {
    Command {
        log_index: usize,
        text: String,
    },
    Output {
        log_index: usize,
        line: usize,
        text: String,
    },
}

/// A right-clicked row that was paged in from the encrypted archive, not
/// `command_log` — there is no in-memory `CommandEntry` behind it, so
/// Clear/Delete go straight to the background writer instead of through
/// `TerminalScreen`'s index-based methods. Carries everything needed to
/// reconstruct the right `HistoryEvent` for either action: `id` + `date` (+
/// its exact `started_at_epoch_ms`, which the writer uses to re-derive the
/// same local date `date` already names) address it, `pane_tag` is preserved
/// through a Clear (which only blanks `command`, mirroring the live path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedTarget {
    pub date: chrono::NaiveDate,
    pub id: u32,
    pub started_at_epoch_ms: u64,
    pub pane_tag: String,
    pub command: String,
}

impl ArchivedTarget {
    pub fn text(&self) -> &str {
        &self.command
    }
}

impl ScrollbackTarget {
    pub fn text(&self) -> &str {
        match self {
            ScrollbackTarget::Command { text, .. } => text,
            ScrollbackTarget::Output { text, .. } => text,
        }
    }
}

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
