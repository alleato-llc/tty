use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use unicode_width::UnicodeWidthChar;

use crate::history::{HistoryEvent, PersistedCommandEntry};

const DEFAULT_SCROLLBACK: usize = 2000;
const TAB_WIDTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermColor {
    Default,
    Named(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The cursor glyph a program asked for via DECSCUSR (`CSI Ps SP q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

/// How a program wants pointer events reported (DEC private modes 1000/1002/1003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseTracking {
    /// No reporting — the widget owns the mouse (select/copy).
    #[default]
    Off,
    /// 1000: press + release only.
    Normal,
    /// 1002: press/release + motion while a button is held (drag).
    ButtonDrag,
    /// 1003: all motion, button or not.
    AnyMotion,
}

/// Mouse reporting state: what to report and in which encoding.
#[derive(Debug, Clone, Copy, Default)]
pub struct MouseState {
    pub tracking: MouseTracking,
    /// 1006: SGR encoding (`CSI < b ; x ; y M/m`). We only emit SGR when set — it's
    /// the universal modern encoding and avoids the 223-column limit of the legacy one.
    pub sgr: bool,
}

impl MouseState {
    pub fn reports(&self) -> bool {
        self.tracking != MouseTracking::Off
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub ch: char,
    pub fg: TermColor,
    pub bg: TermColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub inverse: bool,
    /// Display columns this cell spans: 1 normal, 2 the left half of a wide glyph,
    /// 0 the right-half spacer of a wide glyph (skipped by the renderer + selection).
    pub width: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: TermColor::Default,
            bg: TermColor::Default,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            inverse: false,
            width: 1,
        }
    }
}

impl Cell {
    /// A right-half spacer for a wide glyph (no glyph of its own).
    fn spacer() -> Self {
        Self {
            width: 0,
            ..Self::default()
        }
    }
}

/// One command's recorded text + captured output, built live as you use the shell —
/// not derived from `scrollback` after the fact. `mark_command_boundary` starts a new
/// entry (the current row's text, right as Enter is sent); every completed line after
/// that gets appended to `output` until the next boundary, capped at `max_output_lines`
/// (resolved once, per command, by the host — see `crate::commands::resolve_output_cap`
/// — so a long-running/streaming command like `tail -f` stops growing instead of
/// accumulating forever).
#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub command: String,
    pub output: Vec<String>,
    pub started_at: Instant,
    max_output_lines: usize,
    /// Unique within [`TerminalScreen::pending_history_events`]'s destination file
    /// (a single day's persisted segment) — see [`crate::history::PersistedCommandEntry`].
    pub id: u32,
    /// Wall-clock counterpart of `started_at` (`Instant` has no epoch and can't be
    /// persisted or reconstructed after a restart).
    pub started_at_wall: SystemTime,
    /// Which pane recorded this command — copied from [`TerminalScreen::pane_tag`]
    /// at push time.
    pub pane_tag: String,
    /// Whether this command was recorded on an untracked screen — copied from
    /// [`TerminalScreen::untracked`] at push time, so a host's history UI can
    /// badge the row. Untracked entries exist only in this live log; the
    /// screen never queues history events for them.
    pub untracked: bool,
}

impl CommandEntry {
    /// Whether this command's output hit its cap (there may have been more that
    /// wasn't recorded).
    pub fn is_truncated(&self) -> bool {
        self.output.len() >= self.max_output_lines
    }
}

/// A finished shell command, reported via the OSC 133 semantic-prompt marks
/// (`133;C` = command started, `133;D[;code]` = command finished). Produced only
/// when shell integration is installed; the host drains these
/// ([`TerminalScreen::take_command_completions`]) to drive completion notifications.
/// `duration` is measured at parse time (real-time, on the PTY-read thread), so it's
/// accurate regardless of when the host drains it — cathode owns the clock here
/// because pairing `C`→`D` across byte chunks is what makes the measurement correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCompletion {
    /// The command text (from the most recent [`CommandEntry`] at finish time), for
    /// the notification body. Empty if nothing was captured.
    pub command: String,
    /// The reported exit code (`133;D;<code>`), or `None` when the mark omitted it.
    pub exit_code: Option<i32>,
    /// Wall-clock time from the `C` mark to the `D` mark.
    pub duration: Duration,
}

/// The OSC 133 command currently being delimited. Positions are stored as global
/// line ids (`lines_scrolled + cursor_row` at mark time); see [`CommandMark`].
#[derive(Debug, Clone, Copy, Default)]
struct PendingMark {
    /// Prompt line (`133;A`, or `133;B` if the shell skips A).
    prompt_line: Option<u64>,
    /// Output region start (`133;C`).
    output_start: Option<u64>,
    /// Start instant (`133;C`), for the completion notification's duration.
    started_at: Option<Instant>,
}

/// One finished OSC 133 command's positions, as stable global line ids (see
/// [`TerminalScreen::lines_scrolled`]). Internal; the host reads the resolved,
/// current-buffer form via [`TerminalScreen::command_regions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandMark {
    prompt_line: u64,
    /// `(start, end)` global line ids of the command's output, when `C` and `D` were
    /// both seen.
    output: Option<(u64, u64)>,
    exit_code: Option<i32>,
}

/// A finished OSC 133 command resolved to **current** buffer positions — the host's
/// view for prompt-jump navigation, failed-command gutter marks, and copying a
/// command's output. `prompt_row` and `output` are absolute line indices into the
/// scrollback-plus-live-grid buffer, the same coordinate space as [`Terminal`]'s
/// `scroll_to`. Only commands whose prompt line is still in the buffer appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRegion {
    /// Absolute line of the prompt (for scroll-to and the gutter marker).
    pub prompt_row: usize,
    /// `(start, end)` absolute lines of the command's output, inclusive, when it
    /// produced any (i.e. `C` and `D` were both seen and the start hasn't evicted).
    pub output: Option<(usize, usize)>,
    pub exit_code: Option<i32>,
}

impl CommandRegion {
    /// Whether the command reported a non-zero exit code.
    pub fn failed(&self) -> bool {
        matches!(self.exit_code, Some(code) if code != 0)
    }
}

/// How many [`CommandMark`]s a screen retains before evicting the oldest — a bound on
/// the ephemeral OSC 133 nav data, independent of the persisted `command_log`.
const MAX_COMMAND_MARKS: usize = 512;

/// How many commands [`TerminalScreen::mark_command_boundary`] remembers before
/// evicting the oldest — not user-configurable (unlike the per-command output cap),
/// just a generous backstop. `pub` so a host loading persisted history back in (see
/// [`TerminalScreen::seed_command_log`]) can bound how many entries it bothers
/// reading off disk at startup, rather than duplicating this number.
pub const MAX_COMMAND_LOG: usize = 500;

/// A queued command boundary — see [`TerminalScreen::pending_boundaries`].
struct PendingBoundary {
    /// `None` = derive the command text from the completing row when this boundary
    /// is consumed (a live-typed line, whose fully-resolved text only exists in the
    /// terminal grid once it's echoed back). `Some` = the caller already knows the
    /// text (a pasted line, known before it's ever sent to the shell).
    command: Option<String>,
    max_output_lines: usize,
}

/// Saved pen + cursor for DECSC/DECRC and the alt-screen swap.
#[derive(Clone, Copy)]
struct Saved {
    row: usize,
    col: usize,
    fg: TermColor,
    bg: TermColor,
    bold: bool,
    italic: bool,
    underline: bool,
    dim: bool,
    inverse: bool,
}

pub struct TerminalScreen {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_top: usize,
    pub scroll_bot: usize,
    pub scrollback: VecDeque<Vec<Cell>>,
    /// When each `scrollback` row was pushed, in lockstep — the data behind
    /// [`TerminalScreen::oldest_scrollback_age`]. A parallel deque rather than
    /// widening `Cell`/`Vec<Cell>` itself, which every scrollback reader already
    /// assumes is a plain row of cells.
    scrollback_times: VecDeque<Instant>,
    /// The scrollback cap (configurable per app settings) — how many rows
    /// [`TerminalScreen::scroll_up`] keeps before evicting the oldest.
    max_scrollback: usize,
    /// Commands run so far (see [`CommandEntry`]), oldest first — a separate, live-built
    /// record, not derived from `scrollback`.
    pub command_log: VecDeque<CommandEntry>,
    /// Boundaries queued by [`Self::mark_command_boundary`] /
    /// [`Self::mark_command_boundary_with`], oldest first — resolved into a
    /// `CommandEntry` (created then, not when queued) by
    /// [`Self::record_output_line`] once the front one's own line is seen: a
    /// live-typed boundary (`None`) resolves on the very next row completion
    /// (nothing else can race in between a real Enter and its echo); a known-text
    /// boundary (`Some`, a pasted line) waits for a completing row that actually
    /// ends with that text, since several can be queued at once — ahead of *any*
    /// of their echoes — with the previous command's real output legitimately
    /// completing rows in between.
    pending_boundaries: VecDeque<PendingBoundary>,
    /// Which pane this screen belongs to, for [`CommandEntry::pane_tag`] — set once
    /// by the host via [`Self::set_pane_tag`]; empty until then.
    pane_tag: String,
    /// An untracked ("incognito") screen: commands still enter the live
    /// `command_log` (badged via [`CommandEntry::untracked`]) but no history
    /// event is ever queued — suppressed at the source, so no host drain
    /// path can leak them to persistence. Set once at spawn via
    /// [`Self::set_untracked`].
    untracked: bool,
    /// The next id [`Self::record_output_line`] assigns to a new [`CommandEntry`].
    /// Monotonic for the life of this screen; ids only need to be unique within a
    /// single persisted day, and a globally-increasing counter trivially satisfies
    /// that without cathode needing to know what day it is.
    next_command_id: u32,
    /// Changes queued for the host's persisted-history writer, drained by
    /// [`Self::take_pending_history_events`]. See [`crate::history::HistoryEvent`].
    pending_history_events: Vec<HistoryEvent>,
    pub dirty_rows: BTreeSet<usize>,
    // Pen (current SGR state).
    fg: TermColor,
    bg: TermColor,
    bold: bool,
    italic: bool,
    underline: bool,
    dim: bool,
    inverse: bool,
    // DECSC/DECRC saved cursor.
    saved: Option<Saved>,

    // Modes + cursor presentation, surfaced to the widget / app.
    pub cursor_visible: bool,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    /// DECCKM (mode 1): arrow keys send `ESC O A` instead of `ESC [ A`.
    pub app_cursor_keys: bool,
    /// Bracketed paste (mode 2004): the app wants pastes wrapped in `ESC[200~…ESC[201~`.
    pub bracketed_paste: bool,
    pub mouse: MouseState,

    // Alternate screen (1049/47/1047): a separate buffer with no scrollback, so
    // full-screen apps don't pollute history. We stash the main buffer while in alt.
    alt: bool,
    saved_main: Option<(Vec<Cell>, usize, usize)>, // cells, cursor_row, cursor_col

    // OSC-driven state the host reads.
    pub title: Option<String>,
    pub cwd: Option<String>,
    bell: bool,
    clipboard: Option<String>,
    /// Monotonic count of lines ever pushed into scrollback — the basis for the
    /// stable "global line id" that OSC 133 marks are stored against, so a recorded
    /// position survives eviction without renumbering (see [`Self::command_regions`]).
    lines_scrolled: u64,
    /// The OSC 133 command currently being delimited, built up across the `A`/`B`/`C`
    /// marks and finalized at `D`. `None` between commands / with no shell integration.
    pending_mark: Option<PendingMark>,
    /// Finalized OSC 133 command marks (positions + exit code), oldest first — pruned
    /// as their prompt line evicts from scrollback and capped at [`MAX_COMMAND_MARKS`].
    command_marks: Vec<CommandMark>,
    /// Finished commands (OSC 133) queued for the host — see [`CommandCompletion`]
    /// and [`Self::take_command_completions`].
    pending_command_completions: Vec<CommandCompletion>,
}

impl TerminalScreen {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback(cols, rows, DEFAULT_SCROLLBACK)
    }

    /// Like [`TerminalScreen::new`], with a configurable scrollback cap (the host
    /// reads this from its settings — see `set_max_scrollback` to change it live).
    pub fn with_scrollback(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
            cursor_row: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_bot: rows.saturating_sub(1),
            scrollback: VecDeque::with_capacity(max_scrollback),
            scrollback_times: VecDeque::with_capacity(max_scrollback),
            max_scrollback,
            command_log: VecDeque::new(),
            pending_boundaries: VecDeque::new(),
            pane_tag: String::new(),
            untracked: false,
            next_command_id: 0,
            pending_history_events: Vec::new(),
            lines_scrolled: 0,
            pending_mark: None,
            command_marks: Vec::new(),
            pending_command_completions: Vec::new(),
            dirty_rows: BTreeSet::new(),
            fg: TermColor::Default,
            bg: TermColor::Default,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            inverse: false,
            saved: None,
            cursor_visible: true,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            app_cursor_keys: false,
            bracketed_paste: false,
            mouse: MouseState::default(),
            alt: false,
            saved_main: None,
            title: None,
            cwd: None,
            bell: false,
            clipboard: None,
        }
    }

    /// Whether the alternate screen is active (full-screen app). The host can use this
    /// to suppress its own scrollback affordances.
    pub fn alt_screen(&self) -> bool {
        self.alt
    }

    /// Read-and-clear the bell flag (rung since the last check).
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }

    /// Read-and-clear a pending OSC 52 clipboard write request.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Read-and-clear the shell commands that finished (OSC 133) since the last call.
    pub fn take_command_completions(&mut self) -> Vec<CommandCompletion> {
        std::mem::take(&mut self.pending_command_completions)
    }

    /// The global line id of the row the cursor is on right now — how an OSC 133 mark
    /// captures a position it can find again after the line scrolls (see
    /// [`Self::lines_scrolled`]).
    fn current_line_id(&self) -> u64 {
        self.lines_scrolled + self.cursor_row as u64
    }

    /// The global line id of the oldest line still in the buffer (front of scrollback).
    /// A mark below this has evicted.
    fn buffer_base_line(&self) -> u64 {
        self.lines_scrolled - self.scrollback.len() as u64
    }

    /// Finalize the pending OSC 133 command at its `D` mark: record a [`CommandMark`]
    /// (positions + exit) for the host's nav/flag/copy features, and — when we saw the
    /// command actually start (`C`) — queue a completion for the notification. A `D`
    /// with no prior mark (e.g. an empty Enter, which emits `D` alone) records nothing.
    fn finish_command_mark(&mut self, end_line: u64, exit_code: Option<i32>) {
        let Some(pending) = self.pending_mark.take() else {
            return;
        };
        // Fall back to the output start as the "prompt" when a shell emits only C/D.
        if let Some(prompt_line) = pending.prompt_line.or(pending.output_start) {
            self.command_marks.push(CommandMark {
                prompt_line,
                output: pending.output_start.map(|start| (start, end_line)),
                exit_code,
            });
            self.prune_command_marks();
        }
        if let Some(started) = pending.started_at {
            let command = self
                .command_log
                .back()
                .map(|e| e.command.clone())
                .unwrap_or_default();
            self.pending_command_completions.push(CommandCompletion {
                command,
                exit_code,
                duration: started.elapsed(),
            });
        }
    }

    /// Drop marks whose prompt line has evicted from the buffer, then cap the list.
    fn prune_command_marks(&mut self) {
        let base = self.buffer_base_line();
        self.command_marks.retain(|m| m.prompt_line >= base);
        let overflow = self.command_marks.len().saturating_sub(MAX_COMMAND_MARKS);
        if overflow > 0 {
            self.command_marks.drain(0..overflow);
        }
    }

    /// The finished OSC 133 commands still visible in the buffer, oldest first,
    /// resolved to current absolute line positions (the same coordinate as
    /// [`crate::terminal`]'s `scroll_to`). Drives prompt-jump, failed-command marks,
    /// and output copy on the host. Empty without shell integration.
    pub fn command_regions(&self) -> Vec<CommandRegion> {
        let base = self.buffer_base_line();
        self.command_marks
            .iter()
            .filter(|m| m.prompt_line >= base)
            .map(|m| CommandRegion {
                prompt_row: (m.prompt_line - base) as usize,
                // Keep the output span only while its end is still in the buffer;
                // clamp a partially-evicted start up to the buffer's first line.
                output: m.output.and_then(|(start, end)| {
                    (end >= base).then(|| ((start.max(base) - base) as usize, (end - base) as usize))
                }),
                exit_code: m.exit_code,
            })
            .collect()
    }

    /// Set which pane this screen belongs to, for [`CommandEntry::pane_tag`] on
    /// every command recorded from here on. Called once by the host right after
    /// creating a pane's screen.
    pub fn set_pane_tag(&mut self, tag: String) {
        self.pane_tag = tag;
    }

    /// Mark this screen untracked (or not) — see the `untracked` field. Set
    /// once by the host right after creating a pane's screen, alongside
    /// [`Self::set_pane_tag`].
    pub fn set_untracked(&mut self, untracked: bool) {
        self.untracked = untracked;
    }

    /// Whether this screen is untracked.
    pub fn untracked(&self) -> bool {
        self.untracked
    }

    /// Queue a change for the host's persisted-history writer — the single
    /// gate every push site goes through: an untracked screen queues nothing,
    /// ever, so nothing downstream can persist it.
    fn queue_history_event(&mut self, event: HistoryEvent) {
        if self.untracked {
            return;
        }
        self.pending_history_events.push(event);
    }

    /// Read-and-clear the changes queued for the host's persisted-history writer
    /// since the last call.
    pub fn take_pending_history_events(&mut self) -> Vec<HistoryEvent> {
        std::mem::take(&mut self.pending_history_events)
    }

    /// Raise the command-id floor: the next recorded command gets an id of at
    /// least `next`. Never regresses an id counter that is already higher.
    ///
    /// Persisted-history hosts need this when their archive unlocks *after*
    /// the screen has already recorded commands (a deferred or passphrase-
    /// gated start): day segments upsert by id, so a fresh screen's ids
    /// (`0..n`) would otherwise overwrite same-day entries persisted by an
    /// earlier launch. [`Self::seed_command_log`] applies the same rule for
    /// the ids it loads; this is the hook for screens that were never seeded.
    pub fn reserve_command_ids(&mut self, next: u32) {
        self.next_command_id = self.next_command_id.max(next);
    }

    /// Load previously-persisted entries back into `command_log` (most recent
    /// last, matching how live entries accumulate), respecting `MAX_COMMAND_LOG`
    /// and advancing `next_command_id` past every loaded id so a new command this
    /// session can never collide with one restored from today's segment. Does not
    /// queue any history events — loading is the reverse of a mutation, not one.
    pub fn seed_command_log(&mut self, entries: Vec<PersistedCommandEntry>) {
        for persisted in entries {
            self.next_command_id = self.next_command_id.max(persisted.id + 1);
            self.command_log.push_back(CommandEntry {
                command: persisted.command,
                output: Vec::new(),
                started_at: instant_for(persisted.started_at_epoch_ms),
                // Output is never populated for a loaded entry (it was never
                // persisted), so any cap here reads as "not truncated" — the
                // point is just to avoid `0 >= 0` reading as truncated.
                max_output_lines: usize::MAX,
                id: persisted.id,
                started_at_wall: wall_time_from_epoch_ms(persisted.started_at_epoch_ms),
                pane_tag: persisted.pane_tag,
                // Loaded from the archive, so by definition it was tracked.
                untracked: false,
            });
            if self.command_log.len() > MAX_COMMAND_LOG {
                self.command_log.pop_front();
            }
        }
    }

    /// Change the scrollback cap, truncating from the front if the buffer is now over
    /// it — so lowering the setting applies immediately to an already-open terminal,
    /// not just ones spawned afterward.
    pub fn set_max_scrollback(&mut self, cap: usize) {
        self.max_scrollback = cap;
        while self.scrollback.len() > cap {
            self.scrollback.pop_front();
            self.scrollback_times.pop_front();
        }
    }

    /// The current scrollback cap (see [`Self::set_max_scrollback`]).
    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    /// Drop every buffered (off-screen) scrollback line. The live on-screen grid and
    /// cursor are untouched — this is "clear history," not the shell's own `clear`
    /// (which resets the visible screen instead).
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.scrollback_times.clear();
        let tombstones: Vec<HistoryEvent> = self
            .command_log
            .iter()
            .map(|entry| HistoryEvent::Tombstone {
                id: entry.id,
                started_at_epoch_ms: crate::history::epoch_ms(entry.started_at_wall),
            })
            .collect();
        for tombstone in tombstones {
            self.queue_history_event(tombstone);
        }
        self.command_log.clear();
        self.pending_boundaries.clear();
        // Scrollback is gone, so any OSC 133 mark that lived there has evicted; only a
        // mark still on the live grid (a current prompt) survives.
        self.prune_command_marks();
    }

    /// Blank one recorded command's own text *and* its captured output in place
    /// (index into `command_log`) — the row stays (so later commands keep their
    /// index), its displayed value just empties. Blanking the command text too
    /// (not just the output) matters: plenty of real commands (`cd`, `export`,
    /// `alias`) capture zero output, so leaving the command text untouched would
    /// make "Clear" a silent no-op for them. Distinct from `clear_scrollback`'s
    /// wholesale wipe: this is the Scrollback History panel's per-row "Clear" menu
    /// item, scoped to a single command.
    pub fn clear_command_output(&mut self, index: usize) {
        if let Some(entry) = self.command_log.get_mut(index) {
            entry.command.clear();
            entry.output.clear();
            let event = HistoryEvent::Upsert(PersistedCommandEntry::from(&*entry));
            self.queue_history_event(event);
        }
    }

    /// Blank a single recorded output line's text in place (index into
    /// `command_log`, then into that command's `output`) — the line stays (so
    /// later lines keep their index), its text just empties. The per-row
    /// "Clear" menu item for an output row, as opposed to a whole command's. No
    /// history event: captured output is never persisted, so blanking one
    /// output line has nothing to reflect in the archive.
    pub fn clear_command_output_line(&mut self, index: usize, line: usize) {
        if let Some(entry) = self.command_log.get_mut(index) {
            if let Some(l) = entry.output.get_mut(line) {
                l.clear();
            }
        }
    }

    /// Permanently remove one recorded command (its header row and all captured
    /// output) — index into `command_log`. Every later command's index shifts
    /// down by one. Distinct from `clear_command_output`'s "blank the value, keep
    /// the row": this is the Scrollback History panel's per-row "Delete" menu
    /// item, which only applies to a command's header row.
    pub fn remove_command(&mut self, index: usize) {
        if let Some(entry) = self.command_log.get(index) {
            let tombstone = HistoryEvent::Tombstone {
                id: entry.id,
                started_at_epoch_ms: crate::history::epoch_ms(entry.started_at_wall),
            };
            self.queue_history_event(tombstone);
        }
        self.command_log.remove(index);
    }

    /// The current cursor row's text, trimmed of trailing whitespace — what
    /// [`Self::mark_command_boundary`] would record as the command right now.
    pub fn current_row_text(&self) -> String {
        let row = self.cursor_row;
        let s: String = (0..self.cols).map(|c| self.cell(row, c).ch).collect();
        s.trim_end().to_string()
    }

    /// Queue a new command boundary — call this right before writing Enter/Return
    /// bytes to the shell. The *next* row to complete is this command's own line
    /// finalizing (its echoed Enter) — [`Self::record_output_line`] captures the
    /// *completing* row's text then (whatever's been typed, edited, tab-completed, or
    /// history-recalled so far — the terminal grid is the only place that
    /// fully-resolved text exists) as the command, and starts accumulating its output
    /// from the next completed line onward, up to `max_output_lines` (the host
    /// resolves this — see `crate::commands`). A no-op on the alt screen: a
    /// full-screen app isn't "a command with output" the way a shell invocation is,
    /// the same reasoning that already keeps it out of `scrollback`.
    pub fn mark_command_boundary(&mut self, max_output_lines: usize) {
        self.queue_boundary(None, max_output_lines);
    }

    /// Like [`Self::mark_command_boundary`], but for when the caller already knows
    /// the command text — a pasted line, known before it's ever sent to the shell (so
    /// there's nothing to read off the terminal grid; the paste hasn't been echoed
    /// yet). Queuing rather than creating the entry immediately is what lets a
    /// multi-line paste queue one boundary per complete line and have each resolve as
    /// that line's own echo arrives, in order, over however many `advance_row` calls
    /// it takes — not all at once.
    pub fn mark_command_boundary_with(&mut self, command: String, max_output_lines: usize) {
        self.queue_boundary(Some(command), max_output_lines);
    }

    fn queue_boundary(&mut self, command: Option<String>, max_output_lines: usize) {
        if self.alt {
            return;
        }
        self.pending_boundaries.push_back(PendingBoundary {
            command,
            max_output_lines,
        });
    }

    /// Append the row the cursor is *leaving* to the most recent command's output
    /// (if any, and it hasn't hit its cap), or resolve the front of
    /// [`Self::pending_boundaries`] if this completing row is that boundary's own
    /// line — called as a row is about to advance (see [`Self::advance_row`]), so
    /// it's the same for a wrapped line or an explicit newline. A no-op on the alt
    /// screen, same as `scrollback`.
    fn record_output_line(&mut self) {
        if self.alt {
            return;
        }
        let row = self.cursor_row;
        let cols = self.cols;
        let text: String = (0..cols).map(|c| self.cells[row * cols + c].ch).collect();
        let trimmed = text.trim_end();

        if let Some(front) = self.pending_boundaries.front() {
            // A live-typed boundary (`None`) has nothing to compare against — by
            // construction nothing else can complete a row between marking it and
            // the shell echoing that same Enter, so the very next completion always
            // is it. A known-text boundary (a pasted line, queued before any of a
            // multi-line paste is echoed) instead waits for a completing row whose
            // text actually ends with it — since several boundaries can be queued
            // at once, ahead of *any* of their echoes, and other rows (the previous
            // command's real output) may legitimately complete in between.
            let resolves_now = match &front.command {
                None => true,
                Some(known) => trimmed.ends_with(known.as_str()),
            };
            if resolves_now {
                let pending = self
                    .pending_boundaries
                    .pop_front()
                    .expect("front() just returned Some");
                let id = self.next_command_id;
                self.next_command_id += 1;
                let entry = CommandEntry {
                    command: pending.command.unwrap_or_else(|| trimmed.to_string()),
                    output: Vec::new(),
                    started_at: Instant::now(),
                    max_output_lines: pending.max_output_lines,
                    id,
                    started_at_wall: SystemTime::now(),
                    pane_tag: self.pane_tag.clone(),
                    untracked: self.untracked,
                };
                let event = HistoryEvent::Upsert(PersistedCommandEntry::from(&entry));
                self.queue_history_event(event);
                self.command_log.push_back(entry);
                // A window eviction, not a delete: the archived copy (if any) is
                // untouched, only today's in-memory live view shrinks back down.
                if self.command_log.len() > MAX_COMMAND_LOG {
                    self.command_log.pop_front();
                }
                return;
            }
        }

        let has_room = self
            .command_log
            .back()
            .is_some_and(|e| e.output.len() < e.max_output_lines);
        if !has_room {
            return;
        }
        if let Some(entry) = self.command_log.back_mut() {
            entry.output.push(trimmed.to_string());
        }
    }

    /// The full buffered + on-screen transcript, oldest first, each row trimmed of
    /// trailing whitespace — the data source for a scrollback history view.
    pub fn transcript_lines(&self) -> Vec<String> {
        let row_text = |cells: &[Cell]| -> String {
            let s: String = cells.iter().map(|c| c.ch).collect();
            s.trim_end().to_string()
        };
        // The live grid is only appended off the *main* screen. On the alt screen (a
        // full-screen app like htop/vim/less) `self.cells` is that app's live UI, not
        // history — exactly why `scroll_up` never lets it into `scrollback` either.
        let live: Vec<String> = if self.alt {
            Vec::new()
        } else {
            (0..self.rows)
                .map(|r| row_text(&self.cells[r * self.cols..(r + 1) * self.cols]))
                .collect()
        };
        self.scrollback
            .iter()
            .map(|row| row_text(row))
            .chain(live)
            .collect()
    }

    /// How long the oldest buffered scrollback line has been sitting there, if any.
    pub fn oldest_scrollback_age(&self) -> Option<Duration> {
        self.scrollback_times.front().map(Instant::elapsed)
    }

    /// Resize the grid, preserving the overlapping top-left content (so a window
    /// resize doesn't blank the screen). Cursor + scroll region clamp to the new size.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        let mut next = vec![Cell::default(); cols * rows];
        for r in 0..rows.min(self.rows) {
            for c in 0..cols.min(self.cols) {
                next[r * cols + c] = self.cells[r * self.cols + c].clone();
            }
        }
        self.cells = next;
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.scroll_top = self.scroll_top.min(rows.saturating_sub(1));
        self.scroll_bot = rows.saturating_sub(1);
        self.dirty_rows.extend(0..rows);
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }

    fn set_cell(&mut self, row: usize, col: usize, cell: Cell) {
        self.cells[row * self.cols + col] = cell;
        self.dirty_rows.insert(row);
    }

    fn pen(&self, ch: char, width: u8) -> Cell {
        Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            dim: self.dim,
            inverse: self.inverse,
            width,
        }
    }

    fn write_char(&mut self, ch: char) {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            // Combining mark / zero-width: dropped for now (full grapheme clustering
            // is a separate enhancement). Control chars never reach here.
            return;
        }
        // Wrap if the glyph (1 or 2 cells) won't fit on the current row.
        if self.cursor_col + w > self.cols {
            self.cursor_col = 0;
            self.advance_row();
        }
        let (row, col) = (self.cursor_row, self.cursor_col);
        self.set_cell(row, col, self.pen(ch, w as u8));
        if w == 2 && col + 1 < self.cols {
            self.set_cell(row, col + 1, Cell::spacer());
        }
        self.cursor_col += w;
    }

    fn advance_row(&mut self) {
        // The row being left is now finalized — record it as output before it's gone
        // (whether it's advancing because of a wrap or an explicit newline; both call
        // this, so both are captured uniformly).
        self.record_output_line();
        if self.cursor_row >= self.scroll_bot {
            self.scroll_up(1);
        } else {
            self.cursor_row += 1;
        }
    }

    /// Scroll the region up by `count` rows. Rows leaving the top of the *main* screen
    /// enter scrollback; on the alt screen they're discarded.
    pub fn scroll_up(&mut self, count: usize) {
        for _ in 0..count {
            if !self.alt {
                let top: Vec<Cell> = (0..self.cols)
                    .map(|c| self.cell(self.scroll_top, c).clone())
                    .collect();
                if self.scrollback.len() >= self.max_scrollback {
                    self.scrollback.pop_front();
                    self.scrollback_times.pop_front();
                }
                self.scrollback.push_back(top);
                self.scrollback_times.push_back(Instant::now());
                // One more line has entered scrollback: advance the global line id so
                // OSC 133 marks stay pinned to their line as the buffer scrolls/evicts.
                self.lines_scrolled += 1;
            }
            for r in self.scroll_top..self.scroll_bot {
                for c in 0..self.cols {
                    self.cells[r * self.cols + c] = self.cells[(r + 1) * self.cols + c].clone();
                }
                self.dirty_rows.insert(r);
            }
            self.clear_line(self.scroll_bot);
        }
    }

    /// Scroll the region down by `count` (blank lines inserted at the top of the region).
    pub fn scroll_down(&mut self, count: usize) {
        for _ in 0..count {
            let mut r = self.scroll_bot;
            while r > self.scroll_top {
                for c in 0..self.cols {
                    self.cells[r * self.cols + c] = self.cells[(r - 1) * self.cols + c].clone();
                }
                self.dirty_rows.insert(r);
                r -= 1;
            }
            self.clear_line(self.scroll_top);
        }
    }

    fn clear_line(&mut self, row: usize) {
        for c in 0..self.cols {
            self.cells[row * self.cols + c] = Cell::default();
        }
        self.dirty_rows.insert(row);
    }

    fn tab(&mut self) {
        let next = ((self.cursor_col / TAB_WIDTH) + 1) * TAB_WIDTH;
        self.cursor_col = next.min(self.cols.saturating_sub(1));
    }

    fn save_cursor(&mut self) {
        self.saved = Some(Saved {
            row: self.cursor_row,
            col: self.cursor_col,
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            dim: self.dim,
            inverse: self.inverse,
        });
    }

    fn restore_cursor(&mut self) {
        if let Some(s) = self.saved {
            self.cursor_row = s.row.min(self.rows.saturating_sub(1));
            self.cursor_col = s.col.min(self.cols.saturating_sub(1));
            self.fg = s.fg;
            self.bg = s.bg;
            self.bold = s.bold;
            self.italic = s.italic;
            self.underline = s.underline;
            self.dim = s.dim;
            self.inverse = s.inverse;
        }
    }

    /// Insert `n` blank cells at the cursor, shifting the rest of the row right (ICH).
    fn insert_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let start = self.cursor_col;
        for c in (start..self.cols).rev() {
            self.cells[row * self.cols + c] = if c >= start + n {
                self.cells[row * self.cols + (c - n)].clone()
            } else {
                Cell::default()
            };
        }
        self.dirty_rows.insert(row);
    }

    /// Delete `n` cells at the cursor, shifting the rest of the row left (DCH).
    fn delete_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let start = self.cursor_col;
        for c in start..self.cols {
            self.cells[row * self.cols + c] = if c + n < self.cols {
                self.cells[row * self.cols + (c + n)].clone()
            } else {
                Cell::default()
            };
        }
        self.dirty_rows.insert(row);
    }

    /// Erase `n` cells at the cursor in place (ECH).
    fn erase_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        for c in self.cursor_col..(self.cursor_col + n).min(self.cols) {
            self.cells[row * self.cols + c] = Cell::default();
        }
        self.dirty_rows.insert(row);
    }

    /// Insert `n` blank lines at the cursor row within the scroll region (IL).
    fn insert_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bot {
            return;
        }
        let saved = self.scroll_top;
        self.scroll_top = self.cursor_row;
        self.scroll_down(n);
        self.scroll_top = saved;
    }

    /// Delete `n` lines at the cursor row within the scroll region (DL).
    fn delete_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bot {
            return;
        }
        let saved = self.scroll_top;
        self.scroll_top = self.cursor_row;
        self.scroll_up(n);
        self.scroll_top = saved;
    }

    fn enter_alt(&mut self) {
        if self.alt {
            return;
        }
        self.save_cursor();
        self.saved_main = Some((self.cells.clone(), self.cursor_row, self.cursor_col));
        self.alt = true;
        self.cells = vec![Cell::default(); self.cols * self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.dirty_rows.extend(0..self.rows);
    }

    fn leave_alt(&mut self) {
        if !self.alt {
            return;
        }
        if let Some((cells, row, col)) = self.saved_main.take() {
            // The main buffer may predate a resize; rebuild at the current size.
            if cells.len() == self.cols * self.rows {
                self.cells = cells;
            } else {
                self.cells = vec![Cell::default(); self.cols * self.rows];
            }
            self.cursor_row = row.min(self.rows.saturating_sub(1));
            self.cursor_col = col.min(self.cols.saturating_sub(1));
        }
        self.alt = false;
        self.restore_cursor();
        self.dirty_rows.extend(0..self.rows);
    }

    /// Set/reset a DEC private mode (`CSI ? Pm h/l`).
    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.app_cursor_keys = on,
            25 => self.cursor_visible = on,
            1000 => {
                self.mouse.tracking = if on {
                    MouseTracking::Normal
                } else {
                    MouseTracking::Off
                }
            }
            1002 => {
                self.mouse.tracking = if on {
                    MouseTracking::ButtonDrag
                } else {
                    MouseTracking::Off
                }
            }
            1003 => {
                self.mouse.tracking = if on {
                    MouseTracking::AnyMotion
                } else {
                    MouseTracking::Off
                }
            }
            1006 => self.mouse.sgr = on,
            2004 => self.bracketed_paste = on,
            47 | 1047 => {
                if on {
                    self.enter_alt();
                } else {
                    self.leave_alt();
                }
            }
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    self.enter_alt();
                } else {
                    self.leave_alt();
                }
            }
            _ => {}
        }
    }

    fn apply_sgr(&mut self, params: &[u16]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => {
                    self.fg = TermColor::Default;
                    self.bg = TermColor::Default;
                    self.bold = false;
                    self.italic = false;
                    self.underline = false;
                    self.dim = false;
                    self.inverse = false;
                }
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                7 => self.inverse = true,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                27 => self.inverse = false,
                30..=37 => self.fg = TermColor::Named(params[i] as u8 - 30),
                38 if i + 2 < params.len() && params[i + 1] == 5 => {
                    self.fg = TermColor::Indexed(params[i + 2] as u8);
                    i += 2;
                }
                38 if i + 4 < params.len() && params[i + 1] == 2 => {
                    self.fg = TermColor::Rgb(
                        params[i + 2] as u8,
                        params[i + 3] as u8,
                        params[i + 4] as u8,
                    );
                    i += 4;
                }
                39 => self.fg = TermColor::Default,
                40..=47 => self.bg = TermColor::Named(params[i] as u8 - 40),
                48 if i + 2 < params.len() && params[i + 1] == 5 => {
                    self.bg = TermColor::Indexed(params[i + 2] as u8);
                    i += 2;
                }
                48 if i + 4 < params.len() && params[i + 1] == 2 => {
                    self.bg = TermColor::Rgb(
                        params[i + 2] as u8,
                        params[i + 3] as u8,
                        params[i + 4] as u8,
                    );
                    i += 4;
                }
                49 => self.bg = TermColor::Default,
                90..=97 => self.fg = TermColor::Named(params[i] as u8 - 90 + 8),
                100..=107 => self.bg = TermColor::Named(params[i] as u8 - 100 + 8),
                _ => {}
            }
            i += 1;
        }
    }
}

