use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use base64::Engine as _;
use unicode_width::UnicodeWidthChar;

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
}

impl CommandEntry {
    /// Whether this command's output hit its cap (there may have been more that
    /// wasn't recorded).
    pub fn is_truncated(&self) -> bool {
        self.output.len() >= self.max_output_lines
    }
}

/// How many commands [`TerminalScreen::mark_command_boundary`] remembers before
/// evicting the oldest — not user-configurable (unlike the per-command output cap),
/// just a generous backstop.
const MAX_COMMAND_LOG: usize = 500;

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

    /// Drop every buffered (off-screen) scrollback line. The live on-screen grid and
    /// cursor are untouched — this is "clear history," not the shell's own `clear`
    /// (which resets the visible screen instead).
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.scrollback_times.clear();
        self.command_log.clear();
        self.pending_boundaries.clear();
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
                self.command_log.push_back(CommandEntry {
                    command: pending.command.unwrap_or_else(|| trimmed.to_string()),
                    output: Vec::new(),
                    started_at: Instant::now(),
                    max_output_lines: pending.max_output_lines,
                });
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
                // plain `new()` would silently drop back to the default).
                *self = TerminalScreen::with_scrollback(self.cols, self.rows, self.max_scrollback);
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
            _ => {} // OSC 8 hyperlinks etc.: consumed (no garbage printed), UI is a backlog item.
        }
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
