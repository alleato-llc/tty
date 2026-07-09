//! Resolving a per-command output-line cap from the host's settings — a global
//! default, overridable per command by a glob pattern (e.g. `"tail *"` → 200 lines,
//! beating a global default of 50). Pure and host-agnostic: the actual `CommandEntry`
//! bookkeeping lives on `TerminalScreen` itself (see `screen.rs`), since that's what
//! has access to the live grid; this just answers "how many lines should this one be
//! allowed to record."

/// A classic `*`/`?` glob match (`*` = any run of characters, `?` = exactly one).
/// Fully anchored — the whole `text` must match the whole `pattern`, not a substring.
/// Case-sensitive, byte-based.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// The output-line cap for `command`: the first matching `(pattern, cap)` override
/// in order, else `default`.
pub fn resolve_output_cap(command: &str, overrides: &[(String, usize)], default: usize) -> usize {
    overrides
        .iter()
        .find(|(pattern, _)| glob_match(pattern, command))
        .map(|(_, cap)| *cap)
        .unwrap_or(default)
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
