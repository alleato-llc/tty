//! The **Env view**'s data: the environment a shell captured to its per-session file
//! (written by the shell-integration hook — see [`crate::shell_integration`]). tty
//! reads + parses it while the view is open; nothing here talks to the shell directly.

use std::path::Path;

/// One environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Parse `env` output (newline-delimited `NAME=value`) into variables sorted by name.
/// A line with no `=` (the rare continuation of a value that itself contains a newline)
/// is skipped rather than guessed at.
pub fn parse(bytes: &[u8]) -> Vec<EnvVar> {
    let mut vars: Vec<EnvVar> = String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| EnvVar {
            name: name.to_string(),
            value: value.to_string(),
        })
        .collect();
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    vars
}

/// Read + parse the env a shell captured to `path`. Empty when the file isn't there yet
/// (the view was just opened, no prompt has fired) or can't be read.
pub fn read(path: &Path) -> Vec<EnvVar> {
    std::fs::read(path).map(|b| parse(&b)).unwrap_or_default()
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
