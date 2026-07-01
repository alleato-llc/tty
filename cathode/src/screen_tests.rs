use super::*;
use crate::parser::TermParser;

fn run(cols: usize, rows: usize, bytes: &[u8]) -> TerminalScreen {
    let mut screen = TerminalScreen::new(cols, rows);
    let mut parser = TermParser::new();
    parser.process(bytes, &mut screen);
    screen
}

fn row_text(screen: &TerminalScreen, row: usize) -> String {
    (0..screen.cols).map(|c| screen.cell(row, c).ch).collect()
}

#[test]
fn plain_ascii_lands_in_cells_and_advances_cursor() {
    let s = run(10, 3, b"abc");
    assert_eq!(s.cell(0, 0).ch, 'a');
    assert_eq!(s.cell(0, 1).ch, 'b');
    assert_eq!(s.cell(0, 2).ch, 'c');
    assert_eq!(s.cursor_row, 0);
    assert_eq!(s.cursor_col, 3);
    assert_eq!(s.cell(0, 3).ch, ' ');
}

#[test]
fn carriage_return_moves_to_column_zero() {
    let s = run(10, 3, b"abc\r");
    assert_eq!(s.cursor_col, 0);
    assert_eq!(s.cursor_row, 0);
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
    let s = run(10, 3, b"ab\ncd");
    assert_eq!(row_text(&s, 0).trim_end(), "ab");
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
    assert_eq!(s.cell(0, 2).ch, 'c');
}

#[test]
fn backspace_at_column_zero_is_clamped() {
    let s = run(10, 3, b"\x08");
    assert_eq!(s.cursor_col, 0);
}

#[test]
fn cursor_position_csi_h_is_one_based() {
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
    let s = run(20, 10, b"\x1b[2B\x1b[3C");
    assert_eq!(s.cursor_row, 2);
    assert_eq!(s.cursor_col, 3);
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
    let s = run(3, 3, b"abcd");
    assert_eq!(row_text(&s, 0), "abc");
    assert_eq!(s.cell(1, 0).ch, 'd');
    assert_eq!(s.cursor_row, 1);
    assert_eq!(s.cursor_col, 1);
}

#[test]
fn sgr_bold_and_color_apply_to_written_cells() {
    let s = run(10, 3, b"\x1b[1;31mX");
    let cell = s.cell(0, 0);
    assert_eq!(cell.ch, 'X');
    assert!(cell.bold);
    assert_eq!(cell.fg, TermColor::Named(1));
}

#[test]
fn sgr_reset_clears_attributes() {
    let s = run(10, 3, b"\x1b[1;3mA\x1b[0mB");
    assert!(s.cell(0, 0).bold);
    assert!(s.cell(0, 0).italic);
    assert!(!s.cell(0, 1).bold);
    assert!(!s.cell(0, 1).italic);
    assert_eq!(s.cell(0, 1).fg, TermColor::Default);
}

#[test]
fn sgr_256_indexed_and_rgb_colors() {
    let s = run(10, 3, b"\x1b[38;5;200mP");
    assert_eq!(s.cell(0, 0).fg, TermColor::Indexed(200));
    let s = run(10, 3, b"\x1b[48;2;10;20;30mQ");
    assert_eq!(s.cell(0, 0).bg, TermColor::Rgb(10, 20, 30));
}

#[test]
fn erase_in_line_to_end_k0() {
    let s = run(10, 3, b"abcde\x1b[1;3H\x1b[0K");
    assert_eq!(s.cell(0, 0).ch, 'a');
    assert_eq!(s.cell(0, 1).ch, 'b');
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
    let mut screen = TerminalScreen::new(5, 2);
    let mut parser = TermParser::new();
    parser.process(b"top\r\nbot", &mut screen);
    assert_eq!(row_text(&screen, 0).trim_end(), "top");
    assert_eq!(row_text(&screen, 1).trim_end(), "bot");
    parser.process(b"\r\nnew", &mut screen);
    assert_eq!(row_text(&screen, 0).trim_end(), "bot");
    assert_eq!(row_text(&screen, 1).trim_end(), "new");
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
    let s = run(10, 8, b"\x1b[3;6Hxx\x1b[2;5r");
    assert_eq!(s.scroll_top, 1);
    assert_eq!(s.scroll_bot, 4);
    assert_eq!(s.cursor_row, 0);
    assert_eq!(s.cursor_col, 0);
}

#[test]
fn resize_preserves_overlap() {
    let mut s = run(10, 3, b"hello");
    s.resize(6, 2);
    assert_eq!(s.cols, 6);
    assert_eq!(s.rows, 2);
    // The overlapping top-left content survives the resize.
    assert_eq!(row_text(&s, 0).trim_end(), "hello");
}

// --- new coverage -----------------------------------------------------

#[test]
fn tab_advances_to_next_stop() {
    let s = run(40, 2, b"a\tb");
    assert_eq!(s.cell(0, 0).ch, 'a');
    assert_eq!(s.cell(0, 8).ch, 'b');
}

