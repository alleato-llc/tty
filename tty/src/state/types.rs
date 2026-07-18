//! The data types behind the app state: the terminal `Term`, the pane content
//! `Pane`, a `Tab`'s pane tree, the big `Tty` state struct itself, and the
//! smaller value types (`MetricPopover`, `ResizeEdge`, `MenuKind`, the passphrase
//! prompt, the scrollback/archive row targets). Split out of `state.rs`; the
//! behavior (`impl Tty`) lives there and in the sibling submodules.

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
    /// Where this shell writes its captured environment for the **Env view**
    /// (`$TTY_ENV_FILE`, from `shell_integration::env_channel_path`). `None` when shell
    /// integration is off. The view creates a `<path>.on` flag to switch capture on.
    pub env_file: Option<std::path::PathBuf>,
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
    pub(super) fn has_live_pane(&self) -> bool {
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
    /// The absolute buffer line the focused pane should scroll to for OSC 133
    /// **prompt-jump** (`⌘↑`/`⌘↓`) — the prompt row of the command last jumped to, or
    /// `None` when at the live bottom. Fed to the focused pane's `scroll_to`; reset
    /// when the user types into the shell. See [`Tty::jump_to_prompt`].
    pub scroll_target: Option<usize>,
    /// Whether the **Env view** (`⌘⇧E`) is open for the active pane. While open, the
    /// pane's shell captures its env each prompt (via the `.on` flag) and
    /// [`Tty::refresh_env`] re-reads it. See [`crate::env`].
    pub show_env: bool,
    /// The env vars currently shown, sorted by name — from the live shell hook when it
    /// has captured, else the OS launch-time read (see [`Tty::refresh_env`]).
    pub env_vars: Vec<crate::env::EnvVar>,
    /// Where [`Tty::env_vars`] came from, so the popover can label live vs launch-time.
    pub env_source: EnvSource,
    /// Cache of the OS launch-time read, keyed by the pane shell's pid — that read is a
    /// full process-detail scan, so it runs only when the pid changes, not each redraw.
    pub env_os_cache: Option<(i32, Vec<crate::env::EnvVar>)>,
    /// The Env view's filter query.
    pub env_filter: String,
    /// Whether the Env view reveals values (off by default — env holds secrets, so
    /// values are masked until asked for). Only takes effect when expanded; the compact
    /// list is always masked.
    pub env_reveal: bool,
    /// Whether the Env popover is expanded to its full experience (filter, reveal toggle,
    /// source note, larger size) vs the compact default (a masked list + Add). Toggled by
    /// the popover's expand/restore control, like the metric drill-ins.
    pub env_expanded: bool,
    /// Whether the "Set a variable" modal (opened by the Env popover's Add button) is up.
    pub env_add_open: bool,
    /// The Env popover's top-left position in window pixels. `None` = not yet placed
    /// (the view centers it); a drag sets it, so it's remembered across opens.
    pub env_pos: Option<(f32, f32)>,
    /// The Env popover's `(width, height)` in pixels (border-drag resizes it).
    pub env_size: (f32, f32),
    /// Active title-bar move drag: `(pointer at grab, position at grab)`. Ended by
    /// `PointerReleased`. See the metric popovers' `metric_detail_move_drag`.
    pub env_move_drag: Option<(iced::Point, (f32, f32))>,
    /// Active border resize drag: `(pointer at grab, size at grab, edge)`.
    pub env_resize: Option<(iced::Point, (f32, f32), ResizeEdge)>,
    /// The "add to new shells" draft in the Shell settings overlay editor: the name and
    /// value being typed before [`Tty::add_env_overlay`] commits them to
    /// [`crate::settings::Settings::env`].
    pub env_overlay_name: String,
    pub env_overlay_value: String,
    /// The "set in this pane" draft in the Env popover: name + value to inject as an
    /// `export`/`unset` at the focused shell's prompt. See [`Tty::inject_env_set`].
    pub env_set_name: String,
    pub env_set_value: String,
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
    /// A fixed "now" (Unix-epoch ms) for the wall-clock-relative labels ("N ago", the
    /// archived date). `None` in normal use — [`Tty::now_ms`] reads the real clock.
    /// Snapshot tests set it so date/age render deterministically regardless of when
    /// (or on which side of midnight) the test runs.
    pub clock_override: Option<u64>,
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
    /// A pending "force quit" confirm from the Processes drill-in: the pid + name
    /// to `SIGKILL`. `Some` shows the confirm dialog; taken on confirm, cleared on
    /// cancel. (A plain "Quit"/`SIGTERM` needs no confirm.)
    pub kill_confirm: Option<(i32, String)>,
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
///
/// The popover chrome (the draggable, border-resizable card) lives in rime as
/// [`rime::widgets::popover`] / [`rime::widgets::resize_edges`]; this is rime's
/// [`rime::widgets::ResizeEdge`], re-exported here so the state, messages, and the
/// resize math (`.axes()`) can name it without a `rime::` prefix everywhere.
pub use rime::widgets::ResizeEdge;

/// Where the Env view's current list came from, so the popover can label it and set
/// expectations about freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvSource {
    /// Nothing available (view closed, or the pane has no live pid and no capture).
    #[default]
    None,
    /// Live: the shell-integration hook dumped the shell's env; it updates each prompt.
    Hook,
    /// Launch-time: read from the kernel (the process's initial environment). Static —
    /// variables the shell exports after launch won't appear until the hook is enabled.
    Process,
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
