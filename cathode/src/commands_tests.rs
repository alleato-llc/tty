use super::*;

#[test]
fn strip_prompt_cuts_common_shell_prompts() {
    assert_eq!(strip_prompt("$ ls -la"), "ls -la");
    assert_eq!(strip_prompt("user@host tty % cargo build"), "cargo build");
    assert_eq!(strip_prompt("user@host:~/dev$ git status"), "git status");
    assert_eq!(strip_prompt("❯ npm test"), "npm test");
    assert_eq!(strip_prompt("# apt-get update"), "apt-get update");
}

#[test]
fn strip_prompt_uses_the_earliest_marker_so_command_text_survives() {
    // The real prompt marker comes before any marker-like text *inside* the
    // command — cutting at the earliest keeps `echo $ hi` intact.
    assert_eq!(strip_prompt("user@host % echo $ hi"), "echo $ hi");
    assert_eq!(strip_prompt("$ echo 2 > out.txt"), "echo 2 > out.txt");
}

#[test]
fn strip_prompt_fails_toward_the_full_line() {
    // No recognizable marker (an exotic prompt): unchanged.
    assert_eq!(strip_prompt("➜ dorado cargo test"), "➜ dorado cargo test");
    // Stripping would leave nothing (a bare prompt): unchanged.
    assert_eq!(strip_prompt("user@host % "), "user@host % ");
    assert_eq!(strip_prompt(""), "");
}

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
