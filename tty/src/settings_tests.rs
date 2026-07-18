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

#[test]
fn toml_roundtrips_the_fields_it_writes() {
    let s = Settings {
        theme: Some("nord".into()),
        font_size: Some(15.0),
        shell_integration: ShellIntegration {
            notify: Some(false),
            ..Default::default()
        },
        open_file_command: Some("code -g {file}:{line}:{col}".into()),
        status_bar_metrics: vec![MetricConfig {
            metric: "cpu".into(),
            style: "sparkline".into(),
            warn: Some(70.0),
            alarm: None,
        }],
        ..Default::default()
    };
    let doc = toml_edit::ser::to_document(&s).expect("serialize");
    let back = Settings::from_toml_str(&doc.to_string()).expect("parse");
    assert_eq!(back.theme.as_deref(), Some("nord"));
    assert_eq!(back.font_size, Some(15.0));
    assert_eq!(back.shell_integration.notify, Some(false));
    assert_eq!(
        back.open_file_command.as_deref(),
        Some("code -g {file}:{line}:{col}")
    );
    assert_eq!(back.status_bar_metrics.len(), 1);
    assert_eq!(back.status_bar_metrics[0].warn, Some(70.0));
}

#[test]
fn unset_options_are_omitted_not_written_as_null() {
    // TOML has no null; an unset Option must simply be absent (no "theme = ..." line,
    // no "null" anywhere).
    let s = Settings::default();
    let out = toml_edit::ser::to_document(&s)
        .expect("serialize")
        .to_string();
    assert!(!out.contains("null"), "no nulls:\n{out}");
    assert!(!out.contains("theme"), "unset theme is absent:\n{out}");
}

#[test]
fn save_merge_preserves_comments_and_updates_values() {
    // A hand-edited file with a full-line comment and an inline comment.
    let existing = "\
# my terminal config
font_size = 12  # a touch bigger
theme = \"gruvbox\"
";
    let mut doc = existing.parse::<toml_edit::DocumentMut>().unwrap();
    // The GUI changed font_size and theme.
    let s = Settings {
        font_size: Some(18.0),
        theme: Some("nord".into()),
        ..Default::default()
    };
    let fresh = toml_edit::ser::to_document(&s).unwrap();
    merge_into_doc(&mut doc, &fresh);
    let out = doc.to_string();
    assert!(
        out.contains("# my terminal config"),
        "full-line comment kept:\n{out}"
    );
    assert!(
        out.contains("# a touch bigger"),
        "inline comment kept:\n{out}"
    );
    assert!(out.contains("18"), "font_size updated:\n{out}");
    assert!(out.contains("nord"), "theme updated:\n{out}");
    assert!(!out.contains("gruvbox"), "old value replaced:\n{out}");
}

#[test]
fn save_merge_drops_a_cleared_setting() {
    // theme is set on disk but the schema no longer emits it (cleared to None).
    let mut doc = "theme = \"nord\"\nfont_size = 14\n"
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    let s = Settings {
        font_size: Some(14.0), // theme stays None
        ..Default::default()
    };
    let fresh = toml_edit::ser::to_document(&s).unwrap();
    merge_into_doc(&mut doc, &fresh);
    let out = doc.to_string();
    assert!(!out.contains("theme"), "cleared setting removed:\n{out}");
    assert!(out.contains("font_size"), "kept setting stays:\n{out}");
}

#[test]
fn toml_serializes_full_settings_with_palette_and_overrides() {
    // The `values before tables` TOML rule is a trap: `palette` (a table) and the
    // arrays-of-tables sit amid scalar fields in the struct. Serializing a
    // fully-populated Settings must still succeed and round-trip.
    let s = Settings {
        theme: Some("nord".into()),
        font_size: Some(14.0),
        window_always_on_top: Some(true),
        shell_integration: ShellIntegration {
            notify_min_seconds: Some(30),
            gutter: Some(true),
            ..Default::default()
        },
        palette: Some(Palette {
            ansi: (0..16).map(|i| format!("#0000{i:02x}")).collect(),
            fg: "#ffffff".into(),
            bg: "#000000".into(),
            cursor: "#ff8800".into(),
        }),
        output_line_overrides: vec![OutputLineOverride {
            pattern: "tail *".into(),
            max_lines: 200,
        }],
        status_bar_metrics: vec![MetricConfig {
            metric: "cpu".into(),
            style: "sparkline".into(),
            warn: None,
            alarm: None,
        }],
        ..Default::default()
    };
    let out = toml_edit::ser::to_document(&s)
        .expect("serialize full settings")
        .to_string();
    let back = Settings::from_toml_str(&out).expect("parse full settings");
    assert_eq!(back.palette.as_ref().map(|p| p.ansi.len()), Some(16));
    assert_eq!(back.output_line_overrides.len(), 1);
    assert_eq!(back.window_always_on_top, Some(true));
    assert_eq!(back.shell_integration.notify_min_seconds, Some(30));
    assert_eq!(back.shell_integration.gutter, Some(true));
}

