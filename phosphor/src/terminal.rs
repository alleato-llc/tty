//! A terminal-grid widget: renders a `cathode::TerminalScreen`'s cells with color and
//! text attributes, paints a block cursor, scrolls back through history, supports
//! mouse selection (reported as text for the host to copy), and reports how many
//! columns/rows fit at the current font so the host can resize the PTY. The screen
//! lives in the host (shared with the PTY read loop); the widget is a view over it.
//!
//! Shared by the IDE's terminal panel and the standalone `tty` app, the same way the
//! `editor` widget is shared — refinements land in both at once.

use std::sync::Arc;

use parking_lot::Mutex;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::Quad;
use iced::advanced::text::{self, Paragraph, Text};
use iced::advanced::widget::{tree, Tree};
use iced::advanced::{mouse, renderer, Clipboard, Shell, Widget};
use iced::{
    alignment, Border, Color, Element, Event, Font, Length, Point, Rectangle, Shadow, Size,
};

use iced::keyboard::Modifiers;

use cathode::screen::{Cell, CursorShape, TermColor, TerminalScreen};

use crate::input::{self, MouseButton, MouseEvent};

/// The 16 ANSI colors (0–7 normal, 8–15 bright) as RGB — a conventional dark palette.
/// A host with its own theme can override every field of [`TerminalStyle`]; this is the
/// out-of-the-box default (used by [`TerminalStyle::default_dark`]).
pub const ANSI_DEFAULT: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // black
    (0xcc, 0x24, 0x1d), // red
    (0x98, 0x97, 0x1a), // green
    (0xd7, 0x99, 0x21), // yellow
    (0x45, 0x85, 0x88), // blue
    (0xb1, 0x62, 0x86), // magenta
    (0x68, 0x9d, 0x6a), // cyan
    (0xa8, 0x99, 0x84), // white
    (0x92, 0x83, 0x74), // bright black
    (0xfb, 0x49, 0x34), // bright red
    (0xb8, 0xbb, 0x26), // bright green
    (0xfa, 0xbd, 0x2f), // bright yellow
    (0x83, 0xa5, 0x98), // bright blue
    (0xd3, 0x86, 0x9b), // bright magenta
    (0x8e, 0xc0, 0x7c), // bright cyan
    (0xeb, 0xdb, 0xb2), // bright white
];

/// Terminal-surface colors (the host maps its theme into this — keeps `phosphor` free
/// of a theme dependency, like `EditorStyle`).
#[derive(Debug, Clone, Copy)]
pub struct TerminalStyle {
    /// The 16 ANSI colors: 0–7 normal, 8–15 bright.
    pub ansi: [Color; 16],
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    /// Background tint over selected cells.
    pub selection: Color,
}

impl TerminalStyle {
    /// A conventional dark terminal palette ([`ANSI_DEFAULT`]) — a sensible default for
    /// hosts without their own theme.
    pub fn default_dark() -> Self {
        let ansi = ANSI_DEFAULT.map(|(r, g, b)| Color::from_rgb8(r, g, b));
        Self {
            ansi,
            fg: Color::from_rgb8(0xeb, 0xdb, 0xb2),
            bg: Color::from_rgb8(0x1d, 0x20, 0x21),
            cursor: Color::from_rgb8(0xeb, 0xdb, 0xb2),
            selection: Color::from_rgba8(0x45, 0x85, 0x88, 0.4),
        }
    }
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self::default_dark()
    }
}

/// Row height as a multiple of the font size — terminals sit a touch tighter than the
/// editor. The grid's column width is measured from the monospace font at draw time.
const LINE_HEIGHT_RATIO: f32 = 1.3;

/// An absolute grid position: `(line, col)`, where `line` counts from the top of
/// scrollback (so it's stable as the live screen scrolls).
type Pos = (usize, usize);

/// Per-widget state: how far we've scrolled back, any in-progress selection, and the
/// live modifier state (so ⌥ can override an app's mouse grab to force selection).
#[derive(Default)]
struct State {
    /// Lines scrolled up into history (0 = the live bottom).
    scroll: usize,
    /// `(anchor, head)` of a selection in absolute grid coords.
    selection: Option<(Pos, Pos)>,
    dragging: bool,
    modifiers: Modifiers,
    /// The cell the last drag report was sent for (dedupe motion within a cell).
    last_report: Option<(usize, usize)>,
}

