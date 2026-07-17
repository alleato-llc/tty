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
fn osc133_reports_a_finished_command_with_exit_code() {
    // C (start) → the command runs → D;3 (finished, exit 3). One completion, drained.
    let mut s = TerminalScreen::new(20, 3);
    let mut p = TermParser::new();
    p.process(b"\x1b]133;C\x07", &mut s);
    p.process(b"boom\r\n", &mut s);
    p.process(b"\x1b]133;D;3\x07", &mut s);
    let done = s.take_command_completions();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].exit_code, Some(3));
    assert_eq!(s.take_command_completions(), vec![], "drained");
}

#[test]
fn osc133_d_without_a_c_is_ignored() {
    // An empty Enter emits D with no preceding C — nothing to report.
    let mut s = run(20, 2, b"\x1b]133;D\x07");
    assert!(s.take_command_completions().is_empty());
}

#[test]
fn osc133_d_without_code_reports_none() {
    let mut s = TerminalScreen::new(20, 2);
    let mut p = TermParser::new();
    p.process(b"\x1b]133;C\x07", &mut s);
    p.process(b"\x1b]133;D\x07", &mut s);
    let done = s.take_command_completions();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].exit_code, None);
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

#[test]
fn scrollback_evicts_at_the_configured_cap() {
    let mut screen = TerminalScreen::with_scrollback(5, 1, 3);
    let mut parser = TermParser::new();
    // Five newlines past the single row push five lines into scrollback; only the
    // last 3 should survive.
    for n in 0..5 {
        parser.process(format!("{n}\r\n").as_bytes(), &mut screen);
    }
    assert_eq!(screen.scrollback.len(), 3);
    let lines: Vec<String> = screen
        .scrollback
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    assert_eq!(lines, vec!["2", "3", "4"]);
}

#[test]
fn set_max_scrollback_truncates_a_fuller_buffer() {
    let mut screen = TerminalScreen::with_scrollback(5, 1, 10);
    let mut parser = TermParser::new();
    for n in 0..6 {
        parser.process(format!("{n}\r\n").as_bytes(), &mut screen);
    }
    assert_eq!(screen.scrollback.len(), 6);
    screen.set_max_scrollback(2);
    assert_eq!(screen.scrollback.len(), 2);
    let lines: Vec<String> = screen
        .scrollback
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    // The oldest lines were dropped from the front — the newest 2 remain.
    assert_eq!(lines, vec!["4", "5"]);
}

#[test]
fn clear_scrollback_empties_it_without_touching_the_live_grid() {
    let mut screen = TerminalScreen::with_scrollback(5, 2, 10);
    let mut parser = TermParser::new();
    parser.process(b"a\r\nb\r\nc", &mut screen);
    assert!(!screen.scrollback.is_empty());
    let before = row_text(&screen, 1);
    screen.clear_scrollback();
    assert!(screen.scrollback.is_empty());
    assert_eq!(screen.oldest_scrollback_age(), None);
    assert_eq!(row_text(&screen, 1), before, "live grid is untouched");
}

#[test]
fn transcript_lines_orders_scrollback_before_the_live_grid() {
    let mut screen = TerminalScreen::with_scrollback(5, 1, 10);
    let mut parser = TermParser::new();
    parser.process(b"a\r\nb\r\nc", &mut screen);
    // "a" and "b" scrolled off into scrollback; "c" is the live row.
    assert_eq!(screen.transcript_lines(), vec!["a", "b", "c"]);
}

#[test]
fn transcript_lines_excludes_the_live_grid_while_on_the_alt_screen() {
    // Regression: a full-screen app (htop, vim, less) runs on the alt screen, whose
    // live content must not leak into "history" any more than it leaks into
    // scrollback — even though `transcript_lines` always used to append it.
    let mut screen = TerminalScreen::with_scrollback(10, 3, 10);
    let mut parser = TermParser::new();
    parser.process(b"$ htop", &mut screen);
    assert_eq!(screen.transcript_lines(), vec!["$ htop", "", ""]);
    parser.process(b"\x1b[?1049h", &mut screen);
    parser.process(b"1 [||    ] 12%", &mut screen);
    assert!(screen.alt_screen());
    assert_eq!(
        screen.transcript_lines(),
        Vec::<String>::new(),
        "htop's live dashboard must not appear as history"
    );
    // Leaving the alt screen restores the pre-htop transcript.
    parser.process(b"\x1b[?1049l", &mut screen);
    assert_eq!(screen.transcript_lines(), vec!["$ htop", "", ""]);
}

#[test]
fn oldest_scrollback_age_is_some_once_something_has_scrolled() {
    let mut screen = TerminalScreen::with_scrollback(5, 1, 10);
    assert_eq!(screen.oldest_scrollback_age(), None, "nothing buffered yet");
    let mut parser = TermParser::new();
    parser.process(b"a\r\nb", &mut screen);
    assert!(screen.oldest_scrollback_age().is_some());
}

