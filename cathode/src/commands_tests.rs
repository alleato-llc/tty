use super::*;

#[test]
fn glob_match_is_fully_anchored() {
    assert!(glob_match("ls", "ls"));
    assert!(!glob_match("ls", "ls -la"), "no substring matching");
    assert!(
        !glob_match("ls", "als"),
        "no substring matching, either end"
    );
}

#[test]
fn glob_match_star_matches_any_run() {
    assert!(glob_match("tail *", "tail -f /var/log/syslog"));
    assert!(glob_match("tail *", "tail foo.txt"));
    assert!(!glob_match("tail *", "head foo.txt"));
    assert!(
        !glob_match("tail *", "tail"),
        "star still needs something to match"
    );
}

#[test]
fn glob_match_question_mark_matches_one_char() {
    assert!(glob_match("l?", "ls"));
    assert!(!glob_match("l?", "lss"));
}

#[test]
fn glob_match_is_case_sensitive() {
    assert!(!glob_match("Tail *", "tail -f x"));
}

#[test]
fn resolve_output_cap_uses_first_matching_override() {
    let overrides = vec![("tail *".to_string(), 200), ("ping *".to_string(), 1000)];
    assert_eq!(resolve_output_cap("tail -f x.log", &overrides, 50), 200);
    assert_eq!(resolve_output_cap("ping example.com", &overrides, 50), 1000);
}

#[test]
fn resolve_output_cap_falls_back_to_default() {
    let overrides = vec![("tail *".to_string(), 200)];
    assert_eq!(resolve_output_cap("ls -la", &overrides, 50), 50);
}

#[test]
fn resolve_output_cap_with_no_overrides_is_the_default() {
    assert_eq!(resolve_output_cap("anything", &[], 50), 50);
}