pub struct Terminal<Message> {
    screen: Arc<Mutex<TerminalScreen>>,
    style: TerminalStyle,
    font: Font,
    font_size: f32,
    focused: bool,
    on_resize: Box<dyn Fn(usize, usize) -> Message>,
    /// Fires as a selection is dragged (`Some(text)`) or cleared (`None`); the host
    /// caches it so a copy shortcut can write it to the clipboard.
    on_select: Box<dyn Fn(Option<String>) -> Message>,
    /// Fires with the bytes to send to the PTY when a running app has enabled mouse
    /// reporting (the host writes them to the shell).
    on_mouse: Box<dyn Fn(Vec<u8>) -> Message>,
    /// Case-insensitive scrollback search: matching runs are highlighted.
    find: Option<String>,
}

impl<Message> Terminal<Message> {
    /// Highlight every case-insensitive occurrence of `query` (the `⌘F` search).
    pub fn find(mut self, query: Option<String>) -> Self {
        self.find = query.filter(|q| !q.is_empty());
        self
    }
}

/// Build the terminal widget over `screen`. `on_resize(cols, rows)` fires when the
/// grid that fits changes; `on_select(text)` as a selection is dragged; `on_mouse`
/// carries PTY bytes when the app grabbed the mouse (DEC modes 1000/1002/1003/1006).
#[allow(clippy::too_many_arguments)]
pub fn terminal<Message>(
    screen: Arc<Mutex<TerminalScreen>>,
    style: TerminalStyle,
    font: Font,
    font_size: f32,
    focused: bool,
    on_resize: impl Fn(usize, usize) -> Message + 'static,
    on_select: impl Fn(Option<String>) -> Message + 'static,
    on_mouse: impl Fn(Vec<u8>) -> Message + 'static,
) -> Terminal<Message> {
    Terminal {
        screen,
        style,
        font,
        font_size,
        focused,
        on_resize: Box::new(on_resize),
        on_select: Box::new(on_select),
        on_mouse: Box::new(on_mouse),
        find: None,
    }
}

impl<Message> Terminal<Message> {
    fn line_height(&self) -> f32 {
        (self.font_size * LINE_HEIGHT_RATIO).round().max(1.0)
    }

    /// A bold/italic variant of the base font for an attributed run.
    fn variant(&self, bold: bool, italic: bool) -> Font {
        Font {
            weight: if bold {
                iced::font::Weight::Bold
            } else {
                self.font.weight
            },
            style: if italic {
                iced::font::Style::Italic
            } else {
                self.font.style
            },
            ..self.font
        }
    }

    /// Resolved `(foreground, optional background)` for a cell, applying inverse + dim.
    fn cell_colors(&self, cell: &Cell) -> (Color, Option<Color>) {
        let mut fg = resolve(cell.fg, &self.style, self.style.fg, cell.bold);
        let mut bg = match cell.bg {
            TermColor::Default => None,
            other => Some(resolve(other, &self.style, self.style.bg, false)),
        };
        if cell.inverse {
            let prev_bg = bg.unwrap_or(self.style.bg);
            bg = Some(fg);
            fg = prev_bg;
        }
        if cell.dim {
            fg = Color {
                a: fg.a * 0.6,
                ..fg
            };
        }
        (fg, bg)
    }

    /// `(total_lines, visible_rows, cols)` for the current bounds + font.
    fn dims(&self, cell_w: f32, line_h: f32, bounds: Rectangle) -> (usize, usize, usize) {
        let screen = self.screen.lock();
        let total = screen.scrollback.len() + screen.rows;
        let rows = (bounds.height / line_h).floor().max(1.0) as usize;
        let cols = (bounds.width / cell_w).floor().max(1.0) as usize;
        (total, rows, cols)
    }

    /// The selected text for `state`'s selection, or empty.
    fn selection_text(&self, state: &State) -> String {
        let Some(sel) = state.selection else {
            return String::new();
        };
        let screen = self.screen.lock();
        let history = screen.scrollback.len();
        selected_text(&screen, history, screen.cols, order(sel))
    }
}

/// The viewport's top absolute line: the live bottom (`scroll = 0`) shows the last
/// `rows` lines; scrolling up moves the window earlier.
fn view_top(total: usize, rows: usize, scroll: usize) -> usize {
    total.saturating_sub(rows).saturating_sub(scroll)
}

