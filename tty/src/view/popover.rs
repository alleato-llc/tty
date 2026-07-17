//! The drill-in **chrome**: the floating popover card (its frame, the
//! +/−/⊞/× control cluster, on-screen placement, and the drag-resize edges) and
//! the graduated metric pane's header. Wraps the body that `super::metrics`
//! renders (`metric_body`); the two are split so the chrome and the content
//! evolve independently.

use iced::widget::{column, container, mouse_area, pane_grid, row, text};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::button;

use crate::message::Message;
use crate::state::Tty;

use super::metrics::metric_body;

/// One graduated metric **pane** (a metric drill-in promoted into the pane grid):
/// a compact header (the metric's name + maximize/restore + close) over its
/// [`metric_body`]. Focus/right-click-to-split come from the surrounding
/// `pane_grid` like any pane.
#[allow(clippy::too_many_arguments)]
pub(super) fn metric_pane_content<'a>(
    state: &'a Tty,
    kind: crate::settings::MetricKind,
    win: iced::window::Id,
    pane: pane_grid::Pane,
    is_focused: bool,
    maximized: bool,
    multi: bool,
    highlight: bool,
    accent: iced::Color,
    hairline: iced::Color,
    bg: iced::Color,
) -> pane_grid::Content<'a, Message> {
    let t = theme::tokens();
    // Size the body to the pane's rough share of the window (the closure isn't
    // handed pixel bounds). Bias width toward a split's half so long names in a
    // table truncate (an ellipsis) rather than wrap; the chart fills the height.
    let card_w = (state.window_width * 0.45).max(240.0);
    let chart_h = (state.window_height * 0.30).clamp(120.0, 360.0);

    let header = row![
        container(
            text(kind.to_string())
                .size(13)
                .color(if is_focused { accent } else { t.ink }),
        )
        .width(Length::Fill),
        button::ghost_compact(
            if maximized { "▪" } else { "▫" },
            Message::ToggleMaximizePane(win),
        ),
        button::ghost_compact("×", Message::CloseMetricPane(win, pane)),
    ]
    .align_y(iced::Alignment::Center)
    .spacing(0);

    let inner = column![header, metric_body(state, kind, false, card_w, chart_h)]
        .spacing(6)
        .padding(6);
    let border_color = if is_focused && highlight {
        accent
    } else {
        hairline
    };
    let bordered = container(inner).style(move |_| {
        let border = if multi {
            Border {
                color: border_color,
                width: 1.0,
                radius: 0.0.into(),
            }
        } else {
            Border::default()
        };
        container::Style {
            border,
            ..container::background(bg)
        }
    });
    pane_grid::Content::new(mouse_area(bordered).on_right_press(Message::PaneRightClick(pane)))
}

pub(super) fn metric_popover_card<'a>(
    state: &'a Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
) -> Element<'a, Message> {
    let kind = pop.kind;
    let expanded = pop.expanded;
    let pinned = state.settings.status_bar_metrics_pinned();
    let t = theme::tokens();

    // The current size: the user's dragged override, else the compact or
    // expanded default (see `MetricPopover::effective_size`).
    let (card_w, chart_h) = pop.effective_size(state.window_width, state.window_height);
    let body = metric_body(state, kind, expanded, card_w, chart_h);

    let card = container(body)
        .width(Length::Fixed(card_w))
        .padding(14)
        .style(move |_| container::Style {
            border: Border {
                color: t.hairline,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::background(t.surface)
        });
    // Border-resize strips and the top-right controls (expand, plus a close "×"
    // when popovers are pinned) overlay the card.
    iced::widget::stack![
        with_resize_edges(index, card.into()),
        popover_controls(
            index,
            kind,
            expanded,
            pinned,
            state.settings.graduate_metrics(),
        ),
    ]
    .into()
}

