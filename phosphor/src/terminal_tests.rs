use super::*;

use cathode::parser::TermParser;

fn screen(cols: usize, rows: usize, bytes: &[u8]) -> TerminalScreen {
    let mut screen = TerminalScreen::with_scrollback(cols, rows, 10);
    let mut parser = TermParser::new();
    parser.process(bytes, &mut screen);
    screen
}

#[test]
fn find_matches_is_empty_for_an_empty_query() {
    let s = screen(10, 2, b"hello");
    assert!(find_matches(&s, s.cols, "").is_empty());
}

#[test]
fn find_matches_is_case_insensitive() {
    let s = screen(10, 2, b"Hello");
    let m = find_matches(&s, s.cols, "hello");
    assert_eq!(m, vec![(0, 0, 5)]);
}

#[test]
fn find_matches_spans_scrollback_and_the_live_grid() {
    // A 5-col, 1-row screen: each `\r\n` scrolls the previous line into scrollback.
    let s = screen(5, 1, b"foo\r\nfoo\r\nfoo");
    // Line 0 and 1 are scrollback, line 2 is the live row — all three "foo"s should
    // be found, not just whatever's currently on screen.
    let m = find_matches(&s, s.cols, "foo");
    assert_eq!(m, vec![(0, 0, 3), (1, 0, 3), (2, 0, 3)]);
}

#[test]
fn find_matches_finds_more_than_one_hit_per_line() {
    let s = screen(20, 1, b"foo bar foo");
    let m = find_matches(&s, s.cols, "foo");
    assert_eq!(m, vec![(0, 0, 3), (0, 8, 11)]);
}

#[test]
fn find_matches_none_when_nothing_matches() {
    let s = screen(10, 2, b"hello");
    assert!(find_matches(&s, s.cols, "xyz").is_empty());
}

fn term(s: TerminalScreen) -> Terminal<()> {
    terminal(
        Arc::new(Mutex::new(s)),
        TerminalStyle::default_dark(),
        Font::MONOSPACE,
        14.0,
        true,
        |_, _| (),
        |_| (),
        |_| (),
        |_| (),
        |_| (),
        |_, _, _| (),
    )
}

#[test]
fn view_top_shows_the_live_bottom_at_zero_scroll() {
    assert_eq!(view_top(100, 24, 0), 76);
}

#[test]
fn view_top_scrolls_the_window_earlier() {
    assert_eq!(view_top(100, 24, 10), 66);
}

#[test]
fn view_top_saturates_instead_of_underflowing() {
    assert_eq!(view_top(10, 24, 0), 0, "fewer total lines than rows");
    assert_eq!(view_top(100, 24, 1000), 0, "scrolled past the start");
}

#[test]
fn cell_at_reads_scrollback_below_history_and_live_above_it() {
    let s = screen(5, 1, b"foo\r\nbar\r\nbaz");
    let history = s.scrollback.len();
    assert_eq!(cell_at(&s, history, 0, 0).ch, 'f', "scrollback line 0");
    assert_eq!(cell_at(&s, history, 1, 0).ch, 'b', "scrollback line 1");
    assert_eq!(cell_at(&s, history, 2, 0).ch, 'b', "the live row");
    assert_eq!(
        cell_at(&s, history, 2, 4).ch,
        ' ',
        "past the written column, still on the live row"
    );
}

#[test]
fn cell_at_past_the_live_screen_is_the_default_cell() {
    let s = screen(5, 1, b"x");
    let history = s.scrollback.len();
    assert_eq!(cell_at(&s, history, 999, 0).ch, Cell::default().ch);
}

#[test]
fn row_chars_collects_one_char_per_column() {
    let s = screen(5, 1, b"hi");
    let history = s.scrollback.len();
    assert_eq!(row_chars(&s, history, 5, 0), vec!['h', 'i', ' ', ' ', ' ']);
}

#[test]
fn order_sorts_anchor_before_head_lexicographically() {
    assert_eq!(order(((0, 5), (2, 1))), ((0, 5), (2, 1)), "already ordered");
    assert_eq!(
        order(((2, 1), (0, 5))),
        ((0, 5), (2, 1)),
        "reversed anchor/head swap back"
    );
    assert_eq!(
        order(((1, 5), (1, 2))),
        ((1, 2), (1, 5)),
        "same line, columns compare"
    );
}

#[test]
fn selected_text_joins_lines_and_trims_trailing_spaces() {
    let s = screen(10, 1, b"foo\r\nbar\r\nbaz");
    let history = s.scrollback.len();
    // Select from (line 0, col 0) through (line 2, col 2): "foo", "bar", "baz".
    let text = selected_text(&s, history, s.cols, ((0, 0), (2, 2)));
    assert_eq!(text, "foo\nbar\nbaz");
}

#[test]
fn selected_text_within_a_single_line_is_just_that_span() {
    let s = screen(10, 1, b"hello world");
    let history = s.scrollback.len();
    let text = selected_text(&s, history, s.cols, ((0, 0), (0, 4)));
    assert_eq!(text, "hello");
}

#[test]
fn hit_maps_a_pixel_position_to_an_absolute_grid_cell() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0));
    // 10px cells: (25, 15) lands in visible row 1, col 2.
    let (line, col) = hit(bounds, 10.0, 10.0, Point::new(25.0, 15.0), 50, 10, 20, 0);
    assert_eq!(col, 2);
    assert_eq!(line, view_top(50, 10, 0) + 1);
}

