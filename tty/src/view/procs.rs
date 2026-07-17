//! The Processes drill-in: the sortable, scrollable table of every process
//! (`procs_body`) and the per-process detail it drills into — a live CPU chart,
//! memory / thread count, and the scrollable open-file-descriptor list
//! (`proc_detail_body`). Split out of `view/metrics.rs`; `metric_body` there
//! dispatches the Procs kind here.

use iced::widget::{column, container, mouse_area, row, scrollable, text};
use iced::{Element, Length};

use rime::theme;
use rime::widgets::{line_chart, table, CellAlign, LineChart, Series, TableColumn, TableMetrics};

use crate::message::Message;
use crate::state::Tty;

use super::util::*;

/// The Processes drill-in. Normally a clickable header row (re-sort by clicking a
/// column) over a virtualized, scrollable `rime` table of every process, ordered
/// by the active sort; the bar cell shows only the busiest process, this is the
/// list. Double- or right-clicking a row drills into that one process's detail
/// (fds + a live chart), rendered by [`proc_detail_body`] instead.
pub(super) fn procs_body(state: &Tty, card_w: f32, chart_h: f32) -> Element<'_, Message> {
    use crate::state::ProcSortColumn as Col;
    let t = theme::tokens();

    // A per-process detail takes over the whole card when one is open.
    if state.proc_detail_pid.is_some() {
        return proc_detail_body(state, chart_h);
    }

    let procs = &state.metrics.processes;
    if procs.is_empty() {
        return column![
            text("Processes").size(14).color(t.ink),
            text("Collecting…").size(12).color(t.muted),
        ]
        .spacing(8)
        .into();
    }

    let (sort_col, desc) = state.proc_sort;
    // Row order: indices into `procs` sorted by the active column/direction.
    let mut order: Vec<usize> = (0..procs.len()).collect();
    order.sort_by(|&a, &b| {
        let (pa, pb) = (&procs[a], &procs[b]);
        let cmp = match sort_col {
            Col::Name => pa.name.to_lowercase().cmp(&pb.name.to_lowercase()),
            Col::Cpu => pa
                .cpu_percent
                .partial_cmp(&pb.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Mem => pa.memory_bytes.cmp(&pb.memory_bytes),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // Column widths (must match the header row and the table below); `CELL_PAD`
    // in the table is 8px, so the header cells pad to match.
    const NUM_W: f32 = 64.0;
    // The name column is the fill remainder; truncate names to what fits so a long
    // one (e.g. `com.apple.WebKit.WebContent`) clips with an ellipsis rather than
    // spilling into the numbers. ~7px per glyph at the table's 13px, minus padding.
    let name_px = (card_w - 28.0 - 2.0 * NUM_W - 16.0).max(40.0);
    let name_budget = (name_px / 7.0) as usize;

    let arrow = move |c: Col| -> &'static str {
        if sort_col == c {
            if desc {
                " ▾"
            } else {
                " ▴"
            }
        } else {
            ""
        }
    };
    let header_cell = |label: &str, col: Col, width: Length, right: bool| -> Element<'_, Message> {
        let color = if sort_col == col { t.ink } else { t.muted };
        let txt = text(format!("{label}{}", arrow(col))).size(11).color(color);
        let mut c = container(txt).width(width).padding([0, 8]);
        if right {
            c = c.align_x(iced::alignment::Horizontal::Right);
        }
        mouse_area(c)
            .on_press(Message::SetProcSort(col))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    };
    let header = row![
        header_cell("PROCESS", Col::Name, Length::Fill, false),
        header_cell("CPU", Col::Cpu, Length::Fixed(NUM_W), true),
        header_cell("MEM", Col::Mem, Length::Fixed(NUM_W), true),
    ]
    .align_y(iced::Alignment::Center);

    // Sorted-order metadata for the callbacks: (pid, name) so a right-clicked row
    // opens its context menu, and the CPU% per row so a hog can be graded a color.
    let row_meta: Vec<(i32, String)> = order
        .iter()
        .map(|&i| (procs[i].pid, procs[i].name.clone()))
        .collect();
    let cpu_by_row: Vec<f32> = order.iter().map(|&i| procs[i].cpu_percent).collect();
    // Grade the CPU% cell by the same cutoffs as the CPU status-bar cell, so a
    // busy process reads amber (>=60%) / red (>=85%) at a glance.
    let (warn, alarm, _) = crate::settings::MetricKind::Cpu
        .default_thresholds()
        .unwrap_or((60.0, 85.0, false));
    let (warn, alarm) = (warn as f32, alarm as f32);

    // The virtualized body. The cell closure owns the sorted order and borrows the
    // process list.
    let rows = order.len();
    let cell = move |row: usize, col: usize| -> String {
        let p = &procs[order[row]];
        match col {
            0 => truncate_name(&p.name, name_budget),
            1 => format!("{}%", p.cpu_percent.round() as i32),
            _ => crate::metrics::format_bytes(p.memory_bytes),
        }
    };
    let cell_color = move |row: usize, col: usize| -> Option<iced::Color> {
        if col != 1 {
            return None;
        }
        match grade(cpu_by_row[row], warn, alarm, false) {
            Grade::Calm => None,
            g => Some(grade_color(g)),
        }
    };
    let columns = vec![
        TableColumn::fill("").align(CellAlign::Left),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
    ];
    let body = table(rows, columns, cell)
        .cell_color(cell_color)
        .metrics(TableMetrics {
            row_height: 22.0,
            header_height: 0.0,
        })
        .offset(state.proc_table_scroll)
        .on_scroll(Message::ProcTableScroll)
        // Right-click opens the row's context menu (View fds / Copy path / PID /
        // name); it is the way into the per-process detail.
        .on_right_click(move |row| {
            let (pid, name) = &row_meta[row];
            Message::ProcRowRightClick(*pid, name.clone())
        });

    column![
        text(format!("Processes — {}", rows)).size(14).color(t.ink),
        header,
        container(body)
            .height(Length::Fixed(chart_h))
            .width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// One process's detail, shown in place of the process list when a row is
/// double- or right-clicked: a "‹ Back" control, a live CPU% chart (its history
/// is fresh for this process — we do not retain a series for every process), the
/// current memory / thread count, and the scrollable list of open file
/// descriptors. Refreshed each sample by [`crate::metrics::Metrics::sample_proc_detail`].
fn proc_detail_body(state: &Tty, chart_h: f32) -> Element<'_, Message> {
    use prexp_core::models::ResourceKind;
    let t = theme::tokens();
    let Some(d) = state.metrics.proc_detail.as_ref() else {
        // The process exited between opening and rendering; offer the way back.
        return column![
            back_row("‹ Back"),
            text("That process is no longer running.")
                .size(12)
                .color(t.muted),
        ]
        .spacing(8)
        .into();
    };

    // A "‹ Back" control, then the process identity.
    let head = column![
        back_row(&format!("‹ {}", truncate_name(&d.name, 28))),
        text(format!("pid {} · {} threads", d.pid, d.thread_count))
            .size(11)
            .color(t.muted),
    ]
    .spacing(2);

    // The live CPU chart. It needs a couple of points to draw a segment; until
    // then (just opened) it shows a note.
    let cpu_now = d.cpu_history.back().copied().unwrap_or(0.0);
    let chart: Element<'_, Message> = if d.cpu_history.len() >= 2 {
        let series = vec![Series {
            points: d
                .cpu_history
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v as f64))
                .collect(),
            color: t.accent,
        }];
        let peak = d.cpu_history.iter().copied().fold(1.0_f32, f32::max);
        line_chart(
            LineChart {
                title: "CPU".to_string(),
                series,
                y_max: None,
                y_max_label: Some(format!("{}%", peak.round() as i32)),
                hover_format: Some(hover_percent),
            },
            (chart_h * 0.42).max(48.0),
        )
    } else {
        container(text("Collecting CPU…").size(12).color(t.muted))
            .height(Length::Fixed((chart_h * 0.42).max(48.0)))
            .into()
    };

    let stats = text(format!(
        "CPU {}%   ·   Mem {}",
        cpu_now.round() as i32,
        crate::metrics::format_bytes(d.memory_bytes),
    ))
    .size(13)
    .color(t.ink);

    // The file descriptors: a summary count line, then a scrollable list. When the
    // OS denied fd access we say so instead of showing an empty list.
    let files = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::File)
        .count();
    let sockets = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::Socket)
        .count();
    let fd_head = text(format!(
        "Open files — {} ({} files, {} sockets)",
        d.resources.len(),
        files,
        sockets,
    ))
    .size(12)
    .color(t.muted);

    let fd_list: Element<'_, Message> = if !d.accessible {
        text("fd access denied by the OS for this process.")
            .size(12)
            .color(t.muted)
            .into()
    } else if d.resources.is_empty() {
        text("No open file descriptors.")
            .size(12)
            .color(t.muted)
            .into()
    } else {
        let mut list = column![].spacing(1);
        for r in &d.resources {
            let kind = resource_kind_label(&r.kind);
            let path = r.path.as_deref().unwrap_or("—");
            let line = container(
                text(format!("{:>3}  {:<6} {}", r.descriptor, kind, path))
                    .size(11)
                    .color(t.muted)
                    .font(iced::Font::MONOSPACE),
            )
            .width(Length::Fill);
            // Right-click a descriptor with a path to copy it — the whole row is
            // the hit target (not just the glyphs).
            list = list.push(match &r.path {
                Some(p) => mouse_area(line)
                    .on_right_press(Message::FdRowRightClick(p.clone()))
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into(),
                None => Element::from(line),
            });
        }
        scrollable(list).height(Length::Fill).into()
    };

    column![
        head,
        chart,
        stats,
        fd_head,
        container(fd_list).height(Length::Fill).width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// A clickable "‹ Back" row that returns the Processes drill-in to the list.
fn back_row(label: &str) -> Element<'static, Message> {
    mouse_area(text(label.to_string()).size(14).color(theme::tokens().ink))
        .on_press(Message::CloseProcDetail)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}