#[test]
fn ris_reset_preserves_the_configured_scrollback_cap() {
    let mut screen = TerminalScreen::with_scrollback(5, 1, 3);
    let mut parser = TermParser::new();
    for n in 0..5 {
        parser.process(format!("{n}\r\n").as_bytes(), &mut screen);
    }
    assert_eq!(screen.scrollback.len(), 3);
    parser.process(b"\x1bc", &mut screen); // RIS
    assert!(
        screen.scrollback.is_empty(),
        "RIS clears the buffer as before"
    );
    for n in 0..5 {
        parser.process(format!("{n}\r\n").as_bytes(), &mut screen);
    }
    assert_eq!(screen.scrollback.len(), 3, "the cap survived the reset");
}

#[test]
fn mark_command_boundary_captures_the_current_row_as_the_command() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ls -la", &mut screen);
    screen.mark_command_boundary(50);
    // Queued, not yet resolved — the entry is created once the boundary's own line
    // is seen completing (the shell echoing Enter), not at the moment it's marked.
    assert!(screen.command_log.is_empty());
    parser.process(b"\r\n", &mut screen);
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "$ ls -la");
    assert!(screen.command_log[0].output.is_empty());
}

#[test]
fn output_after_a_boundary_accumulates_into_that_commands_entry_not_the_command_itself() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    // The shell echoes Enter as \r\n (finalizing the command's own row — must NOT
    // become an "output" line), then the real output, then the next (unfinished)
    // prompt — which also must not be captured as output yet.
    parser.process(b"\r\na.txt\r\nb.txt\r\n$ ", &mut screen);
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "$ ls");
    assert_eq!(screen.command_log[0].output, vec!["a.txt", "b.txt"]);
}

#[test]
fn a_second_boundary_starts_a_new_entry() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\na.txt\r\n$ pwd", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\n/home/user\r\n$ ", &mut screen);
    assert_eq!(screen.command_log.len(), 2);
    assert_eq!(screen.command_log[0].command, "$ ls");
    assert_eq!(screen.command_log[0].output, vec!["a.txt"]);
    assert_eq!(screen.command_log[1].command, "$ pwd");
    assert_eq!(screen.command_log[1].output, vec!["/home/user"]);
}

#[test]
fn output_stops_growing_past_its_per_command_cap() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ tail -f x.log", &mut screen);
    screen.mark_command_boundary(2); // a tight cap, as if resolved from an override
    parser.process(b"\r\nline1\r\nline2\r\nline3\r\nline4\r\n", &mut screen);
    assert_eq!(screen.command_log[0].output, vec!["line1", "line2"]);
    assert!(screen.command_log[0].is_truncated());
}

#[test]
fn mark_command_boundary_is_a_no_op_on_the_alt_screen() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"\x1b[?1049h", &mut screen); // enter alt (htop, vim, ...)
    assert!(screen.alt_screen());
    screen.mark_command_boundary(50);
    assert!(
        screen.command_log.is_empty(),
        "a full-screen app isn't a recordable command"
    );
}

#[test]
fn clear_scrollback_also_clears_the_command_log() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\n", &mut screen);
    assert_eq!(screen.command_log.len(), 1);
    screen.clear_scrollback();
    assert!(screen.command_log.is_empty());
}

#[test]
fn clear_scrollback_discards_a_boundary_queued_but_not_yet_resolved() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ls", &mut screen);
    screen.mark_command_boundary(50); // queued, not yet resolved
    screen.clear_scrollback();
    // If the queued boundary survived the clear, this echo would resurrect an entry.
    parser.process(b"\r\na.txt\r\n", &mut screen);
    assert!(
        screen.command_log.is_empty(),
        "a boundary queued before a clear shouldn't resurrect an entry after it"
    );
}

#[test]
fn mark_command_boundary_with_uses_the_given_text_not_the_screen() {
    let mut screen = TerminalScreen::new(20, 5);
    // Nothing has been drawn to the row yet — the caller already knows the text
    // (a pasted line, known before it's ever sent to the shell).
    screen.mark_command_boundary_with("echo one".to_string(), 50);
    assert!(screen.command_log.is_empty(), "queued, not yet resolved");
    let mut parser = TermParser::new();
    parser.process(b"echo one\r\n", &mut screen); // the shell echoing it back
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "echo one");
}