/// The cell at absolute `(line, col)` — from scrollback if `line` is in history, else
/// the live screen (default-filled past the end).
fn cell_at(screen: &TerminalScreen, history: usize, line: usize, col: usize) -> Cell {
    if line < history {
        screen
            .scrollback
            .get(line)
            .and_then(|row| row.get(col))
            .cloned()
            .unwrap_or_default()
    } else {
        let r = line - history;
        if r < screen.rows {
            screen.cell(r, col).clone()
        } else {
            Cell::default()
        }
    }
}

/// Order a selection so anchor ≤ head lexicographically by `(line, col)`.
fn order((a, b): (Pos, Pos)) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Gather the text of an ordered selection: trailing spaces trimmed per line, lines
/// joined by `\n`.
fn selected_text(
    screen: &TerminalScreen,
    history: usize,
    cols: usize,
    (a, b): (Pos, Pos),
) -> String {
    let mut out = String::new();
    for line in a.0..=b.0 {
        let start = if line == a.0 { a.1 } else { 0 };
        let end = if line == b.0 {
            b.1
        } else {
            cols.saturating_sub(1)
        };
        let mut row = String::new();
        for c in start..=end.min(cols.saturating_sub(1)) {
            row.push(cell_at(screen, history, line, c).ch);
        }
        out.push_str(row.trim_end());
        if line != b.0 {
            out.push('\n');
        }
    }
    out
}

/// Pixel position → absolute `(line, col)`, clamped to the grid.
#[allow(clippy::too_many_arguments)]
fn hit(
    bounds: Rectangle,
    cell_w: f32,
    line_h: f32,
    pos: Point,
    total: usize,
    rows: usize,
    cols: usize,
    scroll: usize,
) -> Pos {
    let vr = (((pos.y - bounds.y) / line_h).floor().max(0.0) as usize).min(rows.saturating_sub(1));
    let col = (((pos.x - bounds.x) / cell_w).floor().max(0.0) as usize).min(cols.saturating_sub(1));
    (view_top(total, rows, scroll) + vr, col)
}

/// Pixel position → 0-based visible `(col, row)` for mouse reporting (clamped).
fn cell_pos(bounds: Rectangle, cell_w: f32, line_h: f32, pos: Point) -> (usize, usize) {
    let col = ((pos.x - bounds.x) / cell_w).floor().max(0.0) as usize;
    let row = ((pos.y - bounds.y) / line_h).floor().max(0.0) as usize;
    (col, row)
}