/// The top-right control cluster overlaid on a popover card: a "move to pane" ⊞
/// (graduate the drill-in into a real split pane, when enabled), the expand "+" /
/// collapse "−" toggle, plus a close "×" when popovers are pinned (in the
/// one-at-a-time mode a click away closes it, so no per-card button is needed).
fn popover_controls<'a>(
    index: usize,
    kind: crate::settings::MetricKind,
    expanded: bool,
    pinned: bool,
    can_graduate: bool,
) -> Element<'a, Message> {
    let mut controls = row![].spacing(0).align_y(iced::Alignment::Center);
    if can_graduate {
        controls = controls.push(button::ghost_compact(
            "⊞",
            Message::PromotePopoverMenu(kind),
        ));
    }
    controls = controls.push(button::ghost_compact(
        if expanded { "−" } else { "+" },
        Message::ToggleMetricDetailExpanded(index),
    ));
    if pinned {
        controls = controls.push(button::ghost_compact(
            "×",
            Message::CloseMetricPopover(index),
        ));
    }
    container(controls)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Top)
        .padding([4, 6])
        .into()
}

/// Place a popover `card` in the window: compact cards anchor just above the
/// status bar (centered), expanded ones center; a user drag offsets from there,
/// laid out absolutely via top-left padding (horizontal exact, vertical measured
/// up from the bottom). Pinned popovers past the first cascade up-and-right so a
/// stack stays legible. All clamped to stay on-screen; the headless path (window
/// size 0) falls back to the alignment anchor.
pub(super) fn place_metric_popover<'a>(
    state: &Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
    card: Element<'a, Message>,
) -> Element<'a, Message> {
    use iced::alignment::{Horizontal, Vertical};

    let known = state.window_width > 1.0 && state.window_height > 1.0;
    let cascade = 28.0 * index as f32;
    let dx = pop.move_offset.0 + cascade;
    let dy = pop.move_offset.1 - cascade;
    let (card_w, _) = pop.effective_size(state.window_width, state.window_height);

    if (dx != 0.0 || dy != 0.0) && known {
        let base_gap = if pop.expanded { 60.0 } else { 44.0 };
        let left = ((state.window_width - card_w) / 2.0 + dx)
            .clamp(8.0, (state.window_width - card_w - 8.0).max(8.0));
        let bottom = (base_gap - dy).clamp(8.0, (state.window_height - 120.0).max(8.0));
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Left)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding::ZERO.left(left).bottom(bottom))
            .into()
    } else {
        let a = container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center);
        if pop.expanded {
            a.align_y(Vertical::Center).into()
        } else {
            a.align_y(Vertical::Bottom)
                .padding(iced::Padding::ZERO.bottom(44.0))
                .into()
        }
    }
}

/// One invisible resize strip for [`with_resize_edges`]: a transparent hit area
/// of the given size whose press starts a drag-resize from `edge` (tracked
/// through `PointerMoved`, ended by `PointerReleased`), showing the matching
/// resize cursor on hover. It carries no paint of its own; the card's own border
/// is the visible edge the user grabs.
fn resize_strip<'a>(
    index: usize,
    edge: crate::state::ResizeEdge,
    width: Length,
    height: Length,
    cursor: iced::mouse::Interaction,
) -> Element<'a, Message> {
    mouse_area(iced::widget::Space::new().width(width).height(height))
        .on_press(Message::MetricDetailResizeStart(index, edge))
        .interaction(cursor)
        .into()
}

/// Overlay thin invisible resize strips along the card's right and bottom edges
/// and its bottom-right corner, so the popover resizes by dragging its borders
/// (each with the matching resize cursor) rather than a separate grip. The strips
/// stack over the card; `iced`'s `stack` sizes to its first child, so they span
/// exactly the card, not the window. `index` is the popover the drag addresses.
fn with_resize_edges(index: usize, card: Element<'_, Message>) -> Element<'_, Message> {
    use crate::state::ResizeEdge;
    use iced::alignment::{Horizontal::Right, Vertical::Bottom};
    use iced::mouse::Interaction;

    const EDGE: f32 = 8.0;
    const CORNER: f32 = 16.0;

    let right = container(resize_strip(
        index,
        ResizeEdge::Right,
        Length::Fixed(EDGE),
        Length::Fill,
        Interaction::ResizingHorizontally,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right);
    let bottom = container(resize_strip(
        index,
        ResizeEdge::Bottom,
        Length::Fill,
        Length::Fixed(EDGE),
        Interaction::ResizingVertically,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Bottom);
    let corner = container(resize_strip(
        index,
        ResizeEdge::Corner,
        Length::Fixed(CORNER),
        Length::Fixed(CORNER),
        Interaction::ResizingDiagonallyDown,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right)
    .align_y(Bottom);

    iced::widget::stack![card, right, bottom, corner].into()
}
