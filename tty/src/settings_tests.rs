use super::*;

/// The resolved kinds of the configured metrics, in order.
fn kinds(s: &Settings) -> Vec<MetricKind> {
    s.status_bar_metrics().iter().map(|c| c.kind).collect()
}

#[test]
fn add_appends_dedupes_and_rejects_unknown() {
    let mut s = Settings::default();
    assert!(s.add_status_bar_metric("cpu"));
    assert!(s.add_status_bar_metric("mem"));
    // Duplicate and unknown are both refused, list unchanged.
    assert!(!s.add_status_bar_metric("cpu"));
    assert!(!s.add_status_bar_metric("gpu")); // not a metric we sample
    assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
}

#[test]
fn remove_drops_by_index_and_ignores_out_of_range() {
    let mut s = Settings::default();
    s.add_status_bar_metric("cpu");
    s.add_status_bar_metric("mem");
    s.remove_status_bar_metric(5); // no-op
    s.remove_status_bar_metric(0);
    assert_eq!(kinds(&s), vec![MetricKind::Mem]);
}

#[test]
fn move_reorders_and_clamps_to_the_ends() {
    let mut s = Settings::default();
    s.add_status_bar_metric("cpu");
    s.add_status_bar_metric("mem");
    // Moving CPU down past the end lands it last (clamped), not out of range.
    s.move_status_bar_metric(0, 5);
    assert_eq!(kinds(&s), vec![MetricKind::Mem, MetricKind::Cpu]);
    // And back up past the front.
    s.move_status_bar_metric(1, -5);
    assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
}

#[test]
fn style_is_set_and_canonicalized() {
    let mut s = Settings::default();
    s.add_status_bar_metric("cpu");
    s.set_status_bar_metric_style(0, "number");
    assert_eq!(s.status_bar_metrics()[0].style, MetricStyle::Number);
    // An unrecognized style falls back to the default sparkline.
    s.set_status_bar_metric_style(0, "bogus");
    assert_eq!(s.status_bar_metrics()[0].style, MetricStyle::Sparkline);
    assert_eq!(
        s.status_bar_metrics.first().map(|c| c.style.as_str()),
        Some("sparkline")
    );
}

#[test]
fn unknown_metric_is_dropped_from_resolved_not_fatal() {
    // A forward-version / hand-edited entry parses fine (strings) and is
    // simply skipped in the resolved list — it must not nuke the settings.
    let json = r#"{ "status_bar_metrics": [
        { "metric": "cpu", "style": "sparkline" },
        { "metric": "gpu", "style": "sparkline" },
        { "metric": "mem" }
    ] }"#;
    let s: Settings = serde_json::from_str(json).expect("lenient parse");
    assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
    // Missing style defaults to sparkline.
    assert_eq!(s.status_bar_metrics()[1].style, MetricStyle::Sparkline);
}

#[test]
fn deprecated_enabled_toggle_migrates_to_cpu_and_mem() {
    // An old settings file with the on/off toggle set, no ordered list yet.
    let mut s: Settings =
        serde_json::from_str(r#"{ "status_bar_metrics_enabled": true }"#).expect("parse");
    s.migrate_status_bar_metrics();
    assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
    // The deprecated flag is cleared and never serialized back.
    assert_eq!(s.status_bar_metrics_enabled, None);
    assert!(!serde_json::to_string(&s)
        .unwrap()
        .contains("status_bar_metrics_enabled"));
}

#[test]
fn migration_does_not_clobber_an_existing_list() {
    // A user who already configured the new list keeps it, toggle ignored.
    let mut s: Settings = serde_json::from_str(
        r#"{ "status_bar_metrics_enabled": true,
             "status_bar_metrics": [{ "metric": "net_rx" }] }"#,
    )
    .expect("parse");
    s.migrate_status_bar_metrics();
    assert_eq!(kinds(&s), vec![MetricKind::NetRx]);
}

#[test]
fn interval_defaults_and_clamps() {
    let mut s = Settings::default();
    assert_eq!(
        s.status_bar_metrics_interval_ms(),
        DEFAULT_METRICS_INTERVAL_MS
    );
    s.status_bar_metrics_interval_ms = Some(0);
    assert_eq!(s.status_bar_metrics_interval_ms(), MIN_METRICS_INTERVAL_MS);
    s.status_bar_metrics_interval_ms = Some(u64::MAX);
    assert_eq!(s.status_bar_metrics_interval_ms(), MAX_METRICS_INTERVAL_MS);
}

#[test]
fn metric_kind_setting_str_round_trips_every_kind() {
    // Every kind survives a store→load through its setting string, and the
    // strings are unique (so the config can't alias two kinds).
    let mut seen = std::collections::HashSet::new();
    for &k in &MetricKind::ALL {
        let s = k.as_setting_str();
        assert!(seen.insert(s), "duplicate setting string {s:?}");
        assert_eq!(MetricKind::from_setting_str(s), Some(k), "round-trip {k:?}");
    }
    assert_eq!(MetricKind::from_setting_str("gpu"), None);
}

#[test]
fn default_thresholds_grade_cpu_mem_up_and_battery_down() {
    // CPU (all three drill-ins) and memory alarm as the value climbs.
    for k in [
        MetricKind::Cpu,
        MetricKind::CpuCores,
        MetricKind::CpuAll,
        MetricKind::Mem,
    ] {
        assert_eq!(k.default_thresholds(), Some((60.0, 85.0, false)), "{k:?}");
    }
    // Battery is inverted (low charge is the concern): alarm sits below warn.
    let (warn, alarm, inverted) = MetricKind::Battery.default_thresholds().unwrap();
    assert!(inverted && alarm < warn, "battery grades downward");
    // Rate / text metrics are ungraded.
    assert_eq!(MetricKind::Clock.default_thresholds(), None);
    assert_eq!(MetricKind::NetRx.default_thresholds(), None);
    assert_eq!(MetricKind::Load.default_thresholds(), None);
}

#[test]
fn is_graded_agrees_with_default_thresholds() {
    for &k in &MetricKind::ALL {
        assert_eq!(k.is_graded(), k.default_thresholds().is_some(), "{k:?}");
    }
}

#[test]
fn open_file_command_template_substitutes_placeholders() {
    let argv = resolve_open_file_command(
        Some("code -g {file}:{line}:{col}"),
        "/w/src/main.rs",
        Some(42),
        Some(7),
    );
    assert_eq!(argv, ["code", "-g", "/w/src/main.rs:42:7"]);
    // Missing line/col in the reference default to 1.
    let argv = resolve_open_file_command(Some("vim +{line} {file}"), "a.rs", None, None);
    assert_eq!(argv, ["vim", "+1", "a.rs"]);
}

#[test]
fn open_file_command_default_hands_file_to_os_opener() {
    let argv = resolve_open_file_command(None, "/w/x.rs", Some(9), None);
    assert_eq!(argv.len(), 2);
    assert_eq!(argv[1], "/w/x.rs");
    // A blank/whitespace template falls back to the default, not an empty argv.
    assert_eq!(
        resolve_open_file_command(Some("   "), "y.rs", None, None),
        resolve_open_file_command(None, "y.rs", None, None),
    );
}
