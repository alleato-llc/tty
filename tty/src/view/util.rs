//! Small, side-effect-free helpers for the view layer: threshold grading, value
//! formatting, and label mapping. Kept pure (data in, value out) so they're unit
//! tested in `util_tests.rs` without rendering an `Element`.

use iced::Color;
use rime::theme;

/// A graded cell's alert level against its warn/alarm thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Grade {
    Calm,
    Warn,
    Alarm,
}

/// Grade `value` against `warn`/`alarm` cutoffs. Normal metrics alarm when the
/// value climbs *past* the cutoffs (CPU, memory); `inverted` metrics alarm when
/// it *falls below* them (battery — low charge is the concern).
pub(super) fn grade(value: f32, warn: f32, alarm: f32, inverted: bool) -> Grade {
    if inverted {
        if value <= alarm {
            Grade::Alarm
        } else if value <= warn {
            Grade::Warn
        } else {
            Grade::Calm
        }
    } else if value >= alarm {
        Grade::Alarm
    } else if value >= warn {
        Grade::Warn
    } else {
        Grade::Calm
    }
}

/// The theme color for a [`Grade`]: calm / caution / alarm.
pub(super) fn grade_color(g: Grade) -> Color {
    let t = theme::tokens();
    match g {
        Grade::Alarm => t.danger,
        Grade::Warn => t.warn,
        Grade::Calm => t.success,
    }
}

/// Grade a 0..100 load percentage straight to a color (caution ≥60, alarm ≥85).
pub(super) fn load_color(percent: f32) -> Color {
    grade_color(grade(percent, 60.0, 85.0, false))
}

/// A per-core sparkline's color: efficiency cores read accent, performance cores
/// grade by their current load.
pub(super) fn core_color(kind: prexp_core::system::CpuKind, cur: f32) -> Color {
    match kind {
        prexp_core::system::CpuKind::Efficiency => theme::tokens().accent,
        _ => load_color(cur),
    }
}

/// Truncate `name` to `max` characters, appending `…` when it's cut (counts by
/// char, so it's safe on multi-byte text).
pub(super) fn truncate_name(name: &str, max: usize) -> String {
    if name.chars().count() > max {
        format!(
            "{}…",
            name.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        name.to_string()
    }
}

/// Shorten a string for display inside a fixed-size dialog. Cuts on a char
/// boundary, appends `…`.
pub(super) fn elide(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// A coarse "N ago" label for an elapsed duration (seconds → minutes → hours →
/// days).
pub(super) fn format_age(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// The vertical component of a scroll delta, in either unit.
pub(super) fn scroll_delta_y(delta: iced::mouse::ScrollDelta) -> f32 {
    match delta {
        iced::mouse::ScrollDelta::Lines { y, .. } => y,
        iced::mouse::ScrollDelta::Pixels { y, .. } => y,
    }
}

/// A short label for an open file-descriptor kind (the process detail's fd list).
pub(super) fn resource_kind_label(kind: &prexp_core::models::ResourceKind) -> &'static str {
    use prexp_core::models::ResourceKind as R;
    match kind {
        R::File => "file",
        R::Socket => "sock",
        R::Pipe => "pipe",
        R::Device => "dev",
        R::Kqueue => "kq",
        R::Unknown => "?",
    }
}

/// Hover-readout formatters for the drill-in line charts.
pub(super) fn hover_percent(v: f64) -> String {
    format!("{}%", v.round() as u32)
}
pub(super) fn hover_rate(v: f64) -> String {
    crate::metrics::format_rate(v as f32)
}
pub(super) fn hover_load(v: f64) -> String {
    format!("{v:.2}")
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