#[test]
fn hit_clamps_to_the_grid_bounds() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0));
    // Way past the bottom-right corner still clamps to the last row/col.
    let (line, col) = hit(
        bounds,
        10.0,
        10.0,
        Point::new(9999.0, 9999.0),
        50,
        10,
        20,
        0,
    );
    assert_eq!(col, 19);
    assert_eq!(line, view_top(50, 10, 0) + 9);
    // Above/left of the bounds clamps to row/col 0, not a negative/huge value.
    let (line, col) = hit(bounds, 10.0, 10.0, Point::new(-5.0, -5.0), 50, 10, 20, 0);
    assert_eq!((line, col), (view_top(50, 10, 0), 0));
}

#[test]
fn cell_pos_maps_pixels_to_zero_based_col_row() {
    let bounds = Rectangle::new(Point::new(10.0, 10.0), Size::new(100.0, 100.0));
    assert_eq!(cell_pos(bounds, 10.0, 10.0, Point::new(35.0, 24.0)), (2, 1));
    // Before the bounds' origin clamps to 0, not underflowing.
    assert_eq!(cell_pos(bounds, 10.0, 10.0, Point::new(0.0, 0.0)), (0, 0));
}

#[test]
fn mouse_button_maps_left_middle_right_ignores_others() {
    assert_eq!(mouse_button(mouse::Button::Left), Some(MouseButton::Left));
    assert_eq!(
        mouse_button(mouse::Button::Middle),
        Some(MouseButton::Middle)
    );
    assert_eq!(mouse_button(mouse::Button::Right), Some(MouseButton::Right));
    assert_eq!(mouse_button(mouse::Button::Back), None);
}

#[test]
fn resolve_maps_named_indexed_and_rgb_colors() {
    let style = TerminalStyle::default_dark();
    assert_eq!(
        resolve(TermColor::Default, &style, style.fg, false),
        style.fg
    );
    assert_eq!(
        resolve(TermColor::Named(1), &style, style.fg, false),
        style.ansi[1],
        "plain named color"
    );
    assert_eq!(
        resolve(TermColor::Named(1), &style, style.fg, true),
        style.ansi[9],
        "bold brightens 0..8 to 8..16"
    );
    assert_eq!(
        resolve(TermColor::Named(12), &style, style.fg, true),
        style.ansi[12],
        "already-bright named colors are unaffected by bold"
    );
    assert_eq!(
        resolve(TermColor::Rgb(10, 20, 30), &style, style.fg, false),
        Color::from_rgb8(10, 20, 30)
    );
}

#[test]
fn indexed_covers_the_ansi_cube_and_grayscale_ranges() {
    let style = TerminalStyle::default_dark();
    assert_eq!(indexed(3, &style), style.ansi[3], "0..16 is the ANSI table");
    assert_eq!(
        indexed(16, &style),
        Color::from_rgb8(0, 0, 0),
        "cube origin"
    );
    assert_eq!(
        indexed(231, &style),
        Color::from_rgb8(255, 255, 255),
        "cube's far corner"
    );
    let gray = indexed(232, &style);
    assert_eq!(gray, Color::from_rgb8(8, 8, 8), "grayscale ramp start");
}

#[test]
fn line_height_scales_with_font_size() {
    let t = term(TerminalScreen::new(80, 24));
    assert_eq!(t.line_height(), (14.0 * LINE_HEIGHT_RATIO).round());
}

#[test]
fn cell_colors_swaps_fg_and_bg_on_inverse_and_dims_alpha() {
    let t = term(TerminalScreen::new(80, 24));
    let mut cell = Cell {
        fg: TermColor::Default,
        bg: TermColor::Default,
        ..Cell::default()
    };
    let (fg, bg) = t.cell_colors(&cell);
    assert_eq!(fg, t.style.fg);
    assert_eq!(bg, None);

    cell.inverse = true;
    let (fg, bg) = t.cell_colors(&cell);
    assert_eq!(fg, t.style.bg, "inverse swaps fg/bg");
    assert_eq!(bg, Some(t.style.fg));

    cell.inverse = false;
    cell.dim = true;
    let (fg, _) = t.cell_colors(&cell);
    assert!(fg.a < t.style.fg.a, "dim reduces alpha");
}

#[test]
fn dims_reports_total_lines_visible_rows_and_cols_for_the_bounds() {
    let t = term(screen(10, 5, b""));
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(100.0, 65.0));
    let (total, rows, cols) = t.dims(10.0, 13.0, bounds);
    assert_eq!(total, 5, "no scrollback yet, just the 5 live rows");
    assert_eq!(rows, 5, "65 / 13");
    assert_eq!(cols, 10, "100 / 10");
}

#[test]
fn selection_text_is_empty_with_no_selection() {
    let t = term(screen(10, 2, b"hello"));
    let state = State::default();
    assert_eq!(t.selection_text(&state), "");
}

#[test]
fn selection_text_reads_the_ordered_selection() {
    let t = term(screen(20, 1, b"hello world"));
    let state = State {
        selection: Some(((0, 6), (0, 10))),
        ..State::default()
    };
    assert_eq!(t.selection_text(&state), "world");
}

#[test]
fn resolve_path_absolute_relative_and_home() {
    // Absolute paths pass through untouched.
    assert_eq!(resolve_path("/etc/hosts", Some("/tmp")), "/etc/hosts");
    // Relative paths join onto the cwd (trailing slash normalized).
    assert_eq!(
        resolve_path("src/main.rs", Some("/work/tty/")),
        "/work/tty/src/main.rs"
    );
    // No cwd known → left relative for the host to resolve however it opens files.
    assert_eq!(resolve_path("src/main.rs", None), "src/main.rs");
}