#[test]
fn legacy_json_migrates_cleanly_to_toml() {
    // A representative old `tty.settings.json` (what serde_json::to_string_pretty
    // wrote): the migration reads it via serde_json, then the next save serializes to
    // TOML — the fields must survive both hops.
    let json = r#"{
        "theme": "gruvbox",
        "font_size": 13.0,
        "palette": null,
        "status_bar_metrics": [{ "metric": "cpu", "style": "sparkline" }],
        "encrypted_history_enabled": true,
        "history_key_source": "passphrase"
    }"#;
    let migrated: Settings = serde_json::from_str(json).expect("legacy json parses");
    let toml = toml_edit::ser::to_document(&migrated)
        .expect("serialize to toml")
        .to_string();
    let back = Settings::from_toml_str(&toml).expect("reparse toml");
    assert_eq!(back.theme.as_deref(), Some("gruvbox"));
    assert_eq!(back.font_size, Some(13.0));
    assert_eq!(back.status_bar_metrics.len(), 1);
    assert_eq!(back.encrypted_history_enabled, Some(true));
    assert_eq!(back.history_key_source.as_deref(), Some("passphrase"));
}

#[test]
fn migrates_flat_osc133_keys_into_the_group() {
    // An old file with the flat keys parses, then migrate folds them into the group.
    let toml = "notify_on_command_finish = false\nprompt_gutter = true\n\
                shell_integration_autoinstall = true\nnotify_command_min_seconds = 20\n";
    let mut s = Settings::from_toml_str(toml).expect("parse legacy flat keys");
    s.migrate_shell_integration();
    assert_eq!(s.shell_integration.notify, Some(false));
    assert_eq!(s.shell_integration.gutter, Some(true));
    assert_eq!(s.shell_integration.autoinstall, Some(true));
    assert_eq!(s.shell_integration.notify_min_seconds, Some(20));
    // The flat keys are cleared and everything now lives under the grouped key.
    let out = toml_edit::ser::to_document(&s).unwrap().to_string();
    assert!(
        !out.contains("notify_on_command_finish"),
        "flat keys gone:\n{out}"
    );
    assert!(
        out.contains("shell_integration = {"),
        "grouped under one key:\n{out}"
    );
    assert!(
        out.contains("gutter = true"),
        "sub-option preserved:\n{out}"
    );
    // And it round-trips back into the group.
    let back = Settings::from_toml_str(&out).expect("reparse");
    assert_eq!(back.shell_integration.gutter, Some(true));
}

#[test]
fn migration_prefers_an_already_grouped_value() {
    // Both a flat key and its new home set → the group wins, the flat one is dropped.
    let mut s = Settings {
        shell_integration: ShellIntegration {
            notify: Some(true),
            ..Default::default()
        },
        notify_on_command_finish: Some(false),
        ..Default::default()
    };
    s.migrate_shell_integration();
    assert_eq!(s.shell_integration.notify, Some(true));
    assert_eq!(s.notify_on_command_finish, None);
}

#[test]
fn shell_integration_master_gate_disables_sub_options() {
    let s = Settings {
        shell_integration: ShellIntegration {
            enabled: Some(false),
            notify: Some(true),
            gutter: Some(true),
            autoinstall: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let si = s.shell_integration();
    assert!(!si.enabled);
    assert!(!si.notify, "master off gates notify");
    assert!(!si.gutter, "master off gates gutter");
    assert!(!si.autoinstall, "master off gates autoinstall");
}

#[test]
fn shell_integration_defaults_when_absent() {
    let si = Settings::default().shell_integration();
    assert!(si.enabled, "on by default");
    assert!(si.notify, "notify on by default");
    assert!(!si.gutter, "gutter off by default");
    assert!(!si.autoinstall, "autoinstall off by default");
    assert_eq!(si.notify_min_seconds, DEFAULT_NOTIFY_MIN_SECONDS);
    // A pristine config writes no [shell_integration] block.
    let out = toml_edit::ser::to_document(&Settings::default())
        .unwrap()
        .to_string();
    assert!(!out.contains("shell_integration"), "no empty block:\n{out}");
}

#[test]
fn env_overlay_round_trips_through_toml() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("EDITOR".to_string(), "nvim".to_string());
    env.insert("API_KEY".to_string(), "sk-with spaces".to_string());
    let s = Settings {
        env,
        ..Default::default()
    };
    let out = toml_edit::ser::to_document(&s)
        .expect("serialize")
        .to_string();
    let back = Settings::from_toml_str(&out).expect("parse");
    assert_eq!(back.env.get("EDITOR").map(String::as_str), Some("nvim"));
    assert_eq!(
        back.env.get("API_KEY").map(String::as_str),
        Some("sk-with spaces")
    );
    // Empty overlay writes no `env` key.
    let empty = toml_edit::ser::to_document(&Settings::default())
        .unwrap()
        .to_string();
    assert!(!empty.contains("env"), "no empty env block:\n{empty}");
}

#[test]
fn env_editing_defaults_off_and_is_master_gated() {
    // Off by default (opt-in — it types into your shell).
    assert!(!Settings::default().shell_integration().env_editing);
    // On when explicitly enabled...
    let on = Settings {
        shell_integration: ShellIntegration {
            env_editing: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(on.shell_integration().env_editing);
    // ...but the master switch still gates it.
    let gated = Settings {
        shell_integration: ShellIntegration {
            enabled: Some(false),
            env_editing: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!gated.shell_integration().env_editing);
}

#[test]
fn env_view_defaults_off_and_editing_implies_it() {
    assert!(
        !Settings::default().shell_integration().env_view,
        "off by default"
    );
    // Enabling editing turns the view on too (you can't edit what you can't see).
    let editing = Settings {
        shell_integration: ShellIntegration {
            env_editing: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let si = editing.shell_integration();
    assert!(si.env_view && si.env_editing, "editing implies the view");
    // The master switch gates both.
    let gated = Settings {
        shell_integration: ShellIntegration {
            enabled: Some(false),
            env_view: Some(true),
            env_editing: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let si = gated.shell_integration();
    assert!(!si.env_view && !si.env_editing);
}
