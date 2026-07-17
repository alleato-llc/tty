//! The bottom status bar: the shell name, the ordered machine-stat cells (each a
//! sparkline or number), width-shedding when the bar overflows, and the
//! scroll-paging window through the shed cells. Also the status-bar metrics
//! editor (in the Metrics settings section) and the corner readout text. Reads
//! the resolved per-metric render data from `super::metrics`.

use iced::widget::{column, container, mouse_area, row, text};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, caption, select, sparkline, status_bar_content, SparkSeries, Sparkline,
};

use crate::message::Message;
use crate::state::Tty;

use super::metrics::{metric_render, MetricRender};
use super::util::*;

/// The status-bar footer text size — matches rime's own `status_bar` TEXT_SIZE
/// so the typography is uniform whether the bar is text-only or hosts stats.
const STATUS_BAR_TEXT_SIZE: f32 = 13.0;

/// The bottom status bar as an Element: shell name on the left, then (when
/// configured) the machine-stat cells in display order, then the grid/tab/font
/// cluster. Built on rime's `status_bar_content` (the styled strip) rather than
/// the plain-text `status_bar`, so the canvas sparklines can sit beside the
/// text. When the window is too narrow to hold every cell, metrics are shed
/// from the right (see [`visible_metric_count`]) before anything wraps.
pub(super) fn status_bar_view(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let (left, right) = status_text(state);
    let right_text = || {
        text(right.clone())
            .size(STATUS_BAR_TEXT_SIZE)
            .color(t.muted)
    };

    // Three zones: shell name (left), the machine-stat cells (center), the grid
    // geometry (right). The center cluster sits between two flex spaces so it
    // centers in the gap; with no visible metrics it collapses to nothing and
    // the two spaces merge, leaving the no-stats bar pixel-identical to the old
    // text-only footer (left name, right geometry).
    let editing = state.status_bar_edit;
    // Each cell carries its raw-config index (drag-reorder mutates the stored list
    // by index; the resolved list may drop unknown entries, so we can't assume the
    // display position is the config index).
    let cells: Vec<(usize, crate::settings::ResolvedMetric, MetricRender)> = state
        .settings
        .status_bar_metrics_indexed()
        .into_iter()
        .filter_map(|(i, cfg)| metric_render(cfg, state).map(|r| (i, cfg, r)))
        .collect();
    let total = cells.len();
    let visible = visible_metric_count(&cells, &left, &right, state.window_width);
    // When the bar can't hold every cell, a scroll over it slides this window
    // through the full list; clamp the stored offset to the valid range.
    let max_start = total.saturating_sub(visible);
    let start = state.status_bar_scroll.min(max_start);

    let flex = || iced::widget::Space::new().width(Length::Fill);
    let mut content = row![text(left).size(STATUS_BAR_TEXT_SIZE).color(t.muted), flex()]
        .align_y(iced::Alignment::Center);

    if visible > 0 {
        // A muted chevron on each side when there are more cells off that edge, so
        // the user knows to scroll.
        let chevron = |show: bool, glyph: &str| -> Element<'_, Message> {
            if show {
                text(glyph.to_string())
                    .size(STATUS_BAR_TEXT_SIZE)
                    .color(t.muted)
                    .into()
            } else {
                iced::widget::Space::new().width(Length::Fixed(0.0)).into()
            }
        };
        // What's being dragged, and where a drop would land — for the outline /
        // lift / insertion-bar affordances. A cell mid press-hold shows *no*
        // border: the outline appears only once the hold engages (edit mode), so a
        // quick tap never flashes one.
        let dragging = state.status_metric_drag;
        let drop = state.status_metric_drop;
        // The window shows `visible` cells from `start`. A cell arms a press on
        // press-down (a quick release opens its drill-in; a hold enters edit mode
        // and starts dragging it); in edit mode it also reports drag-overs. An
        // accent insertion bar shows where a dragged cell would drop.
        let mut cluster: Vec<Element<'_, Message>> = Vec::new();
        for (raw_i, cfg, r) in cells.into_iter().skip(start).take(visible) {
            // The drop insertion bar goes just before the target cell (only while
            // actually dragging to a *different* slot).
            if editing && dragging.is_some() && dragging != Some(raw_i) && drop == Some(raw_i) {
                cluster.push(
                    container(
                        iced::widget::Space::new()
                            .width(Length::Fixed(2.0))
                            .height(Length::Fixed(16.0)),
                    )
                    .style(move |_| container::background(t.accent))
                    .into(),
                );
            }
            let cell = metric_cell(cfg.style, r);
            // In edit mode every cell outlines; the one being dragged also fills so
            // it reads as "lifted".
            let is_dragged = dragging == Some(raw_i);
            let el: Element<'_, Message> = if editing {
                container(cell)
                    .padding([1, 4])
                    .style(move |_| container::Style {
                        border: Border {
                            color: t.accent,
                            width: 1.0,
                            radius: 5.0.into(),
                        },
                        background: is_dragged.then(|| {
                            iced::Color {
                                a: 0.18,
                                ..t.accent
                            }
                            .into()
                        }),
                        ..Default::default()
                    })
                    .into()
            } else {
                cell
            };
            let mut area = mouse_area(el).on_press(Message::StatusMetricPress(raw_i));
            if editing {
                area = area
                    .on_enter(Message::StatusMetricDragOver(raw_i))
                    .interaction(iced::mouse::Interaction::Grab);
            }
            cluster.push(area.into());
        }
        let cluster = row![
            chevron(start > 0, "‹"),
            row(cluster).spacing(14).align_y(iced::Alignment::Center),
            chevron(start + visible < total, "›"),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        // Wheel over the cluster slides the window (only meaningful when cells are
        // shed, i.e. `max_start > 0`).
        content = content
            .push(mouse_area(cluster).on_scroll(|d| Message::StatusBarScroll(scroll_delta_y(d))))
            .push(flex());
    }

    // In edit mode the right end shows a hint instead of the geometry.
    if editing {
        content = content.push(
            text("drag to reorder · Esc to finish")
                .size(STATUS_BAR_TEXT_SIZE)
                .color(t.accent),
        );
    } else {
        content = content.push(right_text());
    }

    // In edit mode a press on empty bar space (the flex gaps / hint — cells and
    // the scroll region consume their own press) exits. Otherwise the bar is
    // inert to background presses.
    let area = status_bar_content(content);
    if editing {
        mouse_area(area).on_press(Message::ExitStatusBarEdit).into()
    } else {
        area
    }
}