#[test]
fn wide_char_occupies_two_cells_with_spacer() {
    let s = run(10, 2, "你x".as_bytes());
    assert_eq!(s.cell(0, 0).ch, '你');
    assert_eq!(s.cell(0, 0).width, 2);
    assert_eq!(s.cell(0, 1).width, 0); // spacer
    assert_eq!(s.cell(0, 2).ch, 'x');
    assert_eq!(s.cursor_col, 3);
}

#[test]
fn bracketed_paste_and_app_cursor_modes_toggle() {
    let mut s = run(10, 2, b"\x1b[?2004h\x1b[?1h");
    assert!(s.bracketed_paste);
    assert!(s.app_cursor_keys);
    let mut parser = TermParser::new();
    parser.process(b"\x1b[?2004l", &mut s);
    assert!(!s.bracketed_paste);
}

#[test]
fn mouse_modes_set_tracking_and_encoding() {
    let s = run(10, 2, b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(s.mouse.tracking, MouseTracking::Normal);
    assert!(s.mouse.sgr);
    assert!(s.mouse.reports());
}

#[test]
fn cursor_shape_from_decscusr() {
    let s = run(10, 2, b"\x1b[5 q");
    assert_eq!(s.cursor_shape, CursorShape::Bar);
    assert!(s.cursor_blink);
    let s = run(10, 2, b"\x1b[2 q");
    assert_eq!(s.cursor_shape, CursorShape::Block);
    assert!(!s.cursor_blink);
}

#[test]
fn cursor_visibility_toggles() {
    let s = run(10, 2, b"\x1b[?25l");
    assert!(!s.cursor_visible);
    let s = run(10, 2, b"\x1b[?25l\x1b[?25h");
    assert!(s.cursor_visible);
}

#[test]
fn alt_screen_isolates_and_restores_main() {
    // Write to main, enter alt, write there, leave alt → main content returns and
    // alt content never reached scrollback.
    let mut s = run(10, 3, b"main");
    let mut parser = TermParser::new();
    parser.process(b"\x1b[?1049h", &mut s);
    assert!(s.alt_screen());
    parser.process(b"ALT", &mut s);
    assert_eq!(row_text(&s, 0).trim_end(), "ALT");
    parser.process(b"\x1b[?1049l", &mut s);
    assert!(!s.alt_screen());
    assert_eq!(row_text(&s, 0).trim_end(), "main");
    assert!(
        s.scrollback.is_empty(),
        "alt content must not enter scrollback"
    );
}

#[test]
fn osc_sets_title_and_cwd() {
    let s = run(10, 2, b"\x1b]0;my title\x07\x1b]7;file://host/Users/me\x07");
    assert_eq!(s.title.as_deref(), Some("my title"));
    assert_eq!(s.cwd.as_deref(), Some("/Users/me"));
}

#[test]
fn osc52_decodes_clipboard() {
    // base64("hi") == "aGk=".
    let mut s = run(10, 2, b"\x1b]52;c;aGk=\x07");
    assert_eq!(s.take_clipboard().as_deref(), Some("hi"));
    assert_eq!(s.take_clipboard(), None);
}

#[test]
fn bell_flag_reads_and_clears() {
    let mut s = run(10, 2, b"\x07");
    assert!(s.take_bell());
    assert!(!s.take_bell());
}

#[test]
fn insert_and_delete_chars_shift_the_row() {
    // "abcd", home, insert 2 → "  ab" then "cd" pushed right.
    let s = run(6, 2, b"abcd\x1b[1;1H\x1b[2@");
    assert_eq!(
        row_text(&s, 0),
        "  abcd".chars().take(6).collect::<String>()
    );
    // delete 2 at home → "cdef"-style left shift.
    let s = run(6, 2, b"abcdef\x1b[1;1H\x1b[2P");
    assert_eq!(&row_text(&s, 0)[..4], "cdef");
}

#[test]
fn insert_delete_lines_within_region() {
    let s = run(4, 3, b"aaa\r\nbbb\r\nccc\x1b[1;1H\x1b[1L");
    // A blank line was inserted at the top; "aaa" pushed down.
    assert_eq!(row_text(&s, 0).trim_end(), "");
    assert_eq!(row_text(&s, 1).trim_end(), "aaa");
}

#[test]
fn save_and_restore_cursor() {
    let s = run(20, 5, b"\x1b[3;4H\x1b7\x1b[1;1H\x1b8");
    assert_eq!(s.cursor_row, 2);
    assert_eq!(s.cursor_col, 3);
}

#[test]
fn resize_clamps_cursor() {
    let mut s = run(10, 5, b"\x1b[5;9Hhello");
    assert_eq!(s.cursor_row, 4);
    s.resize(4, 2);
    assert!(s.cursor_row < 2);
    assert!(s.cursor_col < 4);
}