fn wall_time_from_epoch_ms(epoch_ms: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(epoch_ms)
}

/// An `Instant` that elapses the same duration as a wall-clock timestamp from
/// `epoch_ms` — restores a loaded [`CommandEntry`]'s live "how long ago" display
/// correctly instead of resetting it to "just now". Falls back to
/// `Instant::now()` (reads as "0s ago") for a timestamp in the future or an
/// elapsed duration this platform's `Instant` can't represent — a display
/// nicety, never a correctness issue.
fn instant_for(epoch_ms: u64) -> Instant {
    let wall = wall_time_from_epoch_ms(epoch_ms);
    match SystemTime::now().duration_since(wall) {
        Ok(elapsed) => Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now),
        Err(_) => Instant::now(),
    }
}

impl vte::Perform for TerminalScreen {
    fn print(&mut self, c: char) {
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.cursor_col = 0,
            b'\n' | 0x0B | 0x0C => {
                self.advance_row();
                self.dirty_rows.insert(self.cursor_row);
            }
            0x09 => self.tab(),
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            0x07 => self.bell = true,
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let ps: Vec<u16> = params.iter().map(|p| p[0]).collect();
        let p0 = *ps.first().unwrap_or(&0);
        let p1 = *ps.get(1).unwrap_or(&0);
        let n0 = p0.max(1) as usize;

        // DEC private modes: `CSI ? Pm h/l`.
        if intermediates.first() == Some(&b'?') && (action == 'h' || action == 'l') {
            let on = action == 'h';
            for &m in &ps {
                self.set_private_mode(m, on);
            }
            return;
        }
        // DECSCUSR cursor shape: `CSI Ps SP q`.
        if intermediates.first() == Some(&b' ') && action == 'q' {
            let (shape, blink) = match p0 {
                0 | 1 => (CursorShape::Block, true),
                2 => (CursorShape::Block, false),
                3 => (CursorShape::Underline, true),
                4 => (CursorShape::Underline, false),
                5 => (CursorShape::Bar, true),
                6 => (CursorShape::Bar, false),
                _ => (CursorShape::Block, true),
            };
            self.cursor_shape = shape;
            self.cursor_blink = blink;
            return;
        }

        match action {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(n0),
            'B' => self.cursor_row = (self.cursor_row + n0).min(self.rows.saturating_sub(1)),
            'C' => self.cursor_col = (self.cursor_col + n0).min(self.cols.saturating_sub(1)),
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n0),
            'E' => {
                self.cursor_col = 0;
                self.cursor_row = (self.cursor_row + n0).min(self.rows.saturating_sub(1));
            }
            'F' => {
                self.cursor_col = 0;
                self.cursor_row = self.cursor_row.saturating_sub(n0);
            }
            'G' | '`' => {
                self.cursor_col = (p0.saturating_sub(1) as usize).min(self.cols.saturating_sub(1));
            }
            'd' => {
                self.cursor_row = (p0.saturating_sub(1) as usize).min(self.rows.saturating_sub(1));
            }
            'H' | 'f' => {
                self.cursor_row = (p0.saturating_sub(1) as usize).min(self.rows.saturating_sub(1));
                self.cursor_col = (p1.saturating_sub(1) as usize).min(self.cols.saturating_sub(1));
            }
            'J' => match p0 {
                0 => {
                    for c in self.cursor_col..self.cols {
                        self.cells[self.cursor_row * self.cols + c] = Cell::default();
                    }
                    let row = self.cursor_row;
                    for r in (row + 1)..self.rows {
                        self.clear_line(r);
                    }
                    self.dirty_rows.insert(row);
                }
                1 => {
                    let row = self.cursor_row;
                    for r in 0..row {
                        self.clear_line(r);
                    }
                    for c in 0..=self.cursor_col {
                        self.cells[row * self.cols + c] = Cell::default();
                    }
                    self.dirty_rows.insert(row);
                }
                2 | 3 => {
                    for r in 0..self.rows {
                        self.clear_line(r);
                    }
                }
                _ => {}
            },
            'K' => match p0 {
                0 => {
                    let (row, col) = (self.cursor_row, self.cursor_col);
                    for c in col..self.cols {
                        self.cells[row * self.cols + c] = Cell::default();
                    }
                    self.dirty_rows.insert(row);
                }
                1 => {
                    let (row, col) = (self.cursor_row, self.cursor_col);
                    for c in 0..=col {
                        self.cells[row * self.cols + c] = Cell::default();
                    }
                    self.dirty_rows.insert(row);
                }
                2 => {
                    let row = self.cursor_row;
                    self.clear_line(row);
                }
                _ => {}
            },
            '@' => self.insert_chars(n0),
            'P' => self.delete_chars(n0),
            'X' => self.erase_chars(n0),
            'L' => self.insert_lines(n0),
            'M' => self.delete_lines(n0),
            'S' => self.scroll_up(n0),
            'T' => self.scroll_down(n0),
            'm' => {
                if ps.is_empty() {
                    self.apply_sgr(&[0]);
                } else {
                    self.apply_sgr(&ps);
                }
            }
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'r' => {
                let top = (p0.saturating_sub(1) as usize).min(self.rows.saturating_sub(1));
                let bot = if p1 == 0 {
                    self.rows - 1
                } else {
                    (p1 as usize).saturating_sub(1)
                };
                self.scroll_top = top;
                self.scroll_bot = bot.min(self.rows.saturating_sub(1));
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // Charset selection (`ESC ( B` etc.) carries an intermediate; ignore it.
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'M' => {
                // Reverse index: up one row, scrolling the region down at the top.
                if self.cursor_row == self.scroll_top {
                    self.scroll_down(1);
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                }
            }
            b'c' => {
                // RIS — full reset. Preserve the configured scrollback cap (a
                // plain `new()` would silently drop back to the default), the
                // pane tag, and the command-id counter. This also wipes the live
                // `command_log`, same as it always has — deliberately *not*
                // treated as a delete (no tombstones emitted for the archive):
                // RIS is a terminal control-state reset a program can trigger
                // (e.g. `reset`), not a user history-management action, so the
                // persisted archive is left alone even though the in-memory
                // window clears, the same "eviction isn't deletion" reasoning
                // `record_output_line`'s `MAX_COMMAND_LOG` trim already uses.
                let pane_tag = self.pane_tag.clone();
                let next_command_id = self.next_command_id;
                *self = TerminalScreen::with_scrollback(self.cols, self.rows, self.max_scrollback);
                self.pane_tag = pane_tag;
                self.next_command_id = next_command_id;
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let Some(kind) = params.first() else { return };
        let arg = |i: usize| {
            params
                .get(i)
                .map(|b| String::from_utf8_lossy(b).into_owned())
        };
        match *kind {
            b"0" | b"2" => {
                if let Some(t) = arg(1) {
                    self.title = Some(t);
                }
            }
            b"7" => {
                // file://host/abs/path — keep just the path.
                if let Some(uri) = arg(1) {
                    let path = uri.split_once("://").map(|(_, rest)| {
                        rest.find('/')
                            .map(|i| &rest[i..])
                            .unwrap_or(rest)
                            .to_string()
                    });
                    self.cwd = path.or(Some(uri));
                }
            }
            b"52" => {
                // 52 ; <selection> ; <base64>  — a clipboard write request.
                if let Some(payload) = params.get(2) {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload) {
                        if let Ok(s) = String::from_utf8(bytes) {
                            self.clipboard = Some(s);
                        }
                    }
                }
            }
            b"133" => {
                // Semantic-prompt marks (FinalTerm / iTerm2 shell integration). The four
                // marks delimit regions: `A` prompt start, `B` command-input start, `C`
                // output start, `D[;code]` finished. We pin each to a global line id (so
                // the position survives scrollback) to drive prompt-jump navigation,
                // failed-command flagging, and output copy; `C`/`D` also feed the
                // completion notification (duration + exit). Command *text* capture stays
                // the Enter heuristic's job (`record_output_line`).
                if let Some(sub) = params.get(1) {
                    let line = self.current_line_id();
                    match *sub {
                        // Prompt boundary: the first of A/B seen pins the prompt line.
                        b"A" | b"B" => {
                            self.pending_mark
                                .get_or_insert_with(PendingMark::default)
                                .prompt_line
                                .get_or_insert(line);
                        }
                        b"C" => {
                            let m = self.pending_mark.get_or_insert_with(PendingMark::default);
                            m.output_start = Some(line);
                            m.started_at = Some(Instant::now());
                        }
                        b"D" => {
                            let exit_code = params
                                .get(2)
                                .and_then(|b| std::str::from_utf8(b).ok())
                                .and_then(|s| s.trim().parse().ok());
                            self.finish_command_mark(line, exit_code);
                        }
                        _ => {}
                    }
                }
            }
            _ => {} // OSC 8 hyperlinks etc.: consumed (no garbage printed), UI is a backlog item.
        }
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