/// The vertical component of a wheel `ScrollDelta`, for panning the status bar.
/// The furthest the status-bar metric window can scroll: the number of
/// renderable cells minus how many currently fit (0 when everything fits). Shared
/// by the view (to clamp the window) and `update` (to clamp the scroll offset).
pub fn status_bar_scroll_max(state: &Tty) -> usize {
    let (left, right) = status_text(state);
    let cells: Vec<(usize, crate::settings::ResolvedMetric, MetricRender)> = state
        .settings
        .status_bar_metrics_indexed()
        .into_iter()
        .filter_map(|(i, cfg)| metric_render(cfg, state).map(|r| (i, cfg, r)))
        .collect();
    let visible = visible_metric_count(&cells, &left, &right, state.window_width);
    cells.len().saturating_sub(visible)
}
/// One metric's status-bar cell in its configured `style`: a sparkline of its
/// series plus the label, or (for `Number`) the label alone.
fn metric_cell<'a>(style: crate::settings::MetricStyle, r: MetricRender) -> Element<'a, Message> {
    // A graded cell past its threshold recolors its label to the alert color so
    // it reads at a glance; otherwise the muted default.
    let label = text(r.label)
        .size(STATUS_BAR_TEXT_SIZE)
        .color(r.alert.unwrap_or(theme::tokens().muted));
    // Text-only when there is no plottable data (a text metric like uptime, or a
    // rate metric whose history hasn't landed yet), regardless of style.
    let has_data = r.series.iter().any(|(v, _)| !v.is_empty());
    match style {
        _ if !has_data => label.into(),
        crate::settings::MetricStyle::Number => label.into(),
        crate::settings::MetricStyle::Sparkline => {
            let series: Vec<SparkSeries> = r
                .series
                .iter()
                .map(|(values, color)| {
                    SparkSeries::new(values.iter().map(|&v| v as f64).collect(), *color)
                })
                .collect();
            let spark = sparkline(
                Sparkline {
                    series,
                    max: r.max as f64,
                },
                44.0,
                14.0,
            );
            row![spark, label]
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into()
        }
    }
}
/// How many of the leading metric cells fit before the bar would overflow, so
/// the rightmost are shed first. The width of each piece is estimated (iced does
/// not expose measured text extents at view-build time), erring toward showing
/// all: an unknown window width (`<= 0`, before the first resize) sheds nothing.
fn visible_metric_count(
    cells: &[(usize, crate::settings::ResolvedMetric, MetricRender)],
    left: &str,
    right: &str,
    window_width: f32,
) -> usize {
    if window_width <= 0.0 {
        return cells.len();
    }
    // Rough per-glyph advance for the 13px UI font, plus the strip padding
    // ([7,14]) and the left/right end labels that always show.
    const CHAR_W: f32 = 7.0;
    const PADDING: f32 = 28.0;
    const GAP: f32 = 14.0;
    let reserved = PADDING + (left.chars().count() + right.chars().count()) as f32 * CHAR_W + GAP;
    let mut used = 0.0;
    let mut n = 0;
    for (_, cfg, r) in cells {
        let label_w = r.label.chars().count() as f32 * CHAR_W;
        let cell_w = match cfg.style {
            // sparkline (44) + inner gap (6) + label
            crate::settings::MetricStyle::Sparkline => 44.0 + 6.0 + label_w,
            crate::settings::MetricStyle::Number => label_w,
        } + GAP;
        if used + cell_w > (window_width - reserved).max(0.0) {
            break;
        }
        used += cell_w;
        n += 1;
    }
    n
}