#[test]
fn queued_boundaries_from_a_multiline_paste_resolve_in_order_around_real_output() {
    // Mirrors an unbracketed multi-line paste: the app queues one boundary per
    // complete pasted line, all before any of it is echoed — so a naive "the next
    // completing row always resolves the front boundary" would wrongly consume the
    // second boundary on "one" (cmd1's own output), not "echo two"'s echo. Matching
    // on the known text is what keeps them straight.
    let mut screen = TerminalScreen::new(20, 5);
    screen.mark_command_boundary_with("echo one".to_string(), 50);
    screen.mark_command_boundary_with("echo two".to_string(), 50);
    assert!(screen.command_log.is_empty());
    let mut parser = TermParser::new();
    parser.process(b"echo one\r\none\r\necho two\r\ntwo\r\n$ ", &mut screen);
    assert_eq!(screen.command_log.len(), 2);
    assert_eq!(screen.command_log[0].command, "echo one");
    assert_eq!(screen.command_log[0].output, vec!["one"]);
    assert_eq!(screen.command_log[1].command, "echo two");
    assert_eq!(screen.command_log[1].output, vec!["two"]);
}

#[test]
fn a_paste_boundary_matches_the_full_prompt_row_not_just_the_pasted_text() {
    // The known text is just the pasted line ("pwd"), but the row it's echoed onto
    // also carries the prompt ("$ ") — the match has to look for the pasted text at
    // the *end* of the row, not equal the whole row.
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    parser.process(b"$ ", &mut screen); // the prompt, already on screen
    screen.mark_command_boundary_with("pwd".to_string(), 50);
    parser.process(b"pwd\r\n/home/user\r\n", &mut screen);
    assert_eq!(screen.command_log.len(), 1);
    assert_eq!(screen.command_log[0].command, "pwd");
    assert_eq!(screen.command_log[0].output, vec!["/home/user"]);
}

#[test]
fn current_row_text_reflects_in_progress_edits() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    // Backspace-edit before submitting: "gti" -> backspace x3 -> "git status".
    parser.process(b"$ gti\x08\x08\x08git status", &mut screen);
    assert_eq!(screen.current_row_text(), "$ git status");
}

// --- Persisted-history event queue ---

/// Run a command to completion (boundary marked, then the shell's echo of it),
/// mirroring the real call order (`mark_command_boundary` right before the
/// Enter bytes) that every other boundary test in this file already uses.
fn run_command(screen: &mut TerminalScreen, parser: &mut TermParser, command: &str) {
    parser.process(command.as_bytes(), screen);
    screen.mark_command_boundary(50);
    parser.process(b"\r\n", screen);
}

#[test]
fn a_new_command_queues_an_upsert_event() {
    let mut screen = TerminalScreen::new(20, 5);
    screen.set_pane_tag("Tab 1".to_string());
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");

    let events = screen.take_pending_history_events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        HistoryEvent::Upsert(p) => {
            assert_eq!(p.command, "echo hi");
            assert_eq!(p.pane_tag, "Tab 1");
            assert_eq!(p.id, screen.command_log[0].id);
        }
        other => panic!("expected Upsert, got {other:?}"),
    }
    // Draining clears it.
    assert!(screen.take_pending_history_events().is_empty());
}

#[test]
fn clear_command_output_queues_a_superseding_upsert_with_blanked_command() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    let id = screen.command_log[0].id;
    screen.take_pending_history_events(); // drain the original Upsert

    screen.clear_command_output(0);
    let events = screen.take_pending_history_events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        HistoryEvent::Upsert(p) => {
            assert_eq!(p.id, id, "supersedes the same id, doesn't mint a new one");
            assert_eq!(p.command, "", "blanked, matching the in-memory clear");
        }
        other => panic!("expected a superseding Upsert, got {other:?}"),
    }
}

#[test]
fn clear_command_output_line_queues_no_history_event() {
    // Output is never persisted, so blanking one output line has nothing to
    // reflect in the archive.
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    parser.process(b"hi\r\n", &mut screen); // captured output
    screen.take_pending_history_events();

    screen.clear_command_output_line(0, 0);
    assert!(screen.take_pending_history_events().is_empty());
}

#[test]
fn remove_command_queues_a_tombstone_for_its_id() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    let id = screen.command_log[0].id;
    screen.take_pending_history_events();

    screen.remove_command(0);
    let events = screen.take_pending_history_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], HistoryEvent::Tombstone { id: t, .. } if t == id));
}

#[test]
fn clear_scrollback_queues_a_tombstone_per_command() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo one");
    run_command(&mut screen, &mut parser, "echo two");
    let ids: Vec<u32> = screen.command_log.iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 2);
    screen.take_pending_history_events();

    screen.clear_scrollback();
    let events = screen.take_pending_history_events();
    let tombstoned: Vec<u32> = events
        .iter()
        .map(|e| match e {
            HistoryEvent::Tombstone { id, .. } => *id,
            other => panic!("expected only Tombstones, got {other:?}"),
        })
        .collect();
    assert_eq!(tombstoned, ids);
}

