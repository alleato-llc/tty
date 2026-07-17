use super::*;
use crate::settings::{MetricKind, MetricStyle, ResolvedMetric};

fn cell(style: MetricStyle) -> (usize, ResolvedMetric, MetricRender) {
    (
        0,
        ResolvedMetric {
            kind: MetricKind::Cpu,
            style,
            warn: 60.0,
            alarm: 85.0,
        },
        MetricRender {
            label: "CPU".to_string(),
            series: vec![(Default::default(), iced::Color::WHITE)],
            max: 100.0,
            alert: None,
        },
    )
}

#[test]
fn shedding_shows_all_when_width_is_unknown_or_ample() {
    let cells = [cell(MetricStyle::Sparkline), cell(MetricStyle::Sparkline)];
    // Unknown width (pre-first-resize) sheds nothing.
    assert_eq!(visible_metric_count(&cells, "z", "z", 0.0), 2);
    // A wide window fits both.
    assert_eq!(visible_metric_count(&cells, "z", "z", 5000.0), 2);
}

#[test]
fn shedding_drops_rightmost_cells_as_width_shrinks() {
    let cells = [cell(MetricStyle::Sparkline), cell(MetricStyle::Sparkline)];
    // reserved = 28 + (1+1)*7 + 14 = 56; each sparkline cell = 44+6+21 +14 = 85.
    assert_eq!(visible_metric_count(&cells, "z", "z", 141.0), 1);
    assert_eq!(visible_metric_count(&cells, "z", "z", 66.0), 0);
    // Monotonic: never more visible in a narrower window.
    let wide = visible_metric_count(&cells, "z", "z", 5000.0);
    let narrow = visible_metric_count(&cells, "z", "z", 141.0);
    assert!(narrow <= wide);
}