/// The Status-bar settings section's machine-stats editor: the ordered list of
/// configured metrics (each with a style dropdown, reorder, and remove) plus a
/// row of Add buttons for the metrics not yet shown.
pub(super) fn status_bar_metrics_editor(state: &Tty) -> Element<'_, Message> {
    use crate::settings::{MetricKind, MetricStyle};

    // The rows walk the raw stored list so a row's index matches the index the
    // reorder/remove/style messages carry; the Add buttons consult the resolved
    // list for which kinds are already present.
    let raw = &state.settings.status_bar_metrics;
    let present = state.settings.status_bar_metrics();
    let mut rows: Vec<Element<'_, Message>> = vec![caption(
        "Machine stats, in display order. The bar sheds from the right when space is tight.",
    )];

    if raw.is_empty() {
        rows.push(caption("No stats shown. Add a metric below."));
    }

    for (i, cfg) in raw.iter().enumerate() {
        // Show the metric's friendly name, falling back to the raw key for an
        // entry this build does not recognize (so it can still be removed).
        let name = MetricKind::from_setting_str(&cfg.metric)
            .map(|k| k.to_string())
            .unwrap_or_else(|| cfg.metric.clone());
        let style = MetricStyle::from_setting_str(&cfg.style);
        let style_pick = select(
            MetricStyle::ALL.to_vec(),
            Some(style),
            move |s: MetricStyle| Message::StatusBarMetricStyle(i, s.as_setting_str().to_string()),
        );
        let cell = row(vec![
            text(name)
                .size(STATUS_BAR_TEXT_SIZE)
                .width(Length::Fixed(72.0))
                .into(),
            style_pick.into(),
            button::ghost("Up", Message::StatusBarMetricMove(i, -1)).into(),
            button::ghost("Down", Message::StatusBarMetricMove(i, 1)).into(),
            button::ghost("Remove", Message::StatusBarMetricRemove(i)).into(),
        ])
        .spacing(8)
        .align_y(iced::Alignment::Center);
        rows.push(cell.into());
    }

    let add_buttons: Vec<Element<'_, Message>> = MetricKind::ALL
        .iter()
        .filter(|k| !present.iter().any(|c| c.kind == **k))
        .map(|k| {
            button::secondary(
                &format!("Add {k}"),
                Message::StatusBarMetricAdd(k.as_setting_str().to_string()),
            )
            .into()
        })
        .collect();
    if !add_buttons.is_empty() {
        rows.push(caption("ADD A METRIC"));
        // Wrap into rows of four so the full metric list (now ten, with the three
        // CPU variants) doesn't overflow the panel width.
        let mut wrapped = column![].spacing(8);
        let mut current = row![].spacing(8);
        let mut n = 0;
        for btn in add_buttons {
            current = current.push(btn);
            n += 1;
            if n == 4 {
                wrapped = wrapped.push(current);
                current = row![].spacing(8);
                n = 0;
            }
        }
        if n > 0 {
            wrapped = wrapped.push(current);
        }
        rows.push(wrapped.into());
    }

    column(rows).spacing(8).into()
}

/// Grade a 0..=100 load into the theme's calm / caution / alarm colors.
fn status_text(state: &Tty) -> (String, String) {
    let Some(tab) = state.tabs.get(state.active) else {
        return (String::new(), String::new());
    };
    let (cols, rows) = match tab.focused() {
        Some(term) => {
            let s = term.screen.lock();
            (s.cols, s.rows)
        }
        None => (0, 0),
    };
    let tabs = if state.tabs.len() > 1 {
        format!(" · {} tabs", state.tabs.len())
    } else {
        String::new()
    };
    let panes = if tab.panes.len() > 1 {
        format!(" · {} panes", tab.panes.len())
    } else {
        String::new()
    };
    // Narrate the async history start so the user knows *why* an OS keychain
    // dialog might be appearing right now — and say plainly when a locked
    // (passphrase) archive or an untracked tab means nothing is being
    // recorded.
    let history = if state.history_starting {
        " · unlocking history key…"
    } else if state.history_locked {
        " · history locked — not recording"
    } else if tab.untracked {
        " · untracked — not recording"
    } else {
        ""
    };
    (
        tab.label(),
        format!(
            "{cols}×{rows}{tabs}{panes} · {}px{history}",
            state.font_size as u32
        ),
    )
}


#[cfg(test)]
#[path = "status_bar_tests.rs"]
mod tests;