/// Map an iced mouse button to the reportable set (extra buttons are ignored).
fn mouse_button(b: mouse::Button) -> Option<MouseButton> {
    match b {
        mouse::Button::Left => Some(MouseButton::Left),
        mouse::Button::Middle => Some(MouseButton::Middle),
        mouse::Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

/// Map a `TermColor` to an iced color against the style's palette.
fn resolve(c: TermColor, style: &TerminalStyle, default: Color, bold: bool) -> Color {
    match c {
        TermColor::Default => default,
        TermColor::Named(n) => {
            let n = n as usize;
            // Bold brightens the 8 base colors to their 8–15 bright variants.
            let idx = if n < 8 && bold { n + 8 } else { n.min(15) };
            style.ansi[idx]
        }
        TermColor::Indexed(i) => indexed(i, style),
        TermColor::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
    }
}

/// The xterm 256-color cube: 0–15 → ANSI, 16–231 → 6×6×6 cube, 232–255 → grayscale.
fn indexed(i: u8, style: &TerminalStyle) -> Color {
    let i = i as usize;
    if i < 16 {
        return style.ansi[i];
    }
    if i < 232 {
        let i = i - 16;
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        return Color::from_rgb8(LEVELS[(i / 36) % 6], LEVELS[(i / 6) % 6], LEVELS[i % 6]);
    }
    let v = (8 + (i - 232) * 10) as u8;
    Color::from_rgb8(v, v, v)
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Terminal<Message>
where
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let cell_w = cell_width(renderer, self.font, self.font_size);
        let line_h = self.line_height();
        let state = tree.state.downcast_mut::<State>();

        // Resize the PTY whenever the grid that fits changes (window resize, zoom).
        {
            let screen = self.screen.lock();
            let cols = (bounds.width / cell_w).floor().max(1.0) as usize;
            let rows = (bounds.height / line_h).floor().max(1.0) as usize;
            if (cols, rows) != (screen.cols, screen.rows) {
                shell.publish((self.on_resize)(cols, rows));
            }
        }

        // When a running app has grabbed the mouse (DEC 1000/1002/1003), pointer events
        // go to the PTY — unless ⌥ is held, which forces local selection instead.
        let mouse_state = self.screen.lock().mouse;
        let to_app = mouse_state.reports() && !state.modifiers.alt();

        match event {
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(pos) = cursor.position_over(bounds) else {
                    return;
                };
                let notches = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / line_h,
                };
                if to_app {
                    // Report wheel as button 64/65 at the pointer cell.
                    let (col, row) = cell_pos(bounds, cell_w, line_h, pos);
                    let ev = if notches > 0.0 {
                        MouseEvent::WheelUp
                    } else {
                        MouseEvent::WheelDown
                    };
                    for _ in 0..notches.abs().ceil() as usize {
                        if let Some(b) =
                            input::mouse_report(mouse_state, ev, col, row, state.modifiers)
                        {
                            shell.publish((self.on_mouse)(b));
                        }
                    }
                    shell.capture_event();
                } else {
                    // Wheel up shows earlier history (3 lines per notch, like the editor).
                    let history = self.screen.lock().scrollback.len();
                    let next =
                        (state.scroll as f32 + notches * 3.0).clamp(0.0, history as f32) as usize;
                    if next != state.scroll {
                        state.scroll = next;
                        shell.request_redraw();
                    }
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(pos) = cursor.position_over(bounds) else {
                    return;
                };
                if to_app {
                    if let Some(b) = mouse_button(*button) {
                        let (col, row) = cell_pos(bounds, cell_w, line_h, pos);
                        if let Some(bytes) = input::mouse_report(
                            mouse_state,
                            MouseEvent::Press(b),
                            col,
                            row,
                            state.modifiers,
                        ) {
                            shell.publish((self.on_mouse)(bytes));
                        }
                        state.dragging = matches!(b, MouseButton::Left);
                        state.last_report = Some((col, row));
                        shell.capture_event();
                    }
                } else if *button == mouse::Button::Left {
                    let (total, rows, cols) = self.dims(cell_w, line_h, bounds);
                    let p = hit(bounds, cell_w, line_h, pos, total, rows, cols, state.scroll);
                    state.selection = Some((p, p));
                    state.dragging = true;
                    shell.publish((self.on_select)(None));
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(pos) = cursor.position() else { return };
                if to_app {
                    let clamped = Point::new(
                        pos.x.clamp(bounds.x, bounds.x + bounds.width - 1.0),
                        pos.y.clamp(bounds.y, bounds.y + bounds.height - 1.0),
                    );
                    let (col, row) = cell_pos(bounds, cell_w, line_h, clamped);
                    // Dedupe motion within the same cell to avoid flooding the PTY.
                    if state.last_report != Some((col, row)) {
                        let ev = if state.dragging {
                            MouseEvent::Drag(MouseButton::Left)
                        } else {
                            MouseEvent::Move
                        };
                        if let Some(bytes) =
                            input::mouse_report(mouse_state, ev, col, row, state.modifiers)
                        {
                            shell.publish((self.on_mouse)(bytes));
                            state.last_report = Some((col, row));
                        }
                    }
                } else if state.dragging {
                    let (total, rows, cols) = self.dims(cell_w, line_h, bounds);
                    // Clamp into bounds so a drag past the edge still selects the edge.
                    let clamped = Point::new(
                        pos.x.clamp(bounds.x, bounds.x + bounds.width - 1.0),
                        pos.y.clamp(bounds.y, bounds.y + bounds.height - 1.0),
                    );
                    let p = hit(
                        bounds,
                        cell_w,
                        line_h,
                        clamped,
                        total,
                        rows,
                        cols,
                        state.scroll,
                    );
                    if let Some((a, _)) = state.selection {
                        state.selection = Some((a, p));
                    }
                    let text = self.selection_text(state);
                    shell.publish((self.on_select)((!text.is_empty()).then_some(text)));
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if to_app {
                    if let (Some(b), Some(pos)) = (mouse_button(*button), cursor.position()) {
                        let clamped = Point::new(
                            pos.x.clamp(bounds.x, bounds.x + bounds.width - 1.0),
                            pos.y.clamp(bounds.y, bounds.y + bounds.height - 1.0),
                        );
                        let (col, row) = cell_pos(bounds, cell_w, line_h, clamped);
                        if let Some(bytes) = input::mouse_report(
                            mouse_state,
                            MouseEvent::Release(b),
                            col,
                            row,
                            state.modifiers,
                        ) {
                            shell.publish((self.on_mouse)(bytes));
                        }
                    }
                    state.dragging = false;
                    state.last_report = None;
                    shell.capture_event();
                } else if *button == mouse::Button::Left && state.dragging {
                    state.dragging = false;
                    if let Some((a, b)) = state.selection {
                        if a == b {
                            state.selection = None;
                            shell.publish((self.on_select)(None));
                        }
                    }
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let cell_w = cell_width(renderer, self.font, self.font_size);
        let line_h = self.line_height();
        let state = tree.state.downcast_ref::<State>();
        let screen = self.screen.lock();
        let history = screen.scrollback.len();
        let total = history + screen.rows;
        let rows = (bounds.height / line_h).floor().max(1.0) as usize;
        let cols = screen.cols;
        let top = view_top(total, rows, state.scroll);
        let sel = state.selection.map(order);

        renderer.with_layer(bounds, |renderer| {
            for vr in 0..rows {
                let line = top + vr;
                if line >= total {
                    break;
                }
                let y = bounds.y + vr as f32 * line_h;

                // Background runs.
                let mut c = 0;
                while c < cols {
                    let bg = self.cell_colors(&cell_at(&screen, history, line, c)).1;
                    let Some(bg) = bg else {
                        c += 1;
                        continue;
                    };
                    let mut end = c + 1;
                    while end < cols
                        && self.cell_colors(&cell_at(&screen, history, line, end)).1 == Some(bg)
                    {
                        end += 1;
                    }
                    fill_rect(
                        renderer,
                        bounds.x + c as f32 * cell_w,
                        y,
                        (end - c) as f32 * cell_w,
                        line_h,
                        bg,
                    );
                    c = end;
                }

                // Selection tint.
                if let Some((a, b)) = sel {
                    if line >= a.0 && line <= b.0 {
                        let s = if line == a.0 { a.1 } else { 0 };
                        let e = if line == b.0 {
                            b.1
                        } else {
                            cols.saturating_sub(1)
                        };
                        let e = e.min(cols.saturating_sub(1));
                        fill_rect(
                            renderer,
                            bounds.x + s as f32 * cell_w,
                            y,
                            (e + 1 - s) as f32 * cell_w,
                            line_h,
                            self.style.selection,
                        );
                    }
                }

                // Search highlight — tint every case-insensitive match on this line.
                // Column-based (one char per cell) so it's panic-safe with wide glyphs.
                if let Some(q) = self.find.as_deref() {
                    let cells: Vec<char> = (0..cols)
                        .map(|c| cell_at(&screen, history, line, c).ch)
                        .collect();
                    let needle: Vec<char> = q.chars().collect();
                    let hl = Color {
                        a: 0.45,
                        ..self.style.ansi[11]
                    };
                    let mut s = 0;
                    while s + needle.len() <= cells.len() && !needle.is_empty() {
                        let hit = (0..needle.len())
                            .all(|k| cells[s + k].eq_ignore_ascii_case(&needle[k]));
                        if hit {
                            fill_rect(
                                renderer,
                                bounds.x + s as f32 * cell_w,
                                y,
                                needle.len() as f32 * cell_w,
                                line_h,
                                hl,
                            );
                            s += needle.len();
                        } else {
                            s += 1;
                        }
                    }
                }

                // Foreground runs (one fill_text per color+attribute run; underline as
                // a thin quad since `Text` has no underline field).
                let mut c = 0;
                while c < cols {
                    let cell = cell_at(&screen, history, line, c);
                    // A width-0 spacer is the right half of the wide glyph to its left;
                    // its glyph was drawn with that base cell, so skip it here.
                    if cell.width == 0 {
                        c += 1;
                        continue;
                    }
                    let (fg, _) = self.cell_colors(&cell);
                    let key = (fg, cell.bold, cell.italic, cell.underline);
                    let mut run = String::new();
                    run.push(cell.ch);
                    let mut end = c + 1;
                    while end < cols {
                        let nc = cell_at(&screen, history, line, end);
                        // Absorb wide-glyph spacers into the current run without adding a
                        // char — the wide glyph's own advance covers that column.
                        if nc.width == 0 {
                            end += 1;
                            continue;
                        }
                        let (nfg, _) = self.cell_colors(&nc);
                        if (nfg, nc.bold, nc.italic, nc.underline) == key {
                            run.push(nc.ch);
                            end += 1;
                        } else {
                            break;
                        }
                    }
                    let x = bounds.x + c as f32 * cell_w;
                    if !run.trim_end().is_empty() {
                        renderer.fill_text(
                            text::Text {
                                content: run,
                                bounds: Size::new((bounds.x + bounds.width - x).max(1.0), line_h),
                                size: self.font_size.into(),
                                line_height: text::LineHeight::Absolute(line_h.into()),
                                font: self.variant(key.1, key.2),
                                align_x: text::Alignment::Left,
                                align_y: alignment::Vertical::Top,
                                shaping: text::Shaping::Advanced,
                                wrapping: text::Wrapping::None,
                            },
                            Point::new(x, y),
                            fg,
                            bounds,
                        );
                    }
                    if key.3 {
                        fill_rect(
                            renderer,
                            x,
                            y + line_h - 2.0,
                            (end - c) as f32 * cell_w,
                            1.0,
                            fg,
                        );
                    }
                    c = end;
                }
            }

            // Cursor — only at the live bottom (not while scrolled into history), and
            // only when the app hasn't hidden it (DEC 25). Shape per DECSCUSR.
            if self.focused
                && state.scroll == 0
                && screen.cursor_visible
                && screen.cursor_col < cols
            {
                let cvr = (history + screen.cursor_row).saturating_sub(top);
                if cvr < rows {
                    let under = screen.cell(screen.cursor_row, screen.cursor_col);
                    let span = (under.width.max(1) as f32) * cell_w;
                    let cx = bounds.x + screen.cursor_col as f32 * cell_w;
                    let cy = bounds.y + cvr as f32 * line_h;
                    let bar = (self.font_size * 0.12).clamp(1.5, 3.0);
                    match screen.cursor_shape {
                        CursorShape::Block => {
                            fill_rect(renderer, cx, cy, span, line_h, self.style.cursor);
                            // Re-draw the covered glyph in the background color (inverse).
                            let ch = under.ch;
                            if ch != ' ' {
                                renderer.fill_text(
                                    text::Text {
                                        content: ch.to_string(),
                                        bounds: Size::new(span.max(1.0), line_h),
                                        size: self.font_size.into(),
                                        line_height: text::LineHeight::Absolute(line_h.into()),
                                        font: self.font,
                                        align_x: text::Alignment::Left,
                                        align_y: alignment::Vertical::Top,
                                        shaping: text::Shaping::Advanced,
                                        wrapping: text::Wrapping::None,
                                    },
                                    Point::new(cx, cy),
                                    self.style.bg,
                                    bounds,
                                );
                            }
                        }
                        CursorShape::Underline => {
                            fill_rect(
                                renderer,
                                cx,
                                cy + line_h - bar,
                                span,
                                bar,
                                self.style.cursor,
                            );
                        }
                        CursorShape::Bar => {
                            fill_rect(renderer, cx, cy, bar, line_h, self.style.cursor);
                        }
                    }
                }
            }
        });
    }
}

/// Measure the monospace cell width by shaping one glyph.
fn cell_width<Renderer: text::Renderer<Font = Font>>(
    _renderer: &Renderer,
    font: Font,
    size: f32,
) -> f32 {
    let para = <Renderer::Paragraph as Paragraph>::with_text(Text {
        content: "M",
        bounds: Size::new(f32::INFINITY, f32::INFINITY),
        size: size.into(),
        line_height: text::LineHeight::Absolute((size * LINE_HEIGHT_RATIO).into()),
        font,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::None,
    });
    para.min_bounds().width.max(1.0)
}

fn fill_rect<Renderer: renderer::Renderer>(
    renderer: &mut Renderer,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: Color,
) {
    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x,
                y,
                width,
                height,
            },
            border: Border::default(),
            shadow: Shadow::default(),
            snap: true,
        },
        color,
    );
}

impl<'a, Message, Theme, Renderer> From<Terminal<Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Renderer: text::Renderer<Font = Font>,
{
    fn from(t: Terminal<Message>) -> Self {
        Element::new(t)
    }
}
