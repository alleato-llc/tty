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

/// Strip a leading shell prompt from a captured command line, best-effort.
///
/// A recorded command is the full echoed terminal row — prompt included
/// (`user@host dir % cargo build`), because the grid is the only place the
/// fully-resolved text exists. Displays keep the whole line (it's honest
/// context), but *copying* a command wants just the part the user typed.
/// Heuristic: cut after the **earliest** occurrence of a common prompt
/// terminator (`$ `, `% `, `# `, `❯ `, `> `). Earliest matters: in
/// `user@host % echo $ hi` the real prompt marker precedes any marker-like
/// text inside the command itself. A line with no marker (an exotic prompt),
/// or where stripping would leave nothing, comes back unchanged — failing
/// toward copying too much context rather than eating the command.
pub fn strip_prompt(line: &str) -> &str {
    const MARKERS: [&str; 5] = ["$ ", "% ", "# ", "❯ ", "> "];
    let earliest = MARKERS
        .iter()
        .filter_map(|marker| line.find(marker).map(|at| at + marker.len()))
        .min();
    match earliest {
        Some(start) => {
            let rest = line[start..].trim_start();
            if rest.is_empty() {
                line
            } else {
                rest
            }
        }
        None => line,
    }
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
