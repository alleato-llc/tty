use super::*;
use std::time::Duration;

#[test]
fn format_duration_scales_by_magnitude() {
    assert_eq!(format_duration(Duration::from_millis(900)), "900ms");
    assert_eq!(format_duration(Duration::from_millis(3400)), "3.4s");
    assert_eq!(format_duration(Duration::from_secs(125)), "2m 05s");
    assert_eq!(format_duration(Duration::from_secs(3720)), "1h 02m");
}

#[test]
fn sanitize_collapses_newlines_and_runs() {
    assert_eq!(sanitize("git   commit\n -m  x"), "git commit -m x");
}

#[test]
fn elide_truncates_with_ellipsis() {
    assert_eq!(elide("abcdef", 4), "abc…");
    assert_eq!(elide("abc", 4), "abc", "under the cap is untouched");
}

#[test]
fn applescript_escape_quotes_and_backslashes() {
    assert_eq!(applescript_escape(r#"say "hi"\ "#), r#"say \"hi\"\\ "#);
}