#[test]
fn command_log_eviction_past_the_cap_does_not_queue_a_tombstone() {
    // Falling off the live in-memory window is not a delete: the archived copy
    // (if any) must be untouched, so no Tombstone should ever be queued for it.
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    for _ in 0..(MAX_COMMAND_LOG + 5) {
        run_command(&mut screen, &mut parser, "x");
    }
    assert_eq!(screen.command_log.len(), MAX_COMMAND_LOG);

    let events = screen.take_pending_history_events();
    assert_eq!(
        events.len(),
        MAX_COMMAND_LOG + 5,
        "one Upsert per command, no evictions"
    );
    assert!(
        events.iter().all(|e| matches!(e, HistoryEvent::Upsert(_))),
        "eviction must never itself queue a Tombstone"
    );
}

#[test]
fn seed_command_log_queues_no_events_and_advances_the_id_counter_past_loaded_ids() {
    let mut screen = TerminalScreen::new(20, 5);
    screen.seed_command_log(vec![
        PersistedCommandEntry {
            id: 3,
            command: "ls".to_string(),
            started_at_epoch_ms: 1_750_000_000_000,
            pane_tag: "Tab 1".to_string(),
        },
        PersistedCommandEntry {
            id: 9,
            command: "pwd".to_string(),
            started_at_epoch_ms: 1_750_000_001_000,
            pane_tag: "Tab 1".to_string(),
        },
    ]);
    assert_eq!(screen.command_log.len(), 2);
    assert!(
        screen.take_pending_history_events().is_empty(),
        "loading is the reverse of a mutation, not one"
    );

    // A new command's id must not collide with the highest loaded id (9).
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    let new_id = screen.command_log.back().unwrap().id;
    assert!(
        new_id > 9,
        "got id {new_id}, expected something past the loaded max"
    );
}

#[test]
fn an_untracked_screen_queues_no_history_events_ever() {
    let mut screen = TerminalScreen::new(20, 5);
    screen.set_untracked(true);
    let mut parser = TermParser::new();

    // Recording a command: live log yes, history event no.
    run_command(&mut screen, &mut parser, "echo hi");
    assert_eq!(screen.command_log.len(), 1, "the live log still works");
    assert!(
        screen.take_pending_history_events().is_empty(),
        "recording on an untracked screen must queue nothing"
    );

    // The mutation paths queue nothing either.
    screen.clear_command_output(0);
    screen.remove_command(0);
    run_command(&mut screen, &mut parser, "echo again");
    screen.clear_scrollback();
    assert!(
        screen.take_pending_history_events().is_empty(),
        "clear/remove/wipe on an untracked screen must queue nothing"
    );
}

#[test]
fn command_entries_carry_the_untracked_flag() {
    let mut screen = TerminalScreen::new(20, 5);
    screen.set_untracked(true);
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    assert!(screen.command_log[0].untracked);

    let mut tracked = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();
    run_command(&mut tracked, &mut parser, "echo hi");
    assert!(!tracked.command_log[0].untracked);

    // Seeded (archived) entries are by definition tracked.
    tracked.seed_command_log(vec![PersistedCommandEntry {
        id: 9,
        command: "ls".to_string(),
        started_at_epoch_ms: 1_750_000_000_000,
        pane_tag: "Tab 1".to_string(),
    }]);
    assert!(!tracked.command_log.back().unwrap().untracked);
}

#[test]
fn reserve_command_ids_advances_but_never_regresses() {
    let mut screen = TerminalScreen::new(20, 5);
    let mut parser = TermParser::new();

    // A floor set on a fresh screen: the next command starts there.
    screen.reserve_command_ids(40);
    run_command(&mut screen, &mut parser, "echo hi");
    assert_eq!(screen.command_log.back().unwrap().id, 40);

    // A lower floor after commands have run must not roll the counter back
    // (that would mint colliding ids).
    screen.reserve_command_ids(3);
    run_command(&mut screen, &mut parser, "echo again");
    assert_eq!(screen.command_log.back().unwrap().id, 41);
}

#[test]
fn ris_reset_preserves_pane_tag_and_does_not_tombstone_the_wiped_log() {
    let mut screen = TerminalScreen::new(20, 5);
    screen.set_pane_tag("Tab 1".to_string());
    let mut parser = TermParser::new();
    run_command(&mut screen, &mut parser, "echo hi");
    assert_eq!(screen.command_log.len(), 1, "sanity: something to wipe");
    screen.take_pending_history_events();

    parser.process(b"\x1bc", &mut screen); // RIS
    assert!(
        screen.command_log.is_empty(),
        "RIS still wipes the live log"
    );
    assert_eq!(
        screen.pane_tag, "Tab 1",
        "but the pane tag survives a control-state reset"
    );
    assert!(
        screen.take_pending_history_events().is_empty(),
        "RIS is a terminal control-state reset, not a user delete — the \
         archived copy must be left alone"
    );
}
