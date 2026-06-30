use std::collections::BTreeSet;
use std::collections::VecDeque;

const DEFAULT_SCROLLBACK: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermColor {
    Default,
    Named(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
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
        }
    }
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
    pub dirty_rows: BTreeSet<usize>,
    fg: TermColor,
    bg: TermColor,
    bold: bool,
    italic: bool,
    underline: bool,
    dim: bool,
    inverse: bool,
}

impl TerminalScreen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = vec![Cell::default(); cols * rows];
        Self {
            cols,
            rows,
            cells,
            cursor_row: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_bot: rows.saturating_sub(1),
            scrollback: VecDeque::with_capacity(DEFAULT_SCROLLBACK),
            dirty_rows: BTreeSet::new(),
            fg: TermColor::Default,
            bg: TermColor::Default,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            inverse: false,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.cells = vec![Cell::default(); cols * rows];
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.scroll_bot = rows.saturating_sub(1);
        for r in 0..rows {
            self.dirty_rows.insert(r);
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }

    fn write_char_at_cursor(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.advance_row();
        }
        let (row, col) = (self.cursor_row, self.cursor_col);
        // Copy SGR state before the mutable borrow of cells.
        let new_cell = Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            dim: self.dim,
            inverse: self.inverse,
        };
        self.cells[row * self.cols + col] = new_cell;
        self.dirty_rows.insert(row);
        self.cursor_col += 1;
    }

    fn advance_row(&mut self) {
        if self.cursor_row >= self.scroll_bot {
            self.scroll_up(1);
        } else {
            self.cursor_row += 1;
        }
    }

    pub fn scroll_up(&mut self, count: usize) {
        for _ in 0..count {
            let top_row: Vec<Cell> = (0..self.cols)
                .map(|c| self.cell(self.scroll_top, c).clone())
                .collect();
            if self.scrollback.len() >= DEFAULT_SCROLLBACK {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(top_row);
            for r in self.scroll_top..self.scroll_bot {
                for c in 0..self.cols {
                    self.cells[r * self.cols + c] = self.cells[(r + 1) * self.cols + c].clone();
                }
                self.dirty_rows.insert(r);
            }
            for c in 0..self.cols {
                self.cells[self.scroll_bot * self.cols + c] = Cell::default();
            }
            self.dirty_rows.insert(self.scroll_bot);
        }
    }

    fn clear_line(&mut self, row: usize) {
        for c in 0..self.cols {
            self.cells[row * self.cols + c] = Cell::default();
        }
        self.dirty_rows.insert(row);
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
        self.write_char_at_cursor(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.cursor_col = 0,
            b'\n' | 0x0B | 0x0C => {
                if self.cursor_row == self.scroll_bot {
                    self.scroll_up(1);
                } else {
                    self.cursor_row = (self.cursor_row + 1).min(self.rows.saturating_sub(1));
                    self.dirty_rows.insert(self.cursor_row);
                }
            }
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            0x07 => {}
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let ps: Vec<u16> = params.iter().map(|p| p[0]).collect();
        let p0 = *ps.first().unwrap_or(&0);
        let p1 = *ps.get(1).unwrap_or(&0);

        match action {
            'A' => {
                let n = p0.max(1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = p0.max(1) as usize;
                self.cursor_row = (self.cursor_row + n).min(self.rows.saturating_sub(1));
            }
            'C' => {
                let n = p0.max(1) as usize;
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let n = p0.max(1) as usize;
                self.cursor_col = self.cursor_col.saturating_sub(n);
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
                    let row = self.cursor_row;
                    let col = self.cursor_col;
                    for c in col..self.cols {
                        self.cells[row * self.cols + c] = Cell::default();
                    }
                    self.dirty_rows.insert(row);
                }
                1 => {
                    let row = self.cursor_row;
                    let col = self.cursor_col;
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
            'm' => {
                if ps.is_empty() {
                    self.apply_sgr(&[0]);
                } else {
                    self.apply_sgr(&ps);
                }
            }
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

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TermParser;

    /// Drive `bytes` through the real vte parser into a fresh screen.
    fn run(cols: usize, rows: usize, bytes: &[u8]) -> TerminalScreen {
        let mut screen = TerminalScreen::new(cols, rows);
        let mut parser = TermParser::new();
        parser.process(bytes, &mut screen);
        screen
    }

    /// Collect a row's characters into a String for easy assertions.
    fn row_text(screen: &TerminalScreen, row: usize) -> String {
        (0..screen.cols).map(|c| screen.cell(row, c).ch).collect()
    }

    #[test]
    fn plain_ascii_lands_in_cells_and_advances_cursor() {
        let s = run(10, 3, b"abc");
        assert_eq!(s.cell(0, 0).ch, 'a');
        assert_eq!(s.cell(0, 1).ch, 'b');
        assert_eq!(s.cell(0, 2).ch, 'c');
        // Cursor advanced past the last written cell.
        assert_eq!(s.cursor_row, 0);
        assert_eq!(s.cursor_col, 3);
        // Untouched cells remain spaces.
        assert_eq!(s.cell(0, 3).ch, ' ');
    }

    #[test]
    fn carriage_return_moves_to_column_zero() {
        let s = run(10, 3, b"abc\r");
        assert_eq!(s.cursor_col, 0);
        assert_eq!(s.cursor_row, 0);
        // CR alone does not erase what was written.
        assert_eq!(s.cell(0, 0).ch, 'a');
    }

    #[test]
    fn carriage_return_then_write_overwrites() {
        let s = run(10, 3, b"abc\rX");
        assert_eq!(s.cell(0, 0).ch, 'X');
        assert_eq!(s.cell(0, 1).ch, 'b');
        assert_eq!(s.cursor_col, 1);
    }

    #[test]
    fn newline_advances_row_keeping_column() {
        // vte treats lone \n as line feed: row down, column unchanged.
        let s = run(10, 3, b"ab\ncd");
        assert_eq!(row_text(&s, 0).trim_end(), "ab");
        // After "ab" cursor is at col 2; \n moves to row 1 col 2; "cd" lands there.
        assert_eq!(s.cell(1, 2).ch, 'c');
        assert_eq!(s.cell(1, 3).ch, 'd');
        assert_eq!(s.cursor_row, 1);
    }

    #[test]
    fn crlf_returns_to_column_zero_on_next_row() {
        let s = run(10, 3, b"ab\r\ncd");
        assert_eq!(row_text(&s, 0).trim_end(), "ab");
        assert_eq!(row_text(&s, 1).trim_end(), "cd");
        assert_eq!(s.cursor_row, 1);
        assert_eq!(s.cursor_col, 2);
    }

    #[test]
    fn backspace_moves_cursor_left_without_erasing() {
        let s = run(10, 3, b"abc\x08");
        assert_eq!(s.cursor_col, 2);
        // Backspace alone leaves the glyph in place.
        assert_eq!(s.cell(0, 2).ch, 'c');
    }

    #[test]
    fn backspace_at_column_zero_is_clamped() {
        let s = run(10, 3, b"\x08");
        assert_eq!(s.cursor_col, 0);
    }

    #[test]
    fn cursor_position_csi_h_is_one_based() {
        // ESC[2;5H → row 2, col 5 (1-based) → (1, 4) zero-based.
        let s = run(20, 10, b"\x1b[2;5H");
        assert_eq!(s.cursor_row, 1);
        assert_eq!(s.cursor_col, 4);
    }

    #[test]
    fn cursor_position_then_write_lands_at_target() {
        let s = run(20, 10, b"\x1b[3;7HZ");
        assert_eq!(s.cell(2, 6).ch, 'Z');
    }

    #[test]
    fn cursor_position_no_params_homes() {
        // ESC[H with no params → home (0,0).
        let s = run(20, 10, b"\x1b[5;5H\x1b[H");
        assert_eq!(s.cursor_row, 0);
        assert_eq!(s.cursor_col, 0);
    }

    #[test]
    fn cursor_position_clamps_to_bounds() {
        let s = run(8, 4, b"\x1b[99;99H");
        assert_eq!(s.cursor_row, 3);
        assert_eq!(s.cursor_col, 7);
    }

    #[test]
    fn cursor_movement_relative() {
        // Start at (0,0); down 2, right 3.
        let s = run(20, 10, b"\x1b[2B\x1b[3C");
        assert_eq!(s.cursor_row, 2);
        assert_eq!(s.cursor_col, 3);
        // Up 1, left 1.
        let s = run(20, 10, b"\x1b[2B\x1b[3C\x1b[1A\x1b[1D");
        assert_eq!(s.cursor_row, 1);
        assert_eq!(s.cursor_col, 2);
    }

    #[test]
    fn cursor_up_saturates_at_top() {
        let s = run(20, 10, b"\x1b[5A");
        assert_eq!(s.cursor_row, 0);
    }

    #[test]
    fn line_wraps_at_right_edge() {
        // 3 columns: "abcd" → "abc" on row 0, "d" wraps to row 1.
        let s = run(3, 3, b"abcd");
        assert_eq!(row_text(&s, 0), "abc");
        assert_eq!(s.cell(1, 0).ch, 'd');
        assert_eq!(s.cursor_row, 1);
        assert_eq!(s.cursor_col, 1);
    }

    #[test]
    fn sgr_bold_and_color_apply_to_written_cells() {
        // ESC[1;31m sets bold + red (named 1), then 'X'.
        let s = run(10, 3, b"\x1b[1;31mX");
        let cell = s.cell(0, 0);
        assert_eq!(cell.ch, 'X');
        assert!(cell.bold);
        assert_eq!(cell.fg, TermColor::Named(1));
    }

    #[test]
    fn sgr_reset_clears_attributes() {
        // Set bold+italic, write 'A', reset, write 'B'.
        let s = run(10, 3, b"\x1b[1;3mA\x1b[0mB");
        assert!(s.cell(0, 0).bold);
        assert!(s.cell(0, 0).italic);
        assert!(!s.cell(0, 1).bold);
        assert!(!s.cell(0, 1).italic);
        assert_eq!(s.cell(0, 1).fg, TermColor::Default);
    }

    #[test]
    fn sgr_256_indexed_and_rgb_colors() {
        // ESC[38;5;200m → indexed fg 200.
        let s = run(10, 3, b"\x1b[38;5;200mP");
        assert_eq!(s.cell(0, 0).fg, TermColor::Indexed(200));
        // ESC[48;2;10;20;30m → rgb bg.
        let s = run(10, 3, b"\x1b[48;2;10;20;30mQ");
        assert_eq!(s.cell(0, 0).bg, TermColor::Rgb(10, 20, 30));
    }

    #[test]
    fn erase_in_line_to_end_k0() {
        // Write "abcde", move cursor to col 2, erase to end of line.
        let s = run(10, 3, b"abcde\x1b[1;3H\x1b[0K");
        assert_eq!(s.cell(0, 0).ch, 'a');
        assert_eq!(s.cell(0, 1).ch, 'b');
        // From the cursor (col 2) onward cleared.
        assert_eq!(s.cell(0, 2).ch, ' ');
        assert_eq!(s.cell(0, 3).ch, ' ');
        assert_eq!(s.cell(0, 4).ch, ' ');
    }

    #[test]
    fn erase_in_display_all_j2() {
        let s = run(5, 3, b"abc\r\ndef\x1b[2J");
        for r in 0..3 {
            assert_eq!(row_text(&s, r), "     ");
        }
    }

    #[test]
    fn newline_at_bottom_scrolls_up() {
        // 2-row screen: fill row0, newline pushes content up.
        let mut screen = TerminalScreen::new(5, 2);
        let mut parser = TermParser::new();
        parser.process(b"top\r\nbot", &mut screen);
        assert_eq!(row_text(&screen, 0).trim_end(), "top");
        assert_eq!(row_text(&screen, 1).trim_end(), "bot");
        // Now cursor is on the last row; a newline must scroll.
        parser.process(b"\r\nnew", &mut screen);
        assert_eq!(row_text(&screen, 0).trim_end(), "bot");
        assert_eq!(row_text(&screen, 1).trim_end(), "new");
        // The scrolled-off line landed in scrollback.
        assert_eq!(
            screen.scrollback.back().map(|r| r
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()),
            Some("top".to_string())
        );
    }

    #[test]
    fn set_scroll_region_resets_cursor() {
        // ESC[2;5r sets scroll region rows 2..5 (1-based) and homes cursor.
        let s = run(10, 8, b"\x1b[3;6Hxx\x1b[2;5r");
        assert_eq!(s.scroll_top, 1);
        assert_eq!(s.scroll_bot, 4);
        assert_eq!(s.cursor_row, 0);
        assert_eq!(s.cursor_col, 0);
    }

    #[test]
    fn resize_clamps_cursor_and_clears_cells() {
        let mut s = run(10, 5, b"\x1b[5;9Hhello");
        assert_eq!(s.cursor_row, 4);
        s.resize(4, 2);
        assert_eq!(s.cols, 4);
        assert_eq!(s.rows, 2);
        // Cursor clamped into the smaller grid.
        assert!(s.cursor_row < 2);
        assert!(s.cursor_col < 4);
        // Fresh grid is blank.
        assert_eq!(s.cell(0, 0).ch, ' ');
    }
}
