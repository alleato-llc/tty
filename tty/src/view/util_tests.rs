use super::*;

#[test]
fn grade_normal_metric_climbs_past_cutoffs() {
    // CPU/memory: alarm as the value rises. warn 60, alarm 85.
    assert_eq!(grade(30.0, 60.0, 85.0, false), Grade::Calm);
    assert_eq!(grade(70.0, 60.0, 85.0, false), Grade::Warn);
    assert_eq!(grade(90.0, 60.0, 85.0, false), Grade::Alarm);
    // Cutoffs are inclusive on the way up.
    assert_eq!(grade(60.0, 60.0, 85.0, false), Grade::Warn);
    assert_eq!(grade(85.0, 60.0, 85.0, false), Grade::Alarm);
    // Just under a cutoff stays a step calmer.
    assert_eq!(grade(59.9, 60.0, 85.0, false), Grade::Calm);
    assert_eq!(grade(84.9, 60.0, 85.0, false), Grade::Warn);
}

#[test]
fn grade_inverted_metric_falls_below_cutoffs() {
    // Battery: alarm as the value drops. warn 40, alarm 20 (alarm < warn).
    assert_eq!(grade(90.0, 40.0, 20.0, true), Grade::Calm);
    assert_eq!(grade(30.0, 40.0, 20.0, true), Grade::Warn);
    assert_eq!(grade(10.0, 40.0, 20.0, true), Grade::Alarm);
    // Cutoffs are inclusive on the way down.
    assert_eq!(grade(40.0, 40.0, 20.0, true), Grade::Warn);
    assert_eq!(grade(20.0, 40.0, 20.0, true), Grade::Alarm);
    // Just above a cutoff stays a step calmer.
    assert_eq!(grade(40.1, 40.0, 20.0, true), Grade::Calm);
    assert_eq!(grade(20.1, 40.0, 20.0, true), Grade::Warn);
}

#[test]
fn truncate_name_appends_ellipsis_only_when_cut() {
    assert_eq!(truncate_name("rustc", 16), "rustc");
    // Exactly at the budget is left alone.
    assert_eq!(truncate_name("abcdef", 6), "abcdef");
    // Over budget: keep `max - 1` chars + the ellipsis (total `max` glyphs).
    assert_eq!(
        truncate_name("com.apple.WebKit.WebContent", 10),
        "com.apple…"
    );
    assert_eq!(truncate_name("abcdefg", 4).chars().count(), 4);
    // Multi-byte is counted by char, not byte.
    assert_eq!(truncate_name("héllo wörld", 4), "hél…");
}

#[test]
fn elide_cuts_on_char_boundary() {
    assert_eq!(elide("short", 10), "short");
    assert_eq!(elide("abcdefgh", 5), "abcde…");
    assert_eq!(elide("日本語テスト", 3), "日本語…");
}

#[test]
fn format_age_steps_through_units() {
    use std::time::Duration;
    assert_eq!(format_age(Duration::from_secs(0)), "0s ago");
    assert_eq!(format_age(Duration::from_secs(59)), "59s ago");
    assert_eq!(format_age(Duration::from_secs(60)), "1m ago");
    assert_eq!(format_age(Duration::from_secs(3599)), "59m ago");
    assert_eq!(format_age(Duration::from_secs(3600)), "1h ago");
    assert_eq!(format_age(Duration::from_secs(86_399)), "23h ago");
    assert_eq!(format_age(Duration::from_secs(86_400)), "1d ago");
    assert_eq!(format_age(Duration::from_secs(200_000)), "2d ago");
}

#[test]
fn scroll_delta_y_reads_either_unit() {
    use iced::mouse::ScrollDelta;
    assert_eq!(scroll_delta_y(ScrollDelta::Lines { x: 0.0, y: 3.0 }), 3.0);
    assert_eq!(
        scroll_delta_y(ScrollDelta::Pixels { x: 0.0, y: -12.5 }),
        -12.5
    );
}

#[test]
fn resource_kind_labels_are_stable() {
    use prexp_core::models::ResourceKind as R;
    assert_eq!(resource_kind_label(&R::File), "file");
    assert_eq!(resource_kind_label(&R::Socket), "sock");
    assert_eq!(resource_kind_label(&R::Pipe), "pipe");
    assert_eq!(resource_kind_label(&R::Device), "dev");
    assert_eq!(resource_kind_label(&R::Kqueue), "kq");
    assert_eq!(resource_kind_label(&R::Unknown), "?");
}

#[test]
fn hover_formatters_round_and_fix_precision() {
    assert_eq!(hover_percent(49.6), "50%");
    assert_eq!(hover_percent(0.4), "0%");
    assert_eq!(hover_load(1.234), "1.23");
    assert_eq!(hover_load(2.0), "2.00");
}
