//! System notifications for finished commands. Uses macOS `osascript` (no crate
//! dependency, no app-bundle requirement — unlike `UNUserNotificationCenter`). A
//! no-op on other platforms for now.

use cathode::screen::CommandCompletion;

/// Longest command text shown in a notification body before eliding — keeps a
/// pasted one-liner from filling the banner.
const MAX_COMMAND_CHARS: usize = 80;

/// Post a "command finished" notification for `c` (exit status as a ✓/✗ prefix,
/// the command text, and its duration). Called by the host only when the window is
/// unfocused and the command ran past the configured threshold.
pub fn command_finished(c: &CommandCompletion) {
    let ok = matches!(c.exit_code, Some(0) | None);
    let mark = if ok { "✓" } else { "✗" };
    let title = match c.exit_code {
        Some(code) if !ok => format!("{mark} Command failed ({code})"),
        _ => format!("{mark} Command finished"),
    };
    let cmd = elide(&sanitize(&c.command), MAX_COMMAND_CHARS);
    let dur = format_duration(c.duration);
    let body = if cmd.is_empty() {
        format!("Took {dur}")
    } else {
        format!("{cmd} — {dur}")
    };
    display(&title, &body);
}

/// Collapse whitespace/newlines to single spaces so the text stays one line in the
/// AppleScript string (and the banner).
fn sanitize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// A compact human duration: `900ms`, `3.4s`, `2m 05s`, `1h 02m`.
fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = d.as_secs();
    if secs < 60 {
        return format!("{:.1}s", d.as_secs_f64());
    }
    if secs < 3600 {
        return format!("{}m {:02}s", secs / 60, secs % 60);
    }
    format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
}

/// Fire a notification via `osascript`. Best-effort: a spawn failure is logged, not
/// surfaced. macOS only.
fn display(title: &str, body: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        applescript_escape(body),
        applescript_escape(title),
    );
    if let Err(e) = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
    {
        tracing::warn!("osascript notification failed: {e}");
    }
}

/// Escape a string for embedding in an AppleScript double-quoted literal.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
