//! OSC 133 shell integration — what makes command-completion notifications possible.
//! Two paths: the [`ZSH_SNIPPET`] a user pastes into their own rc (manual, the
//! default), and a best-effort [`autoinstall_env`] that wires zsh up automatically by
//! pointing it at a generated `ZDOTDIR`.
//!
//! The hooks emit the semantic-prompt marks cathode parses (`OSC 133;C` before a
//! command runs, `OSC 133;D;<exit>` after) — see `cathode::screen`'s OSC handling — and
//! also capture the shell's environment each prompt for the **Env view**: `_tty_capture_env`
//! writes `env` to `$TTY_ENV_FILE` (set per session by [`env_channel_path`]), but only
//! while the `<file>.on` flag exists, so it costs one `stat` per command when the view
//! is closed and a small write only while it's open.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The zsh hooks, ready to paste into `~/.zshrc`. Kept in sync with what
/// [`autoinstall_env`] installs.
pub const ZSH_SNIPPET: &str = "\
# tty shell integration — command marks (OSC 133) + env capture for the Env view\n\
autoload -Uz add-zsh-hook\n\
_tty_preexec() { printf '\\e]133;C\\a' }\n\
_tty_precmd()  { printf '\\e]133;D;%s\\a' \"$?\"; _tty_capture_env }\n\
_tty_capture_env() {\n\
  [[ -n \"$TTY_ENV_FILE\" ]] || return\n\
  # One baseline dump at startup, then per-prompt only while the view is open (.on).\n\
  [[ -e \"${TTY_ENV_FILE}.on\" || ! -e \"$TTY_ENV_FILE\" ]] || return\n\
  env > \"${TTY_ENV_FILE}.tmp\" 2>/dev/null && mv -f \"${TTY_ENV_FILE}.tmp\" \"$TTY_ENV_FILE\"\n\
}\n\
add-zsh-hook preexec _tty_preexec\n\
add-zsh-hook precmd  _tty_precmd\n\
_tty_capture_env\n";

/// The env vars to set on a freshly spawned shell so tty's OSC 133 hooks load
/// automatically, or an empty vec when auto-install can't apply.
///
/// zsh only: we generate a `ZDOTDIR` whose startup files source the user's real
/// config and then add the hooks, and point the child there. Any other shell (or a
/// failure creating the dir) returns empty — the user falls back to the manual
/// snippet. `shell` is the child's shell path (from `$SHELL`).
pub fn autoinstall_env(shell: &str) -> Vec<(String, String)> {
    if !shell_is_zsh(shell) {
        return Vec::new();
    }
    let Some(dir) = zsh_zdotdir() else {
        return Vec::new();
    };
    let prev = std::env::var("ZDOTDIR").unwrap_or_default();
    vec![
        ("ZDOTDIR".to_string(), dir.to_string_lossy().into_owned()),
        ("TTY_PREV_ZDOTDIR".to_string(), prev),
    ]
}

/// Create a fresh per-session file path for the shell's env capture (the **Env view**),
/// in a user-only temp dir. `None` if the dir can't be created. tty exports this as
/// `TTY_ENV_FILE`; the hook writes the shell's `env` here each prompt while the
/// companion `<path>.on` flag exists (tty creates that flag only when the view is open).
pub fn env_channel_path() -> Option<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tty-env-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    // env holds secrets — keep the dir to the current user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Some(dir.join("env"))
}

fn shell_is_zsh(shell: &str) -> bool {
    std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name == "zsh")
}

/// Path to the generated zsh integration dir, created once per process (its contents
/// are static). `None` if the files can't be written.
fn zsh_zdotdir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(build_zsh_zdotdir).clone()
}

fn build_zsh_zdotdir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("tty-zsh-integration-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let quoted = dir.to_string_lossy().replace('"', "\\\"");
    // .zshenv runs first (and for every zsh): restore the user's real ZDOTDIR, source
    // their .zshenv, then force ZDOTDIR back to ours so our .zshrc also runs.
    let zshenv = format!(
        "# tty shell integration (generated)\n\
         if [[ -n \"$TTY_PREV_ZDOTDIR\" ]]; then export ZDOTDIR=\"$TTY_PREV_ZDOTDIR\"; else unset ZDOTDIR; fi\n\
         export _TTY_USER_ZDOTDIR=\"${{ZDOTDIR:-$HOME}}\"\n\
         [[ -f \"$_TTY_USER_ZDOTDIR/.zshenv\" ]] && source \"$_TTY_USER_ZDOTDIR/.zshenv\"\n\
         export ZDOTDIR=\"{quoted}\"\n",
    );
    // .zshrc restores ZDOTDIR for the rest of the session, sources the user's .zshrc,
    // then installs the hooks.
    let zshrc = format!(
        "export ZDOTDIR=\"$_TTY_USER_ZDOTDIR\"\n\
         [[ -f \"$ZDOTDIR/.zshrc\" ]] && source \"$ZDOTDIR/.zshrc\"\n\
         {ZSH_SNIPPET}",
    );
    std::fs::write(dir.join(".zshenv"), zshenv).ok()?;
    std::fs::write(dir.join(".zshrc"), zshrc).ok()?;
    Some(dir)
}

#[cfg(test)]
#[path = "shell_integration_tests.rs"]
mod tests;
