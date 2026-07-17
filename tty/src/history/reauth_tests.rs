use std::time::{Duration, Instant};

use super::*;

#[test]
fn never_authenticated_this_session_is_always_due() {
    let now = Instant::now();
    assert!(is_due(None, None, now));
    assert!(is_due(None, Some(30), now));
}

#[test]
fn authenticated_with_no_interval_configured_is_not_due_again() {
    let now = Instant::now();
    assert!(!is_due(Some(now), None, now));
    assert!(!is_due(
        Some(now),
        None,
        now + Duration::from_secs(60 * 60 * 24)
    ));
}

#[test]
fn authenticated_within_the_interval_is_not_due() {
    let now = Instant::now();
    assert!(!is_due(
        Some(now),
        Some(30),
        now + Duration::from_secs(60 * 10)
    ));
}

#[test]
fn authenticated_past_the_interval_is_due_again() {
    let now = Instant::now();
    assert!(is_due(
        Some(now),
        Some(30),
        now + Duration::from_secs(60 * 31)
    ));
}

#[test]
fn interval_boundary_is_inclusive() {
    let now = Instant::now();
    assert!(is_due(
        Some(now),
        Some(30),
        now + Duration::from_secs(60 * 30)
    ));
}
