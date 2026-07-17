use super::*;

fn sample() -> PersistedCommandEntry {
    PersistedCommandEntry {
        id: 7,
        command: "ls -la".to_string(),
        started_at_epoch_ms: 1_750_000_000_000,
        pane_tag: "Tab 1".to_string(),
    }
}

#[test]
fn persisted_command_entry_round_trips_through_json() {
    let entry = sample();
    let json = serde_json::to_string(&entry).unwrap();
    let back: PersistedCommandEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back, entry);
}

#[test]
fn epoch_ms_converts_a_system_time_and_back() {
    let t = wall_time_ms(1_750_000_000_000);
    assert_eq!(epoch_ms(t), 1_750_000_000_000);
}

// Exposed to this test module only, mirroring `screen.rs`'s private
// `wall_time_from_epoch_ms` — kept here rather than made `pub` on the real type,
// since nothing outside tests needs the inverse of `epoch_ms`.
fn wall_time_ms(epoch_ms: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(epoch_ms)
}
