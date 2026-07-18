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

/// Build sorted [`EnvVar`]s from name/value pairs — the OS-read path (a process's
/// launch-time environment, read from the kernel via `prexp-core`), as opposed to
/// [`parse`]'s newline-delimited `env` text from the shell hook.
pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Vec<EnvVar> {
    let mut vars: Vec<EnvVar> = pairs
        .into_iter()
        .map(|(name, value)| EnvVar { name, value })
        .collect();
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    vars
}

/// A valid env-var name: a leading letter/underscore then letters/digits/underscores.
/// Anything else is rejected so a name can't smuggle shell syntax into an injected
/// command.
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The bytes to type at a bash/zsh prompt to set `name` to `value` — single-quote
/// wrapped with embedded quotes escaped (`'\''`), so the value is inert data and can't
/// break out into shell code. `None` for an invalid `name`.
pub fn export_command(name: &str, value: &str) -> Option<Vec<u8>> {
    is_valid_name(name).then(|| {
        let escaped = value.replace('\'', "'\\''");
        format!("export {name}='{escaped}'\n").into_bytes()
    })
}

/// The bytes to type to unset `name`. `None` for an invalid `name`.
pub fn unset_command(name: &str) -> Option<Vec<u8>> {
    is_valid_name(name).then(|| format!("unset {name}\n").into_bytes())
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
