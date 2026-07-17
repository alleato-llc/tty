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
use crate::message::Message;
use crate::settings::Settings;
use crate::theme::Theme;

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

/// One tab: a tree of terminal panes (a single pane until the user splits) plus which
/// pane currently has focus. The split tree, drag-to-resize dividers, and cardinal
/// navigation are owned by iced's `pane_grid::State`; we just hold a `Term` per pane.
pub struct Tab {
    pub panes: pane_grid::State<Term>,
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
        let (panes, focus) = pane_grid::State::new(term);
        Self {
            panes,
            focus,
            title: None,
            untracked: false,
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
    /// Whether the status bar is in live-edit (drag-to-reorder) mode, and an
    /// in-progress metric-cell drag `(config index, pointer anchor)` — mirrors
    /// [`Self::tab_drag`]. Edit mode is entered by a long right-press on the bar
    /// and left by Escape or a click on empty bar space.
    pub status_bar_edit: bool,
    pub status_metric_drag: Option<(usize, iced::Point)>,
    /// When a right-press on the bar armed the long-press-to-edit gesture (the
    /// instant it began); `None` when not armed. A tick checks the elapsed hold
    /// against [`crate::settings::Settings::status_bar_edit_hold_secs`].
    pub status_bar_edit_arm: Option<std::time::Instant>,
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
            status_metric_drag: None,
            status_bar_edit_arm: None,
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

    /// The settings toggle, ON direction: open the one enable dialog. It
    /// carries every fixed-at-enable choice (key source, KDF, cipher) plus
    /// the passphrase fields or the OS-keychain explainer, depending on the
    /// source picked *in the dialog* — nothing touches the keychain or
    /// derives anything until the user confirms there, and the setting
    /// itself commits only when the async start succeeds. A no-op while a
    /// start is already in flight or the feature is already on.
    pub fn request_enable_encrypted_history(&mut self) {
        if self.history_starting {
            tracing::info!("encrypted history: enable ignored — a start is already in flight");
            return;
        }
        if self.settings.encrypted_history_enabled() {
            tracing::info!("encrypted history: enable ignored — already enabled");
            return;
        }
        // An untracked session keeps its promise: the setting persists (for
        // the next launch), but nothing starts recording *this* session —
        // the History section says so.
        if self.session_untracked {
            tracing::info!("encrypted history: enable persisted; session untracked until relaunch");
            self.settings.encrypted_history_enabled = Some(true);
            self.settings.save();
            return;
        }
        tracing::info!("encrypted history: enable requested — opening the enable dialog");
        self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Enable));
    }

    /// The settings toggle, OFF direction: stop the writer for this session.
    /// Never deletes the archive (that's the separate, confirmed Reset).
    pub fn disable_encrypted_history(&mut self) {
        self.history_writer = None;
        self.history_read = None;
        self.settings.encrypted_history_enabled = Some(false);
        self.history_start_failed = false;
        self.history_locked = false;
        self.passphrase_prompt = None;
        // Fail toward requiring a fresh check rather than trusting a stale
        // one — a later re-enable gets a new archive underneath it.
        self.last_history_auth = None;
        self.settings.save();
    }

    /// Pick the history key source (persisted). Like the cipher, only takes
    /// effect the next time the feature starts fresh.
    pub fn set_history_key_source(&mut self, source: String) {
        self.settings.history_key_source = Some(source);
        self.settings.save();
    }

    /// Pick the launch behavior (persisted; takes effect next launch —
    /// including this-session-untracked, which stays untracked either way).
    pub fn set_history_session_start(&mut self, mode: String) {
        self.settings.history_session_start = Some(mode);
        self.settings.save();
    }

    /// Pick the passphrase KDF (persisted). New archives only — an existing
    /// archive keeps its sidecar's recorded recipe.
    pub fn set_history_kdf(&mut self, kdf: String) {
        self.settings.history_kdf = Some(kdf);
        self.settings.save();
    }

    /// Pick the fan-out PRF (persisted). Fixed at enable like the cipher; an
    /// existing archive must decrypt under the same choice or a Reset is
    /// required.
    pub fn set_history_fanout(&mut self, fanout: String) {
        self.settings.history_fanout = Some(fanout);
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

    /// Add a metric to the status bar's ordered list (persisted). The first
    /// metric added takes a sample immediately so numbers appear without waiting
    /// for the next tick (that sample is the CPU% baseline; a real percentage
    /// lands one tick later).
    pub fn add_status_bar_metric(&mut self, metric: &str) {
        let was_empty = self.settings.status_bar_metrics().is_empty();
        if self.settings.add_status_bar_metric(metric) {
            self.settings.save();
            if was_empty {
                self.metrics.sample();
            }
        }
    }

    /// Remove the status-bar metric at `idx` (persisted). Emptying the list
    /// resets the sampler so its history starts clean if re-enabled.
    pub fn remove_status_bar_metric(&mut self, idx: usize) {
        self.settings.remove_status_bar_metric(idx);
        self.settings.save();
        if self.settings.status_bar_metrics().is_empty() {
            self.metrics = crate::metrics::Metrics::default();
        }
    }

    /// Reorder the status-bar metric at `idx` by `delta` (persisted).
    pub fn move_status_bar_metric(&mut self, idx: usize, delta: i32) {
        self.settings.move_status_bar_metric(idx, delta);
        self.settings.save();
    }

    /// Set the render style of the status-bar metric at `idx` (persisted).
    pub fn set_status_bar_metric_style(&mut self, idx: usize, style: &str) {
        self.settings.set_status_bar_metric_style(idx, style);
        self.settings.save();
    }

    /// Nudge a graded metric's caution/alarm threshold (persisted).
    pub fn step_status_bar_metric_threshold(&mut self, idx: usize, warn: bool, delta: f64) {
        self.settings
            .step_status_bar_metric_threshold(idx, warn, delta);
        self.settings.save();
    }

    /// Nudge the edit-mode long-press hold duration by `delta` seconds
    /// (persisted, clamped to the allowed range).
    pub fn step_status_bar_edit_hold(&mut self, delta: f32) {
        let next = (self.settings.status_bar_edit_hold_secs() + delta).clamp(
            crate::settings::MIN_EDIT_HOLD_SECS,
            crate::settings::MAX_EDIT_HOLD_SECS,
        );
        self.settings.status_bar_edit_hold_secs = Some(next);
        self.settings.save();
    }

    /// Arm the long-press-to-edit gesture (a right-press landed on the bar).
    /// No-op once already editing.
    pub fn arm_status_bar_edit(&mut self) {
        if !self.status_bar_edit {
            self.status_bar_edit_arm = Some(std::time::Instant::now());
        }
    }

    /// Cancel an armed long-press (the right button was released before the hold
    /// completed). Leaves an active edit session alone.
    pub fn disarm_status_bar_edit(&mut self) {
        self.status_bar_edit_arm = None;
    }

    /// If the long-press has been held for the configured duration, enter edit
    /// mode. Called from the periodic tick while armed.
    pub fn check_status_bar_edit_hold(&mut self) {
        if let Some(started) = self.status_bar_edit_arm {
            let hold = self.settings.status_bar_edit_hold_secs();
            if started.elapsed().as_secs_f32() >= hold {
                self.status_bar_edit = true;
                self.status_bar_edit_arm = None;
            }
        }
    }

    /// Leave drag-to-reorder edit mode (Escape or a click on empty bar space).
    pub fn exit_status_bar_edit(&mut self) {
        self.status_bar_edit = false;
        self.status_bar_edit_arm = None;
        self.status_metric_drag = None;
    }

    /// Begin dragging the status-bar metric at config index `idx` (edit mode).
    pub fn start_status_metric_drag(&mut self, idx: usize) {
        if self.status_bar_edit {
            self.status_metric_drag = Some((idx, self.pointer));
        }
    }

    /// While a metric drag is armed, moving the pointer over the cell at config
    /// index `target` live-reorders the dragged metric to that slot (persisted),
    /// mirroring [`Self::reorder_dragged_tab`]. No-op when not dragging.
    pub fn reorder_dragged_metric(&mut self, target: usize) {
        let Some((from, start)) = self.status_metric_drag else {
            return;
        };
        let len = self.settings.status_bar_metrics.len();
        if from == target || from >= len || target >= len {
            return;
        }
        let item = self.settings.status_bar_metrics.remove(from);
        self.settings.status_bar_metrics.insert(target, item);
        self.status_metric_drag = Some((target, start));
        self.settings.save();
    }

    /// End a metric drag on pointer release (the reorder already happened live).
    pub fn finish_status_metric_drag(&mut self) {
        self.status_metric_drag = None;
    }

    /// Whether the floating (auto-hide) status bar should show right now: the
    /// pointer sits within [`STATUS_BAR_REVEAL_ZONE`] of the window's bottom
    /// edge. Only consulted when `settings.status_bar_autohide()` is on.
    pub fn status_bar_revealed(&self) -> bool {
        self.window_height > 0.0 && self.pointer.y >= self.window_height - STATUS_BAR_REVEAL_ZONE
    }

    /// Reopen the passphrase unlock prompt from the locked banner (after an
    /// earlier dismiss this session).
    pub fn open_history_unlock(&mut self) {
        if self.history_locked && !self.history_starting && self.passphrase_prompt.is_none() {
            self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));
        }
    }

    /// The passphrase prompt's main field changed.
    pub fn set_passphrase_draft(&mut self, text: String) {
        if let Some(prompt) = self.passphrase_prompt.as_mut() {
            if !prompt.busy {
                *prompt.draft = text;
            }
        }
    }

    /// The passphrase prompt's confirm field changed (enable flow).
    pub fn set_passphrase_confirm(&mut self, text: String) {
        if let Some(prompt) = self.passphrase_prompt.as_mut() {
            if !prompt.busy {
                *prompt.confirm = text;
            }
        }
    }

    /// Dismiss the passphrase prompt (its `Zeroizing` drafts wipe on drop).
    /// Enable flow: the setting stays off. Unlock flow: history stays locked
    /// for the session; the banner's "Unlock…" reopens it.
    pub fn cancel_passphrase_prompt(&mut self) {
        self.passphrase_prompt = None;
    }

    /// Submit the passphrase prompt: validate inline (length; the enable
    /// flow's two entries matching), then derive the key + start on a
    /// background thread — Argon2id is deliberately slow and must not run on
    /// the UI thread. The result lands in [`Self::apply_history_started`]
    /// with `WrongPassphrase` mapped from `Error::AuthFailed`.
    pub fn submit_passphrase(&mut self) -> iced::Task<Message> {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome, StartedHandle};

        let Some(prompt) = self.passphrase_prompt.as_mut() else {
            return iced::Task::none();
        };
        if prompt.busy {
            return iced::Task::none();
        }
        if prompt.draft.chars().count() < history::passphrase::MIN_PASSPHRASE_LEN {
            prompt.error = Some(format!(
                "At least {} characters.",
                history::passphrase::MIN_PASSPHRASE_LEN
            ));
            return iced::Task::none();
        }
        if prompt.kind == PassphrasePromptKind::Enable && *prompt.draft != *prompt.confirm {
            prompt.error = Some("The two entries don't match.".to_string());
            return iced::Task::none();
        }
        prompt.error = None;
        prompt.busy = true;
        let origin = match prompt.kind {
            PassphrasePromptKind::Enable => HistoryStartOrigin::Enable,
            PassphrasePromptKind::Unlock => HistoryStartOrigin::Unlock,
        };
        let passphrase = prompt.draft.clone();
        self.history_starting = true;
        let cipher = self.settings.history_cipher();
        let kdf = self.settings.history_kdf();
        let prf = self.settings.history_fanout().resolve(cipher);
        iced::Task::perform(
            history::passphrase::start_async(cipher, kdf, prf, passphrase),
            move |result| {
                let outcome = match result {
                    Ok(started) => HistoryStartOutcome::Ready(StartedHandle::new(started)),
                    Err(history::Error::AuthFailed) => HistoryStartOutcome::WrongPassphrase,
                    Err(e) => {
                        tracing::warn!("encrypted history: passphrase start failed: {e}");
                        HistoryStartOutcome::Failed
                    }
                };
                Message::HistoryStarted(origin, outcome)
            },
        )
    }

    /// Kick off an async history start (the keychain read runs on its own
    /// thread — see `history::start_keychain_async`). The result comes back
    /// as `Message::HistoryStarted` and lands in
    /// [`Self::apply_history_started`].
    pub fn begin_history_start(
        &mut self,
        origin: crate::message::HistoryStartOrigin,
    ) -> iced::Task<Message> {
        use crate::message::{HistoryStartOutcome, StartedHandle};
        self.history_starting = true;
        let cipher = self.settings.history_cipher();
        let prf = self.settings.history_fanout().resolve(cipher);
        iced::Task::perform(history::start_keychain_async(cipher, prf), move |result| {
            let outcome = match result {
                Some(started) => HistoryStartOutcome::Ready(StartedHandle::new(started)),
                None => HistoryStartOutcome::Failed,
            };
            Message::HistoryStarted(origin, outcome)
        })
    }

    /// Apply a finished async history start. Success installs the writer +
    /// read key, raises the command-id floor on every live screen (ids below
    /// it belong to entries already archived today — see
    /// `TerminalScreen::reserve_command_ids`), and seeds the active pane's
    /// live log *only if it's still empty*: commands typed before the
    /// archive opened are not retro-recorded, and appending yesterday's
    /// entries after today's would scramble the panel's ordering. Failure
    /// keeps the honest long-standing semantics: an enable failure reverts
    /// the setting to off (never "on but broken"); a startup or post-Reset
    /// failure keeps the setting and shows the red banner.
    pub fn apply_history_started(
        &mut self,
        origin: crate::message::HistoryStartOrigin,
        outcome: crate::message::HistoryStartOutcome,
    ) {
        use crate::message::{HistoryStartOrigin, HistoryStartOutcome};
        self.history_starting = false;
        // Either branch gets a new key/archive underneath it (or none at
        // all) — require a fresh re-auth check rather than trusting a stale
        // one from before the (re)start.
        self.last_history_auth = None;

        match outcome {
            HistoryStartOutcome::Ready(handle) => {
                // The user can flip the toggle off while the start is in
                // flight (it isn't blocked on it). Honor that: drop the
                // handle — the writer thread exits with it. Enable is the
                // exception: the setting is deliberately still off until
                // this very moment. An untracked session never installs a
                // writer, full stop (belt-and-braces — no start should be
                // in flight in one).
                if self.session_untracked
                    || (origin != HistoryStartOrigin::Enable
                        && !self.settings.encrypted_history_enabled())
                {
                    return;
                }
                let Some(started) = handle.take() else {
                    return;
                };
                self.history_writer = Some(started.writer);
                self.history_read = Some((started.cipher, started.keys));
                self.history_start_failed = false;
                self.history_locked = false;
                self.passphrase_prompt = None;
                if origin == HistoryStartOrigin::Enable {
                    self.settings.encrypted_history_enabled = Some(true);
                    self.settings.save();
                }

                let floor = started.seed.iter().map(|e| e.id + 1).max().unwrap_or(0);
                self.history_id_floor = self.history_id_floor.max(floor);
                self.reserve_command_ids_everywhere();

                if !started.seed.is_empty() {
                    if let Some(term) = self.active_term() {
                        let mut screen = term.screen.lock();
                        if screen.command_log.is_empty() {
                            screen.seed_command_log(started.seed);
                        } else {
                            tracing::info!(
                                "encrypted history: not seeding — commands ran before the \
                                 archive opened (they are not retro-recorded)"
                            );
                        }
                    }
                }
            }
            HistoryStartOutcome::WrongPassphrase => {
                // Wrong passphrase (or a corrupted archive — deliberately
                // indistinguishable): inline error, retry in place. History
                // stays locked, the setting stays put; this is not the red
                // "broken archive" banner.
                if let Some(prompt) = self.passphrase_prompt.as_mut() {
                    prompt.busy = false;
                    prompt.draft.clear();
                    prompt.confirm.clear();
                    prompt.error = Some(match prompt.kind {
                        PassphrasePromptKind::Unlock => {
                            "Wrong passphrase (or the archive is corrupted). Try again.".into()
                        }
                        // Enabling over an archive keyed differently (e.g.
                        // one created under the keychain source): no retry
                        // can succeed — say what actually helps.
                        PassphrasePromptKind::Enable => {
                            "An existing archive is keyed differently — this passphrase \
                             can't open it. Reset encrypted history to start fresh."
                                .into()
                        }
                    });
                }
            }
            HistoryStartOutcome::Failed => {
                match origin {
                    // An unlock failure that isn't AuthFailed (an unreadable
                    // KDF sidecar, an io error): the archive exists and the
                    // setting stays on — surface it in the prompt, with the
                    // way out.
                    HistoryStartOrigin::Unlock => {
                        if let Some(prompt) = self.passphrase_prompt.as_mut() {
                            prompt.busy = false;
                            prompt.error = Some(
                                "Couldn't open the archive (see the log). \
                                 Reset encrypted history to start fresh."
                                    .into(),
                            );
                        } else {
                            self.history_start_failed = true;
                        }
                    }
                    HistoryStartOrigin::Startup => self.history_start_failed = true,
                    // Enable/post-Reset failures revert the setting — never
                    // "on but broken".
                    HistoryStartOrigin::Enable | HistoryStartOrigin::Reset => {
                        self.history_start_failed = true;
                        self.passphrase_prompt = None;
                        self.history_locked = false;
                        self.settings.encrypted_history_enabled = Some(false);
                        self.settings.save();
                    }
                }
            }
        }
    }

    /// Raise every live screen's command-id counter to the current floor —
    /// every pane of every tab, detached windows included (they persist to
    /// the same archive).
    fn reserve_command_ids_everywhere(&mut self) {
        let floor = self.history_id_floor;
        for tab in &mut self.tabs {
            for (_, term) in tab.panes.iter_mut() {
                term.screen.lock().reserve_command_ids(floor);
            }
        }
        for tab in self.detached.values_mut() {
            for (_, term) in tab.panes.iter_mut() {
                term.screen.lock().reserve_command_ids(floor);
            }
        }
    }

    /// Pick the history cipher (persisted). Only takes effect the next time
    /// the feature starts fresh — see `Message::SetHistoryCipher`.
    pub fn set_history_cipher(&mut self, cipher: String) {
        self.settings.history_cipher = Some(cipher);
        self.settings.save();
    }

    /// Set the re-auth idle interval (clamped, persisted). `0` disables it,
    /// leaving only the once-per-session gate.
    pub fn set_history_reauth_interval_minutes(&mut self, n: u32) {
        self.settings.history_reauth_interval_minutes = Some(n.clamp(
            crate::settings::MIN_HISTORY_REAUTH_INTERVAL_MINUTES,
            crate::settings::MAX_HISTORY_REAUTH_INTERVAL_MINUTES,
        ));
        self.settings.save();
    }

    /// Nudge the re-auth idle interval from the settings stepper.
    pub fn step_history_reauth_interval_minutes(&mut self, delta: i64) {
        let current = i64::from(self.settings.history_reauth_interval_minutes());
        self.set_history_reauth_interval_minutes((current + delta).max(0) as u32);
    }

    /// If opening the Scrollback History panel needs a fresh Touch ID/device-
    /// password check first, the reason text to show in the native prompt —
    /// `None` if it can open immediately (off macOS, no archive active this
    /// session, or the last check is still within the once-per-session/
    /// interval policy — see `history::reauth::is_due`).
    pub fn history_reauth_reason(&self) -> Option<String> {
        if !cfg!(target_os = "macos") || self.history_writer.is_none() {
            return None;
        }
        let interval = match self.settings.history_reauth_interval_minutes() {
            0 => None,
            n => Some(n),
        };
        if history::reauth::is_due(self.last_history_auth, interval, Instant::now()) {
            Some("unlock your encrypted command history".to_string())
        } else {
            None
        }
    }

    /// Record a successful re-auth (called from the `HistoryReauthResult`
    /// handler once the native prompt succeeds).
    pub fn mark_history_authenticated(&mut self) {
        self.last_history_auth = Some(Instant::now());
    }

    /// Open the "Reset encrypted history" confirmation dialog — a distinct,
    /// explicit action from the enable/disable toggle, which never deletes
    /// anything.
    pub fn request_reset_encrypted_history(&mut self) {
        self.confirm_reset_history = true;
    }

    /// Dismiss the reset confirmation without deleting anything.
    pub fn cancel_reset_encrypted_history(&mut self) {
        self.confirm_reset_history = false;
    }

    /// Permanently delete the whole encrypted history archive (every day
    /// segment and the manifest) — the whole directory goes at once, not
    /// just the unreadable parts, so there's no risk of leaving a manifest
    /// and segments out of sync with each other. If the feature is still
    /// enabled, kicks off an async start of a fresh empty archive in its
    /// place so the user isn't left toggling it off and back on themselves.
    pub fn confirm_reset_encrypted_history(&mut self) -> iced::Task<Message> {
        self.confirm_reset_history = false;
        self.history_writer = None;
        self.history_read = None;
        self.scrollback_archived.clear();
        self.scrollback_archive_cursor = None;
        self.history_start_failed = false;
        self.last_history_auth = None;

        if let Err(e) = std::fs::remove_dir_all(history::history_dir()) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("encrypted history: failed to remove the archive directory: {e}");
            }
        }

        if self.settings.encrypted_history_enabled() && !self.session_untracked {
            match self.settings.history_key_source() {
                crate::settings::KeySource::Keychain => {
                    return self.begin_history_start(crate::message::HistoryStartOrigin::Reset);
                }
                // A fresh archive under the passphrase source needs a fresh
                // passphrase (and KDF sidecar) — ask for one instead of
                // starting anything.
                crate::settings::KeySource::Passphrase => {
                    self.history_locked = true;
                    self.passphrase_prompt =
                        Some(PassphrasePrompt::new(PassphrasePromptKind::Enable));
                }
            }
        }
        iced::Task::none()
    }

    /// The boot-time history start, chained by `main` alongside opening the
    /// main window — `Tty::new` itself must never touch the keychain (a
    /// blocked OS dialog there freezes the whole launch). The passphrase
    /// source returns no task: it boots *locked*, with the unlock prompt
    /// already open (see `Tty::new`), and starts only when the user submits.
    pub fn startup_history_task(&mut self) -> iced::Task<Message> {
        if !self.settings.encrypted_history_enabled()
            || self.settings.history_key_source() == crate::settings::KeySource::Passphrase
            || self.session_untracked
            || self.show_session_start_prompt
        {
            return iced::Task::none();
        }
        self.begin_history_start(crate::message::HistoryStartOrigin::Startup)
    }

    /// The startup chooser's answer. Record: begin the start now (keychain)
    /// or open the passphrase unlock prompt (chained, never stacked). Stay
    /// untracked: the whole session goes untracked — see
    /// [`Self::make_session_untracked`].
    pub fn choose_session_start(&mut self, record: bool) -> iced::Task<Message> {
        self.show_session_start_prompt = false;
        if record {
            if self.settings.history_key_source() == crate::settings::KeySource::Passphrase {
                self.history_locked = true;
                self.passphrase_prompt = Some(PassphrasePrompt::new(PassphrasePromptKind::Unlock));
                return iced::Task::none();
            }
            return self.begin_history_start(crate::message::HistoryStartOrigin::Startup);
        }
        self.make_session_untracked();
        iced::Task::none()
    }

    /// Flip the whole session untracked: every existing tab (main strip and
    /// detached) and every pane's screen — future tabs inherit it via
    /// [`Self::new_tab_with`]. Commands typed before this point were never
    /// persisted either: the writer doesn't start until the chooser answers.
    fn make_session_untracked(&mut self) {
        self.session_untracked = true;
        for tab in &mut self.tabs {
            tab.untracked = true;
            for (_, term) in tab.panes.iter_mut() {
                term.screen.lock().set_untracked(true);
            }
        }
        for tab in self.detached.values_mut() {
            tab.untracked = true;
            for (_, term) in tab.panes.iter_mut() {
                term.screen.lock().set_untracked(true);
            }
        }
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

    /// Toggle the scrollback history panel; closing clears its filter so reopening
    /// starts fresh.
    pub fn toggle_scrollback_panel(&mut self) {
        self.show_scrollback = !self.show_scrollback;
        if !self.show_scrollback {
            self.scrollback_query.clear();
            self.scrollback_selected = None;
            self.scrollback_scroll = 0.0;
            self.scrollback_expanded.clear();
            self.scrollback_archived.clear();
            self.scrollback_archive_cursor = None;
        }
    }

    /// Page the Scrollback History panel one day older into the encrypted
    /// archive, prepending that day's entries before whatever's already
    /// paged in. A no-op if the feature is off, or if there's nothing older
    /// (`history::page_older` itself warns and returns `None` on a read
    /// failure — this never panics, it just doesn't add anything that time).
    pub fn page_scrollback_older(&mut self) {
        let Some((_, keys)) = self.history_read.as_ref() else {
            return;
        };
        let Some((date, mut entries)) = history::page_older(keys, self.scrollback_archive_cursor)
        else {
            return;
        };
        entries.append(&mut self.scrollback_archived);
        self.scrollback_archived = entries;
        self.scrollback_archive_cursor = Some(date);
    }

    /// Page the Scrollback History panel one day newer, back toward the
    /// present — the inverse of `page_scrollback_older`. Purely local: drops
    /// the oldest paged-in day's entries from `scrollback_archived` (always
    /// at the front, since entries are oldest-first) and moves the cursor to
    /// whichever day is now oldest among what's left, or `None` if that
    /// empties the list (fully back to the live view). Unlike paging older,
    /// this never touches disk — undoing a page-in only means forgetting
    /// what was already loaded, not fetching anything new.
    pub fn page_scrollback_newer(&mut self) {
        let Some(cursor) = self.scrollback_archive_cursor else {
            return;
        };
        self.scrollback_archived
            .retain(|e| history::local_date_from_epoch_ms(e.started_at_epoch_ms) != cursor);
        self.scrollback_archive_cursor = self
            .scrollback_archived
            .first()
            .map(|e| history::local_date_from_epoch_ms(e.started_at_epoch_ms));
    }

    /// Open the settings History section's read-only archive viewer, loading
    /// the most recent day if nothing is paged in yet. Callers gate this
    /// behind re-auth (see `update`'s `ToggleSettingsHistoryViewer` handler) —
    /// it shows the same protected data as the panel.
    pub fn open_settings_history_viewer(&mut self) {
        self.show_settings_history = true;
        if self.settings_history.is_empty() {
            self.page_settings_history_older();
        }
    }

    /// Close the settings archive viewer and drop everything it paged in —
    /// decrypted history doesn't linger in memory behind a closed view.
    pub fn close_settings_history_viewer(&mut self) {
        self.show_settings_history = false;
        self.settings_history.clear();
        self.settings_history_cursor = None;
        self.settings_history_selected = None;
        self.settings_history_scroll = 0.0;
        self.confirm_delete_settings_row = None;
    }

    /// Open the per-row "Delete this command?" confirmation for a viewer row.
    pub fn request_delete_settings_history_row(&mut self, target: ArchivedTarget) {
        self.close_menu();
        self.confirm_delete_settings_row = Some(target);
    }

    /// Dismiss the per-row delete confirmation without touching anything.
    pub fn cancel_delete_settings_history_row(&mut self) {
        self.confirm_delete_settings_row = None;
    }

    /// The per-row delete confirmation's "Delete" — tombstone the entry via
    /// the background writer ([`Self::delete_archived_target`], which also
    /// drops it from both surfaces' paged-in copies).
    pub fn confirm_delete_settings_history_row(&mut self) {
        if let Some(target) = self.confirm_delete_settings_row.take() {
            self.delete_archived_target(&target);
        }
    }

    /// Page the settings archive viewer one day older — the viewer's own
    /// counterpart of [`Self::page_scrollback_older`], with its own cursor so
    /// the panel and the viewer never fight over one.
    pub fn page_settings_history_older(&mut self) {
        let Some((_, keys)) = self.history_read.as_ref() else {
            return;
        };
        let Some((date, mut entries)) = history::page_older(keys, self.settings_history_cursor)
        else {
            return;
        };
        entries.append(&mut self.settings_history);
        self.settings_history = entries;
        self.settings_history_cursor = Some(date);
    }

    /// Blank an archived row's command text in place, straight to the
    /// background writer (there is no in-memory `CommandEntry` for a paged-in
    /// entry to mutate) — the archive counterpart of
    /// [`Self::clear_scrollback_target`]. Also updates both surfaces'
    /// paged-in copies (the panel's and the settings viewer's) so they
    /// reflect it without waiting for a re-page.
    pub fn clear_archived_target(&mut self, target: &ArchivedTarget) {
        let Some(writer) = self.history_writer.as_ref() else {
            return;
        };
        writer.send(cathode::history::HistoryEvent::Upsert(
            cathode::history::PersistedCommandEntry {
                id: target.id,
                command: String::new(),
                started_at_epoch_ms: target.started_at_epoch_ms,
                pane_tag: target.pane_tag.clone(),
            },
        ));
        for list in [&mut self.scrollback_archived, &mut self.settings_history] {
            if let Some(entry) = list.iter_mut().find(|e| e.id == target.id) {
                entry.command.clear();
            }
        }
    }

    /// Permanently remove an archived row, straight to the background writer
    /// — the archive counterpart of [`Self::delete_scrollback_target`]. Also
    /// drops it from both surfaces' paged-in copies so the panel and the
    /// settings viewer reflect it immediately.
    pub fn delete_archived_target(&mut self, target: &ArchivedTarget) {
        let Some(writer) = self.history_writer.as_ref() else {
            return;
        };
        writer.send(cathode::history::HistoryEvent::Tombstone {
            id: target.id,
            started_at_epoch_ms: target.started_at_epoch_ms,
        });
        self.scrollback_archived.retain(|e| e.id != target.id);
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
        self.settings_history.retain(|e| e.id != target.id);
        self.settings_history_selected = None;
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

    /// Empty a single Scrollback History row's value in place (the row stays, its
    /// text goes blank) — the active pane's per-row "Clear" menu item, as opposed
    /// to [`Self::clear_active_scrollback`]'s wholesale wipe.
    pub fn clear_scrollback_target(&mut self, target: &ScrollbackTarget) {
        let Some(term) = self.active_term() else {
            return;
        };
        let mut screen = term.screen.lock();
        match *target {
            ScrollbackTarget::Command { log_index, .. } => screen.clear_command_output(log_index),
            ScrollbackTarget::Output {
                log_index, line, ..
            } => screen.clear_command_output_line(log_index, line),
        }
    }

    /// Permanently remove a Scrollback History command entry (its header row and
    /// all captured output) — the active pane's "Delete" menu item, unlike
    /// [`Self::clear_scrollback_target`]'s "blank the value, keep the row". Only
    /// applies to a `Command` target (no-op on an `Output` line — there's nothing
    /// sensible to "delete" for a single captured line, just clear it). Deleting
    /// shifts every later command's index, so the panel's selection/expand state
    /// (both indices into the row list that just changed) resets, mirroring
    /// [`Self::set_scrollback_query`]'s reasoning.
    pub fn delete_scrollback_target(&mut self, target: &ScrollbackTarget) {
        let ScrollbackTarget::Command { log_index, .. } = *target else {
            return;
        };
        let Some(term) = self.active_term() else {
            return;
        };
        term.screen.lock().remove_command(log_index);
        self.scrollback_selected = None;
        self.scrollback_expanded.clear();
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
        self.new_tab_with(false);
    }

    /// [`Self::new_tab`], optionally untracked — the tab's commands then never
    /// reach encrypted history (suppressed inside the screen itself). In an
    /// untracked *session* every tab is untracked regardless of what the
    /// caller asked for.
    pub fn new_tab_with(&mut self, untracked: bool) {
        let untracked = untracked || self.session_untracked;
        let cwd = self.active_term().and_then(|t| t.screen.lock().cwd.clone());
        let pane_tag = format!("Tab {}", self.tabs.len() + 1);
        if let Some(term) = spawn_term(
            80,
            24,
            cwd.as_deref(),
            self.settings.max_scrollback(),
            &pane_tag,
            self.history_id_floor,
            untracked,
        ) {
            let mut tab = Tab::new(term);
            tab.untracked = untracked;
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
    }

    /// Split `window`'s focused pane toward `dir`, spawning a fresh shell there (seeded
    /// with the focused pane's cwd) and focusing it. Left/Right split the column (vertical
    /// divider); Up/Down split the row (horizontal divider).
    pub fn split_focused(&mut self, window: iced::window::Id, dir: pane_grid::Direction) {
        let tab = self.tab_for(window);
        let cwd = tab
            .and_then(Tab::focused)
            .and_then(|t| t.screen.lock().cwd.clone());
        let pane_tag = tab.map(Tab::label).unwrap_or_else(|| "Tab".to_string());
        // A pane split off an untracked tab is untracked too — the promise is
        // per-tab, not per-pane.
        let untracked = tab.is_some_and(|t| t.untracked);
        if let Some(term) = spawn_term(
            80,
            24,
            cwd.as_deref(),
            self.settings.max_scrollback(),
            &pane_tag,
            self.history_id_floor,
            untracked,
        ) {
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
        let writer = self.history_writer.as_ref();
        let mut clip = None;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            for (_, term) in tab.panes.iter_mut() {
                let (signal, requested) = drain_pane(term, writer);
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
                let (_signal, requested) = drain_pane(term, writer);
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
            // A detached window inherits the app's always-on-top setting.
            level: self.window_level(),
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
